#[cfg(target_os = "linux")]
pub(crate) fn read_stat(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
}

// comm can itself contain spaces and ')', so the field list is only unambiguous after the last ')'.
#[cfg(any(test, target_os = "linux"))]
pub(crate) fn stat_fields(text: &str) -> Option<std::str::SplitWhitespace<'_>> {
    Some(text.get(text.rfind(')')? + 1..)?.split_whitespace())
}

#[cfg(any(test, target_os = "linux"))]
fn parse_proc_stat_starttime(stat: &str) -> Option<String> {
    stat_fields(stat)?.nth(19).map(|s| s.to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn process_start_marker(pid: u32) -> Option<String> {
    parse_proc_stat_starttime(&read_stat(pid)?)
}

#[cfg(target_os = "macos")]
pub(crate) fn process_start_marker(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(windows)]
pub(crate) fn process_start_marker(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(format!(
            "{}-{}",
            creation.dwHighDateTime, creation.dwLowDateTime
        ))
    }
}

#[cfg(unix)]
pub(crate) fn process_alive(pid: u32) -> bool {
    let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if ret == 0 {
        return true;
    }
    // ESRCH doesn't reliably map to io::ErrorKind::NotFound, so compare the raw errno instead.
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(windows)]
pub(crate) fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

#[cfg(unix)]
pub(crate) fn send_signal(pid: u32, signal: libc::c_int) -> std::io::Result<()> {
    let ret = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err);
        }
    }
    Ok(())
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

    #[test]
    fn parses_starttime_from_proc_stat_with_simple_comm() {
        let stat = "12345 (ipfs) S 1 12345 12345 0 -1 4194304 100 0 0 0 10 5 0 0 20 0 1 0 \
                     987654321 20971520 512";
        assert_eq!(
            parse_proc_stat_starttime(stat),
            Some("987654321".to_string())
        );
    }

    #[test]
    fn parses_starttime_from_proc_stat_with_spaces_and_parens_in_comm() {
        let stat = "12345 (my ip)fs proc) S 1 12345 12345 0 -1 4194304 100 0 0 0 10 5 0 0 20 0 1 0 \
                     555555 20971520 512";
        assert_eq!(parse_proc_stat_starttime(stat), Some("555555".to_string()));
    }

    #[test]
    fn parse_proc_stat_starttime_is_none_without_a_closing_paren() {
        assert_eq!(parse_proc_stat_starttime("garbage"), None);
    }
}
