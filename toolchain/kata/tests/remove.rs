//! Integration tests for `kata remove` against a temporary catalog repository.

#![cfg(feature = "publisher")]

#[cfg(test)]
mod common;

#[cfg(test)]
mod tests {
    use std::fs;

    use kata::progress::Noop;
    use kata::remove::{Input, run};
    use kata::repository;
    use kata::schema::documents::{CoreDocument, Document};
    use kata::schema::kinds::{CORE_API_VERSION, Kind};
    use kata::schema::view::Role;
    use tempfile::TempDir;

    use crate::common::{Recorder, input, sourced, write_fixtures};

    #[test]
    fn remove_kernel_deletes_by_source_and_keeps_the_rest() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = Input {
            source: Some("muak-os/rt".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };
        let recorder = Recorder::default();

        // ACT
        let removed = run(&input, &recorder).expect("remove kernel");

        // ASSERT
        assert_eq!(removed.identity, "kernels/muak-os/rt");
        assert_eq!(removed.digest, "sha256:kernel-rt");
        let events = recorder.labels();
        assert_eq!(events.len(), 1, "removal reports exactly one event");
        assert!(events[0].contains("kernels/muak-os/rt"));
        assert!(events[0].contains("sha256:kernel-rt"));
        assert!(events[0].contains(&removed.identity));
        assert!(events[0].contains(&removed.digest));
        let Document::Core(core) =
            repository::load(Kind::Core, root.path(), "v1.1.0").expect("reload")
        else {
            panic!("expected core document");
        };
        assert_eq!(core.kernels.len(), 1);
        assert_eq!(
            core.kernels.first().expect("kernel").source,
            "muak-os/linux"
        );
    }

    #[test]
    fn remove_last_kernel_is_refused() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        let core = CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: "v1.1.0".to_owned(),
            kernels: vec![sourced("muak-os/linux", "v6", "sha256:kernel")],
            stub: None,
            installer: None,
        };
        repository::write(&Document::Core(core), root.path(), "v1.1.0").expect("write core");
        let input = Input {
            source: Some("muak-os/linux".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&input, &Noop).expect_err("last kernel must be refused");
        assert!(error.to_string().contains("requires a kernel"));
    }

    #[test]
    fn remove_named_deletes_by_name_and_keeps_the_rest() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = Input {
            name: Some("rpi_generic".to_owned()),
            ..input(&root, Role::Overlay, "v1.1.0")
        };

        // ACT
        let removed = run(&input, &Noop).expect("remove overlay");

        // ASSERT
        assert_eq!(removed.identity, "overlays/rpi_generic");
        assert_eq!(removed.digest, "sha256:rpi_generic");
        let Document::Overlays(overlays) =
            repository::load(Kind::Overlays, root.path(), "v1.1.0").expect("reload")
        else {
            panic!("expected overlays document");
        };
        assert!(overlays.overlays.is_empty(), "document stays canonical");
    }

    #[test]
    fn remove_named_reports_unknown_entries() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = Input {
            name: Some("rpi_5".to_owned()),
            ..input(&root, Role::Overlay, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&input, &Noop).expect_err("unknown entry must fail");
        assert!(
            error
                .to_string()
                .contains("no overlays entry named 'rpi_5'")
        );
    }

    #[test]
    fn remove_kernel_reports_missing_documents_and_sources() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        let without_document = Input {
            source: Some("muak-os/linux".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&without_document, &Noop).expect_err("missing document must fail");
        assert!(error.to_string().contains("no core document"));

        // ARRANGE
        write_fixtures(&root);
        let unknown = Input {
            source: Some("muak-os/unknown".to_owned()),
            ..input(&root, Role::Kernel, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&unknown, &Noop).expect_err("unknown source must fail");
        assert!(
            error
                .to_string()
                .contains("no kernels entry for source 'muak-os/unknown'")
        );
    }

    #[test]
    fn remove_reports_missing_documents() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        fs::create_dir_all(root.path().join("core")).expect("create core dir");
        let input = Input {
            name: Some("rpi_generic".to_owned()),
            ..input(&root, Role::Overlay, "v1.1.0")
        };

        // ACT / ASSERT
        let error = run(&input, &Noop).expect_err("missing document must fail");
        assert!(error.to_string().contains("no overlays document"));
    }

    #[test]
    fn remove_slot_refused_by_the_resolvability_invariant() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);
        let input = input(&root, Role::Stub, "v1.1.0");

        // ACT / ASSERT
        let error = run(&input, &Noop).expect_err("stub removal must fail");
        assert!(error.to_string().contains("requires a stub"));
    }

    #[test]
    fn remove_requires_source_or_name() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_fixtures(&root);

        // ACT / ASSERT
        let error = run(&input(&root, Role::Kernel, "v1.1.0"), &Noop)
            .expect_err("kernel removal without --source must fail");
        assert!(error.to_string().contains("requires --source"));
        let error = run(&input(&root, Role::Overlay, "v1.1.0"), &Noop)
            .expect_err("overlay removal without --name must fail");
        assert!(error.to_string().contains("requires --name"));
    }
}
