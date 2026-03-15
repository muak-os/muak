//! PCR#11 value calculation for sealing policy.

use ring::digest;

use crate::types::SHA256_DIGEST_SIZE;

/// Computes the expected PCR#11 value by simulating extend operations.
pub fn predict_pcr11(section_data: &[(&str, &[u8])]) -> [u8; SHA256_DIGEST_SIZE] {
    let mut pcr = [0u8; SHA256_DIGEST_SIZE];

    for (name, data) in section_data {
        let mut name_bytes = name.as_bytes().to_vec();
        name_bytes.push(0u8);
        let name_measurement = digest::digest(&digest::SHA256, &name_bytes);
        pcr = extend(&pcr, name_measurement.as_ref());

        let data_measurement = digest::digest(&digest::SHA256, data);
        pcr = extend(&pcr, data_measurement.as_ref());
    }

    pcr
}

/// Computes the trial policy digest for PCR#11 with a given expected value.
pub fn compute_policy_digest(expected_pcr: &[u8; SHA256_DIGEST_SIZE]) -> [u8; SHA256_DIGEST_SIZE] {
    let mut policy = [0u8; SHA256_DIGEST_SIZE];

    let mut buf = Vec::with_capacity(SHA256_DIGEST_SIZE + 4 + 4 + 2 + 1 + 3);
    buf.extend_from_slice(&policy);
    buf.extend_from_slice(&crate::types::TPM2_CC_POLICY_PCR.to_be_bytes());

    let pcr_select = pcr_selection_bytes();
    buf.extend_from_slice(&pcr_select);

    let mut pcr_digest_ctx = digest::Context::new(&digest::SHA256);
    pcr_digest_ctx.update(expected_pcr);
    let pcr_digest = pcr_digest_ctx.finish();
    buf.extend_from_slice(pcr_digest.as_ref());

    let result = digest::digest(&digest::SHA256, &buf);
    policy.copy_from_slice(result.as_ref());

    policy
}

/// PCR extend: SHA256(old_value || measurement).
fn extend(current: &[u8; SHA256_DIGEST_SIZE], measurement: &[u8]) -> [u8; SHA256_DIGEST_SIZE] {
    let mut buf = Vec::with_capacity(SHA256_DIGEST_SIZE + measurement.len());
    buf.extend_from_slice(current);
    buf.extend_from_slice(measurement);
    let result = digest::digest(&digest::SHA256, &buf);
    let mut out = [0u8; SHA256_DIGEST_SIZE];
    out.copy_from_slice(result.as_ref());
    out
}

/// Serializes PCR selection for SHA-256 PCR#11.
fn pcr_selection_bytes() -> Vec<u8> {
    let mut sel = Vec::with_capacity(8);
    sel.extend_from_slice(&1u32.to_be_bytes());
    sel.extend_from_slice(&crate::types::TPM2_ALG_SHA256.to_be_bytes());
    sel.push(3);
    let byte_index = (crate::types::PCR_INDEX / 8) as usize;
    let bit_index = crate::types::PCR_INDEX % 8;
    let mut bitmap = [0u8; 3];
    bitmap[byte_index] = 1 << bit_index;
    sel.extend_from_slice(&bitmap);
    sel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predict_pcr11_empty() {
        // ACT
        let result = predict_pcr11(&[]);

        // ASSERT
        assert_eq!(result, [0u8; SHA256_DIGEST_SIZE]);
    }

    #[test]
    fn predict_pcr11_deterministic() {
        // ARRANGE
        let sections = [
            (".cmdline", b"console=ttyS0" as &[u8]),
            (".linux", &[0xDE, 0xAD]),
            (".initrd", &[0xBE, 0xEF]),
        ];

        // ACT
        let a = predict_pcr11(&sections);
        let b = predict_pcr11(&sections);

        // ASSERT
        assert_eq!(a, b);
    }

    #[test]
    fn predict_pcr11_order_matters() {
        // ARRANGE
        let s1 = [(".cmdline", b"a" as &[u8]), (".linux", b"b" as &[u8])];
        let s2 = [(".linux", b"b" as &[u8]), (".cmdline", b"a" as &[u8])];

        // ACT
        let a = predict_pcr11(&s1);
        let b = predict_pcr11(&s2);

        // ASSERT
        assert_ne!(a, b);
    }

    #[test]
    fn policy_digest_deterministic() {
        // ARRANGE
        let pcr = [0x42u8; SHA256_DIGEST_SIZE];

        // ACT
        let a = compute_policy_digest(&pcr);
        let b = compute_policy_digest(&pcr);

        // ASSERT
        assert_eq!(a, b);
    }

    #[test]
    fn policy_digest_different_pcrs() {
        // ARRANGE
        let pcr_a = [0x01u8; SHA256_DIGEST_SIZE];
        let pcr_b = [0x02u8; SHA256_DIGEST_SIZE];

        // ACT
        let a = compute_policy_digest(&pcr_a);
        let b = compute_policy_digest(&pcr_b);

        // ASSERT
        assert_ne!(a, b);
    }
}
