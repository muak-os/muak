//! PCR#11 value calculation for sealing policy.

use ring::digest;

pub(crate) type Digest = [u8; 32];

const TPM2_CC_POLICY_PCR: u32 = 0x0000_017F;
const PCR11_SELECTION: [u8; 10] = [0, 0, 0, 1, 0, 0x0B, 3, 0, 8, 0];
const SHA256_DIGEST_SIZE: usize = 32;

/// Computes the expected PCR#11 value by simulating extend operations.
#[must_use]
pub fn predict_pcr11(section_data: &[(&str, &[u8])]) -> Digest {
    let mut pcr = [0_u8; SHA256_DIGEST_SIZE];

    for &(name, data) in section_data {
        pcr = extend(&pcr, hash_name(name).as_ref());
        pcr = extend(&pcr, digest::digest(&digest::SHA256, data).as_ref());
    }

    pcr
}

/// Computes the trial policy digest for PCR#11 with a given expected value.
#[must_use]
pub(crate) fn compute_policy_digest(expected_pcr: &Digest) -> Digest {
    let mut policy = [0_u8; SHA256_DIGEST_SIZE];
    let mut policy_ctx = digest::Context::new(&digest::SHA256);
    policy_ctx.update(&policy);
    policy_ctx.update(&TPM2_CC_POLICY_PCR.to_be_bytes());
    policy_ctx.update(&PCR11_SELECTION);
    policy_ctx.update(digest::digest(&digest::SHA256, expected_pcr).as_ref());
    policy.copy_from_slice(policy_ctx.finish().as_ref());

    policy
}

/// Hashes a section name with a null terminator using SHA256.
fn hash_name(name: &str) -> digest::Digest {
    let mut context = digest::Context::new(&digest::SHA256);
    context.update(name.as_bytes());
    context.update(&[0_u8]);

    context.finish()
}

/// PCR extend: `SHA256(old_value || measurement)`.
fn extend(current: &Digest, measurement: &[u8]) -> Digest {
    let mut context = digest::Context::new(&digest::SHA256);
    context.update(current);
    context.update(measurement);
    let result = context.finish();
    let mut out = [0_u8; SHA256_DIGEST_SIZE];
    out.copy_from_slice(result.as_ref());

    out
}

pub(crate) fn pcr11_selection_bytes() -> &'static [u8; 10] {
    &PCR11_SELECTION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predict_pcr11_empty() {
        // ACT
        let result = predict_pcr11(&[]);

        // ASSERT
        assert_eq!(
            result, [0_u8; SHA256_DIGEST_SIZE],
            "empty PCR prediction should be zero"
        );
    }

    #[test]
    fn predict_pcr11_deterministic() {
        // ARRANGE
        let sections: [(&str, &[u8]); 3] = [
            (".cmdline", b"console=ttyS0"),
            (".linux", &[0xDE, 0xAD]),
            (".initrd", &[0xBE, 0xEF]),
        ];

        // ACT
        let first_prediction = predict_pcr11(&sections);
        let second_prediction = predict_pcr11(&sections);

        // ASSERT
        assert_eq!(
            first_prediction, second_prediction,
            "PCR prediction should be deterministic"
        );
    }

    #[test]
    fn predict_pcr11_order_matters() {
        // ARRANGE
        let ordered_sections: [(&str, &[u8]); 2] = [(".cmdline", b"a"), (".linux", b"b")];
        let reordered_sections: [(&str, &[u8]); 2] = [(".linux", b"b"), (".cmdline", b"a")];

        // ACT
        let ordered_prediction = predict_pcr11(&ordered_sections);
        let reordered_prediction = predict_pcr11(&reordered_sections);

        // ASSERT
        assert_ne!(
            ordered_prediction, reordered_prediction,
            "PCR prediction should be order-sensitive"
        );
    }

    #[test]
    fn policy_digest_deterministic() {
        // ARRANGE
        let pcr = [0x42_u8; SHA256_DIGEST_SIZE];

        // ACT
        let first_digest = compute_policy_digest(&pcr);
        let second_digest = compute_policy_digest(&pcr);

        // ASSERT
        assert_eq!(
            first_digest, second_digest,
            "policy digest should be deterministic"
        );
    }

    #[test]
    fn policy_digest_different_pcrs() {
        // ARRANGE
        let pcr_a = [0x01_u8; SHA256_DIGEST_SIZE];
        let pcr_b = [0x02_u8; SHA256_DIGEST_SIZE];

        // ACT
        let first_digest = compute_policy_digest(&pcr_a);
        let second_digest = compute_policy_digest(&pcr_b);

        // ASSERT
        assert_ne!(
            first_digest, second_digest,
            "policy digest should depend on PCR value"
        );
    }
}
