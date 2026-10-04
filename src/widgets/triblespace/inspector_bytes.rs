//! Decimal byte labels for immutable inspectors; exact values stay on hover.
pub(super) fn compact_bytes(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut amount = bytes as f64;
    let mut unit = "B";
    for next in ["kB", "MB", "GB", "TB", "PB", "EB"] {
        amount /= 1000.0;
        unit = next;
        if amount < 1000.0 {
            break;
        }
    }
    format!("{amount:.1} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_units_match_the_pile_instrument() {
        assert_eq!(compact_bytes(0), "0 B");
        assert_eq!(compact_bytes(999), "999 B");
        assert_eq!(compact_bytes(1000), "1.0 kB");
        assert_eq!(compact_bytes(233_305_898_752), "233.3 GB");
        assert_eq!(compact_bytes(u64::MAX), "18.4 EB");
    }
}
