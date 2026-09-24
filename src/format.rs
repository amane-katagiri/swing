const BYTE_UNITS: [(u64, &str); 4] = [
    (1u64 << 40, "TB"),
    (1u64 << 30, "GB"),
    (1u64 << 20, "MB"),
    (1u64 << 10, "KB"),
];

pub fn format_bytes(n: u64) -> String {
    for &(factor, unit) in &BYTE_UNITS {
        if n < factor {
            continue;
        }
        if n.is_multiple_of(factor) {
            return format!("{} {unit}", n / factor);
        }
        if (u128::from(n) * 10).is_multiple_of(u128::from(factor)) {
            return format!("{:.1} {unit}", n as f64 / factor as f64);
        }
    }
    format!("{n} B")
}

const DURATION_UNITS: [(u64, &str); 3] = [(86_400, "d"), (3_600, "h"), (60, "m")];

pub fn format_duration_secs(secs: u64) -> String {
    for &(factor, unit) in &DURATION_UNITS {
        if secs != 0 && secs.is_multiple_of(factor) {
            return format!("{}{unit}", secs / factor);
        }
    }
    format!("{secs}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_prefers_the_largest_exact_unit() {
        assert_eq!(format_bytes(107_374_182_400), "100 GB");
        assert_eq!(format_bytes(512 * (1u64 << 20)), "512 MB");
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(300), "300 B");
    }

    #[test]
    fn format_duration_secs_prefers_the_largest_exact_unit() {
        assert_eq!(format_duration_secs(300), "5m");
        assert_eq!(format_duration_secs(86_400), "1d");
        assert_eq!(format_duration_secs(7_200), "2h");
        assert_eq!(format_duration_secs(600), "10m");
        assert_eq!(format_duration_secs(30), "30s");
        assert_eq!(format_duration_secs(0), "0s");
    }
}
