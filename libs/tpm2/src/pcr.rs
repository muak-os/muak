//! PCR#11 value calculation for sealing policy.

use sha2::{Digest as _, Sha256};

pub(crate) type Digest = [u8; 32];

const TPM2_CC_POLICY_PCR: u32 = 0x0000_017F;
const PCR11_SELECTION: [u8; 10] = [0, 0, 0, 1, 0, 0x0B, 3, 0, 8, 0];
const SHA256_DIGEST_SIZE: usize = 32;

/// Computes the expected PCR#11 value by simulating extend operations
/// using pre-computed SHA-256 hashes of each section's data.
#[must_use]
pub fn predict_pcr11(sections: &[(&str, &[u8; 32])]) -> Digest {
    let mut pcr = [0_u8; SHA256_DIGEST_SIZE];

    for &(name, data_hash) in sections {
        pcr = extend(&pcr, hash_name(name).as_ref());
        pcr = extend(&pcr, data_hash);
    }

    pcr
}

/// Computes the trial policy digest for PCR#11 with a given expected value.
#[must_use]
pub(crate) fn compute_policy_digest(expected_pcr: &Digest) -> Digest {
    let mut policy = [0_u8; SHA256_DIGEST_SIZE];
    let mut policy_ctx = Sha256::new();
    policy_ctx.update(policy);
    policy_ctx.update(TPM2_CC_POLICY_PCR.to_be_bytes());
    policy_ctx.update(PCR11_SELECTION);
    policy_ctx.update(Sha256::digest(expected_pcr));
    policy.copy_from_slice(&policy_ctx.finalize());

    policy
}

/// Hashes a section name with a null terminator using SHA256.
fn hash_name(name: &str) -> [u8; SHA256_DIGEST_SIZE] {
    let mut context = Sha256::new();
    context.update(name.as_bytes());
    context.update([0_u8]);

    let mut out = [0_u8; SHA256_DIGEST_SIZE];
    out.copy_from_slice(&context.finalize());

    out
}

/// PCR extend: `SHA256(old_value || measurement)`.
fn extend(current: &Digest, measurement: &[u8]) -> Digest {
    let mut context = Sha256::new();
    context.update(current);
    context.update(measurement);
    let mut out = [0_u8; SHA256_DIGEST_SIZE];
    out.copy_from_slice(&context.finalize());

    out
}

pub(crate) fn pcr11_selection_bytes() -> &'static [u8; 10] {
    &PCR11_SELECTION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha256_arr(data: &[u8]) -> [u8; 32] {
        let digest = Sha256::digest(data);
        let mut hash = [0; 32];
        hash.copy_from_slice(&digest);

        hash
    }

    #[test]
    fn predict_pcr11_empty() {
        // ACT
        let result = predict_pcr11(&[]);

        // ASSERT
        assert_eq!(result, [0_u8; SHA256_DIGEST_SIZE]);
    }

    #[test]
    fn predict_pcr11_deterministic() {
        // ARRANGE
        let mut h_cmdline = [0; 32];
        let mut h_kernel = [0; 32];
        let mut h_initrd = [0; 32];
        h_cmdline.copy_from_slice(&Sha256::digest(b"console=ttyS0"));
        h_kernel.copy_from_slice(&Sha256::digest([0xDE, 0xAD]));
        h_initrd.copy_from_slice(&Sha256::digest([0xBE, 0xEF]));

        let sections: [(&str, &[u8; 32]); 3] = [
            (".cmdline", &h_cmdline),
            (".kernel", &h_kernel),
            (".initrd", &h_initrd),
        ];

        // ACT
        let first = predict_pcr11(&sections);
        let second = predict_pcr11(&sections);

        // ASSERT
        assert_eq!(first, second);
    }

    #[test]
    fn predict_pcr11_order_matters() {
        // ARRANGE
        let h_a = sha256_arr(b"a");
        let h_b = sha256_arr(b"b");

        let ordered: [(&str, &[u8; 32]); 2] = [(".cmdline", &h_a), (".kernel", &h_b)];
        let reordered: [(&str, &[u8; 32]); 2] = [(".kernel", &h_b), (".cmdline", &h_a)];

        // ACT / ASSERT
        assert_ne!(predict_pcr11(&ordered), predict_pcr11(&reordered));
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
