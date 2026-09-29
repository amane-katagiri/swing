#[cfg(target_os = "linux")]
pub(crate) fn read_stat(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
}

// comm can itself contain spaces and ')', so the field list is only unambiguous after the last ')'.
pub(crate) fn stat_fields(text: &str) -> Option<std::str::SplitWhitespace<'_>> {
    Some(text.get(text.rfind(')')? + 1..)?.split_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_start_after_the_last_paren_of_comm() {
        let fields: Vec<&str> = stat_fields("42 (a) b) (c) S 1 2").unwrap().collect();
        assert_eq!(fields, ["S", "1", "2"]);
        assert!(stat_fields("42 no paren").is_none());
    }
}
