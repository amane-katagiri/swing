use anyhow::{Context, Result, bail};

#[cfg(not(windows))]
pub(super) fn decode_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

// schtasks writes in the OEM code page (CP932 on Japanese Windows), not UTF-8.
#[cfg(windows)]
pub(super) fn decode_output(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{GetOEMCP, MultiByteToWideChar};

    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    let Ok(len) = i32::try_from(bytes.len()) else {
        return String::from_utf8_lossy(bytes).into_owned();
    };
    unsafe {
        let cp = GetOEMCP();
        let n = MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; n as usize];
        MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n);
        String::from_utf16_lossy(&wide)
    }
}

#[cfg(any(windows, test))]
pub(super) fn output_with_timeout(
    cmd: &mut std::process::Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;

    fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    }

    let display = format!("{cmd:?}");
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running command: {display}"))?;
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("waiting for command: {display}"))?
        {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "command did not finish within {} s: {display}",
                timeout.as_secs_f32()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let collect = |h: Option<std::thread::JoinHandle<Vec<u8>>>| {
        h.and_then(|h| h.join().ok()).unwrap_or_default()
    };
    Ok(std::process::Output {
        status,
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

#[cfg(any(windows, test))]
pub(super) fn task_listed(csv: &str, name: &str) -> bool {
    let wanted = format!("\"\\{name}\"");
    csv.lines().any(|line| {
        line.split(',')
            .next()
            .is_some_and(|first| first.trim().eq_ignore_ascii_case(&wanted))
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn run_command(mut cmd: std::process::Command) -> Result<std::process::Output> {
    let display = format!("{cmd:?}");
    let output = cmd
        .output()
        .with_context(|| format!("running command: {display}"))?;
    require_success(&display, output)
}

#[cfg(windows)]
pub(super) fn run_command_with_timeout(
    mut cmd: std::process::Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output> {
    let display = format!("{cmd:?}");
    require_success(&display, output_with_timeout(&mut cmd, timeout)?)
}

fn require_success(display: &str, output: std::process::Output) -> Result<std::process::Output> {
    if !output.status.success() {
        bail!(
            "command failed: {display}; stderr: {}",
            decode_output(&output.stderr).trim()
        );
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_listing_matches_only_the_root_task_by_exact_name() {
        let csv = "\"\\swing-old\",\"N/A\",\"Ready\"\r\n\"\\Folder\\swing\",\"N/A\",\"Ready\"\r\n";
        assert!(!task_listed(csv, "swing"));
        assert!(task_listed(
            &format!("{csv}\"\\SWING\",\"N/A\",\"Running\"\r\n"),
            "swing"
        ));
        assert!(!task_listed("", "swing"));
    }

    #[cfg(unix)]
    #[test]
    fn output_with_timeout_collects_output_and_gives_up_on_a_hung_command() {
        let out = output_with_timeout(
            std::process::Command::new("sh").args(["-c", "printf out; printf err >&2; exit 3"]),
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(out.stdout, b"out");
        assert_eq!(out.stderr, b"err");
        assert_eq!(out.status.code(), Some(3));

        let started = std::time::Instant::now();
        let err = output_with_timeout(
            std::process::Command::new("sleep").arg("30"),
            std::time::Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(err.to_string().contains("did not finish"), "{err:#}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
