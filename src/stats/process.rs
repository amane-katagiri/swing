use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessUsage {
    pub cpu: Duration,
    pub rss: u64,
}

#[cfg(target_os = "linux")]
pub fn usage(pid: u32) -> Option<ProcessUsage> {
    let text = crate::proc::read_stat(pid)?;
    let ticks = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).ok()?;
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    parse_proc_stat(&text, ticks, page)
}

#[cfg(any(target_os = "linux", test))]
fn parse_proc_stat(text: &str, ticks_per_sec: u64, page_size: u64) -> Option<ProcessUsage> {
    let fields: Vec<&str> = crate::proc::stat_fields(text)?.collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let rss_pages: u64 = fields.get(21)?.parse().ok()?;
    if ticks_per_sec == 0 {
        return None;
    }
    let ticks = utime + stime;
    Some(ProcessUsage {
        cpu: Duration::from_secs(ticks / ticks_per_sec)
            + Duration::from_nanos((ticks % ticks_per_sec) * 1_000_000_000 / ticks_per_sec),
        rss: rss_pages * page_size,
    })
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn usage(pid: u32) -> Option<ProcessUsage> {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_taskinfo>()).ok()?;
    let written = unsafe {
        libc::proc_pidinfo(
            libc::c_int::try_from(pid).ok()?,
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    let mut timebase = libc::mach_timebase_info { numer: 0, denom: 0 };
    if unsafe { libc::mach_timebase_info(&raw mut timebase) } != 0 || timebase.denom == 0 {
        return None;
    }
    let ticks = u128::from(info.pti_total_user) + u128::from(info.pti_total_system);
    let nanos = ticks * u128::from(timebase.numer) / u128::from(timebase.denom);
    Some(ProcessUsage {
        cpu: Duration::from_nanos(u64::try_from(nanos).ok()?),
        rss: info.pti_resident_size,
    })
}

#[cfg(windows)]
pub fn usage(pid: u32) -> Option<ProcessUsage> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn hundred_nanos(t: FILETIME) -> u64 {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    }

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    let mut memory: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(0);
    memory.cb = size;
    let ok = unsafe {
        GetProcessTimes(
            handle,
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        ) != 0
            && GetProcessMemoryInfo(handle, &raw mut memory, size) != 0
    };
    unsafe { CloseHandle(handle) };
    if !ok {
        return None;
    }
    Some(ProcessUsage {
        cpu: Duration::from_nanos((hundred_nanos(kernel) + hundred_nanos(user)) * 100),
        rss: u64::try_from(memory.WorkingSetSize).ok()?,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn usage(_pid: u32) -> Option<ProcessUsage> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat_even_when_the_command_has_spaces_and_parens() {
        let text = "1234 (ipfs (x) y) S 1 1234 1234 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 12 0 5000 123456789 300 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0";
        assert_eq!(
            parse_proc_stat(text, 100, 4096),
            Some(ProcessUsage {
                cpu: Duration::from_secs(3),
                rss: 300 * 4096,
            })
        );
    }

    #[test]
    fn rejects_truncated_proc_stat() {
        assert_eq!(parse_proc_stat("1234 (ipfs) S 1 2 3", 100, 4096), None);
    }

    #[test]
    fn reads_its_own_process() {
        let usage = usage(std::process::id());
        if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
            assert!(usage.is_some_and(|u| u.rss > 0));
        }
    }
}
