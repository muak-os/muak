//! TPM2 command template and payload writers.

use crate::buffer::{CommandBuffer, u16_len};
use crate::error::{Result, Tpm2Error};

const TPM2_ALG_SHA256: u16 = 0x000B;
const TPM2_ALG_KEYEDHASH: u16 = 0x0008;
const TPM2_ALG_NULL: u16 = 0x0010;
const SRK_TEMPLATE: [u8; 26] = [
    0x00, 0x23, 0x00, 0x0B, 0x00, 0x03, 0x04, 0x72, 0x00, 0x00, 0x00, 0x06, 0x00, 0x80, 0x00, 0x43,
    0x00, 0x10, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
];

/// Writes a `TPM2B_SENSITIVE_CREATE` structure with the given user auth and data.
pub(crate) fn write_sensitive_create(
    command: &mut CommandBuffer,
    user_auth: &[u8],
    data: &[u8],
) -> Result<()> {
    let inner_size = checked_sum(&[2, user_auth.len(), 2, data.len()])?;
    command.write_u16(u16_len(inner_size)?);
    command.write_u16(u16_len(user_auth.len())?);
    command.write_bytes(user_auth);
    command.write_u16(u16_len(data.len())?);
    command.write_bytes(data);

    Ok(())
}

/// Writes a `TPM2B_PUBLIC` template for the Storage Root Key (SRK).
pub(crate) fn write_srk_template(command: &mut CommandBuffer) {
    command.write_u16(u16::try_from(SRK_TEMPLATE.len()).ok().unwrap_or(0));
    command.write_bytes(&SRK_TEMPLATE);
}

/// Writes a `TPM2B_PUBLIC` template for a sealed object with the given policy digest.
pub(crate) fn write_seal_template(command: &mut CommandBuffer, policy_digest: &[u8]) -> Result<()> {
    let template_len =
        16_usize
            .checked_add(policy_digest.len())
            .ok_or(Tpm2Error::BufferTooLarge {
                actual: policy_digest.len(),
                max: usize::MAX.saturating_sub(16),
            })?;
    command.write_u16(u16_len(template_len)?);
    command.write_u16(TPM2_ALG_KEYEDHASH);
    command.write_u16(TPM2_ALG_SHA256);
    command.write_u32(0x0000_0012);
    command.write_u16(u16_len(policy_digest.len())?);
    command.write_bytes(policy_digest);
    command.write_u16(TPM2_ALG_NULL);
    command.write_u16(0);

    Ok(())
}

fn checked_sum(values: &[usize]) -> Result<usize> {
    values.iter().try_fold(0_usize, |total, value| {
        total.checked_add(*value).ok_or(Tpm2Error::BufferTooLarge {
            actual: total,
            max: usize::MAX.saturating_sub(*value),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_builders_match_expected_wire_format() {
        // ARRANGE
        let policy = [0xAA; 32];
        let mut sensitive_command = CommandBuffer::new(0, 0);
        let mut srk_command = CommandBuffer::new(0, 0);
        let mut seal_command = CommandBuffer::new(0, 0);

        // ACT
        let sensitive_result = write_sensitive_create(&mut sensitive_command, &[1, 2], &[3, 4, 5]);
        write_srk_template(&mut srk_command);
        let seal_result = write_seal_template(&mut seal_command, &policy);
        let sensitive = sensitive_command.finalize();
        let srk = srk_command.finalize();
        let seal = seal_command.finalize();

        // ASSERT
        assert!(sensitive_result.is_ok(), "sensitive writer should succeed");
        assert!(seal_result.is_ok(), "seal template writer should succeed");
        assert!(sensitive.is_ok(), "sensitive area should build");
        assert!(seal.is_ok(), "seal template should build");
        let sensitive = sensitive.ok().unwrap_or_default();
        let srk = srk.ok().unwrap_or_default();
        let seal = seal.ok().unwrap_or_default();
        assert_eq!(
            sensitive.get(10..12),
            Some(9_u16.to_be_bytes().as_slice()),
            "inner size should match"
        );
        assert_eq!(
            srk.get(12..),
            Some(SRK_TEMPLATE.as_slice()),
            "SRK template should match"
        );
        assert_eq!(
            seal.len(),
            58,
            "seal template command should include policy digest"
        );
    }

    #[test]
    fn sensitive_create_rejects_oversized_data() {
        // ARRANGE
        let oversized = vec![0_u8; usize::from(u16::MAX)];
        let mut command = CommandBuffer::new(0, 0);

        // ACT
        let result = write_sensitive_create(&mut command, &[1], &oversized);

        // ASSERT
        assert!(result.is_err(), "oversized sensitive area should fail");
    }

    #[test]
    fn seal_template_rejects_oversized_policy() {
        // ARRANGE
        let oversized = vec![0_u8; usize::from(u16::MAX) + 1];
        let mut command = CommandBuffer::new(0, 0);

        // ACT
        let result = write_seal_template(&mut command, &oversized);

        // ASSERT
        assert!(result.is_err(), "oversized policy digest should fail");
    }
}
