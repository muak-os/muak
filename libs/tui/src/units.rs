//! Human-readable unit formatting.

const KIB: u64 = 1024;
const MIB: u64 = KIB.saturating_mul(1024);
const GIB: u64 = MIB.saturating_mul(1024);
const TIB: u64 = GIB.saturating_mul(1024);

/// Formats `value` bytes with binary IEC units.
#[must_use]
pub fn bytes(value: u64) -> String {
    if value < KIB {
        return format!("{value} B");
    }

    let (divisor, label) = match value {
        value if value >= TIB => (TIB, "TiB"),
        value if value >= GIB => (GIB, "GiB"),
        value if value >= MIB => (MIB, "MiB"),
        _ => (KIB, "KiB"),
    };

    let whole = value.checked_div(divisor).unwrap_or(0);
    let remainder = value.checked_rem(divisor).unwrap_or(0);
    let tenths = remainder
        .saturating_mul(10)
        .checked_div(divisor)
        .unwrap_or(0);

    if tenths == 0 {
        format!("{whole} {label}")
    } else {
        format!("{whole}.{tenths} {label}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_below_one_kib_use_bytes() {
        // ARRANGE
        let value = 1023;

        // ACT
        let rendered = bytes(value);

        // ASSERT
        assert_eq!(rendered, "1023 B");
    }

    #[test]
    fn exact_kib_omits_the_fraction() {
        // ARRANGE
        let value = 1024;

        // ACT
        let rendered = bytes(value);

        // ASSERT
        assert_eq!(rendered, "1 KiB");
    }

    #[test]
    fn fractional_kib_keeps_one_decimal() {
        // ARRANGE
        let value = 1536;

        // ACT
        let rendered = bytes(value);

        // ASSERT
        assert_eq!(rendered, "1.5 KiB");
    }

    #[test]
    fn mebibytes_use_the_mib_label() {
        // ARRANGE
        let value = 5 * MIB;

        // ACT
        let rendered = bytes(value);

        // ASSERT
        assert_eq!(rendered, "5 MiB");
    }

    #[test]
    fn gibibytes_use_the_gib_label() {
        // ARRANGE
        let value = GIB + (GIB.checked_div(2).unwrap_or(0));

        // ACT
        let rendered = bytes(value);

        // ASSERT
        assert_eq!(rendered, "1.5 GiB");
    }

    #[test]
    fn tebibytes_use_the_tib_label() {
        // ARRANGE
        let value = 2 * TIB;

        // ACT
        let rendered = bytes(value);

        // ASSERT
        assert_eq!(rendered, "2 TiB");
    }

    #[test]
    fn zero_uses_bytes() {
        // ARRANGE
        let value = 0;

        // ACT
        let rendered = bytes(value);

        // ASSERT
        assert_eq!(rendered, "0 B");
    }
}
