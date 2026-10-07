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

#[cfg(target_os = "linux")]
pub(crate) fn boot_id() -> Option<String> {
    let text = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(target_os = "macos")]
pub(crate) fn process_start_marker(pid: u32) -> Option<String> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
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
pub(crate) fn open_process(
    pid: u32,
    access: windows_sys::Win32::System::Threading::PROCESS_ACCESS_RIGHTS,
) -> std::io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::FromRawHandle;
    let handle = unsafe { windows_sys::Win32::System::Threading::OpenProcess(access, 0, pid) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) })
}

#[cfg(windows)]
pub(crate) fn open_process_for_query(pid: u32) -> Option<std::os::windows::io::OwnedHandle> {
    open_process(
        pid,
        windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION,
    )
    .ok()
}

#[cfg(windows)]
pub(crate) fn process_start_marker(pid: u32) -> Option<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;
    let handle = open_process_for_query(pid)?;
    unsafe {
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(
            handle.as_raw_handle(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        );
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
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::STILL_ACTIVE;
    use windows_sys::Win32::System::Threading::GetExitCodeProcess;
    let Some(handle) = open_process_for_query(pid) else {
        return false;
    };
    let mut code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(handle.as_raw_handle(), &mut code) };
    ok != 0 && code == STILL_ACTIVE as u32
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
