//! Integration tests for the error type.

#[cfg(test)]
mod tests {
    use core::mem::{needs_drop, size_of};

    use sandbox::error::{Errno, Mechanism, SandboxError};

    #[test]
    fn error_type_is_allocation_free() {
        // ARRANGE
        let error = SandboxError::Denied {
            mechanism: Mechanism::Mounts,
            source: Errno::new(1),
        };

        // ACT
        let drops = needs_drop::<SandboxError>();
        let size = size_of::<SandboxError>();

        // ASSERT
        assert!(
            !drops,
            "the error type must stay usable between fork and exec"
        );
        assert_eq!(size_of_val(&error), size);
        assert!(size <= 32, "the error type grew to {size} bytes");
    }

    #[test]
    fn io_error_reports_the_kernel_number() {
        // ARRANGE
        let error = SandboxError::Io(Errno::new(13));

        // ACT
        let displayed = error.to_string();

        // ASSERT
        assert!(displayed.contains("13"), "display was {displayed}");
    }

    #[test]
    fn variants_report_their_mechanism() {
        // ARRANGE
        let cases = [
            (
                SandboxError::NotCompiled {
                    mechanism: Mechanism::Seccomp,
                },
                Mechanism::Seccomp,
            ),
            (
                SandboxError::Unsupported {
                    mechanism: Mechanism::Cgroup,
                    detail: "cgroup v2 is missing",
                },
                Mechanism::Cgroup,
            ),
            (
                SandboxError::Denied {
                    mechanism: Mechanism::Rootfs,
                    source: Errno::new(2),
                },
                Mechanism::Rootfs,
            ),
        ];

        // ACT
        let displayed = cases.map(|(error, mechanism)| (error.to_string(), mechanism.name()));

        // ASSERT
        for (message, name) in displayed {
            assert!(
                message.contains(name),
                "message {message} does not mention {name}"
            );
        }
    }
}
