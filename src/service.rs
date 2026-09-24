use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::resolve_config_path;

fn resolve_service_paths(config_path: Option<&Path>) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let config = resolve_config_path(config_path);
    if !config.exists() {
        bail!(
            "service install needs a config file (swing.toml): pass --config or set SWING_CONFIG"
        );
    }
    let config = config
        .canonicalize()
        .with_context(|| format!("resolving config path {}", config.display()))?;
    let workdir = config
        .parent()
        .map(Path::to_path_buf)
        .context("config file has no parent directory")?;
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("resolving current executable path")?;
    Ok((config, workdir, exe))
}

fn require_system_supported(system: bool) -> Result<()> {
    if system && !cfg!(target_os = "linux") {
        bail!("--system is only supported on Linux");
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn write_service_file(path: &Path, content: impl AsRef<[u8]>) -> Result<()> {
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}

fn escape_systemd_specifiers(value: &str) -> String {
    value.replace('%', "%%")
}

fn quote_systemd_arg(arg: &str) -> String {
    let escaped = escape_systemd_specifiers(arg)
        .replace('$', "$$")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub fn systemd_unit(exe: &Path, config: &Path, workdir: &Path, system: bool) -> String {
    let exec_start = format!(
        "{} up --config {}",
        quote_systemd_arg(&exe.to_string_lossy()),
        quote_systemd_arg(&config.to_string_lossy()),
    );
    let wanted_by = if system {
        "multi-user.target"
    } else {
        "default.target"
    };
    format!(
        "[Unit]\n\
Description=SWING mirror agent\n\
After=network-online.target\n\
Wants=network-online.target\n\
\n\
[Service]\n\
ExecStart={exec_start}\n\
WorkingDirectory={workdir}\n\
Restart=on-failure\n\
RestartSec=5\n\
KillSignal=SIGTERM\n\
TimeoutStopSec=60\n\
\n\
[Install]\n\
WantedBy={wanted_by}\n",
        workdir = escape_systemd_specifiers(&workdir.to_string_lossy()),
    )
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn launchd_plist(exe: &Path, config: &Path, workdir: &Path, log: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    let config = xml_escape(&config.to_string_lossy());
    let workdir = xml_escape(&workdir.to_string_lossy());
    let log = xml_escape(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>jp.ne.ama.swing</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>up</string>
        <string>--config</string>
        <string>{config}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{workdir}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    </dict>
</dict>
</plist>
"#
    )
}

pub fn launchd_tray_plist(tray: &Path, config: &Path, workdir: &Path) -> String {
    let tray = xml_escape(&tray.to_string_lossy());
    let config = xml_escape(&config.to_string_lossy());
    let workdir = xml_escape(&workdir.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>jp.ne.ama.swing-tray</string>
    <key>ProgramArguments</key>
    <array>
        <string>{tray}</string>
        <string>--config</string>
        <string>{config}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{workdir}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    )
}

// Explorer starts Run entries through CreateProcess, which is not documented to accept \\?\ paths.
fn strip_verbatim(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
}

pub fn tray_run_command(tray: &Path, config: &Path) -> String {
    format!(
        "\"{}\" --config \"{}\"",
        strip_verbatim(&tray.to_string_lossy()),
        strip_verbatim(&config.to_string_lossy())
    )
}

pub fn tray_exe_path(exe: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "swing-tray.exe"
    } else {
        "swing-tray"
    };
    let path = exe.parent()?.join(name);
    path.is_file().then_some(path)
}

fn quote_schtasks_arg(arg: &str) -> String {
    let escaped = xml_escape(arg);
    format!("&quot;{escaped}&quot;")
}

pub fn schtasks_xml(exe: &Path, config: &Path, workdir: &Path, log: &Path, user: &str) -> String {
    let workdir_str = xml_escape(&workdir.to_string_lossy());
    let user_str = xml_escape(user);
    // S4U needs elevation to register; InteractiveToken would open a console window without conhost --headless.
    let arguments = format!(
        "--headless {} up --config {} --log-file {} --exit-with-parent",
        quote_schtasks_arg(&exe.to_string_lossy()),
        quote_schtasks_arg(&config.to_string_lossy()),
        quote_schtasks_arg(&log.to_string_lossy()),
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>SWING mirror agent</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user_str}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user_str}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Hidden>true</Hidden>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>%SystemRoot%\System32\conhost.exe</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{workdir_str}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(not(windows))]
fn decode_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

// schtasks writes in the OEM code page (CP932 on Japanese Windows), not UTF-8.
#[cfg(windows)]
fn decode_output(bytes: &[u8]) -> String {
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

fn run_command(mut cmd: std::process::Command) -> Result<std::process::Output> {
    let display = format!("{cmd:?}");
    let output = cmd
        .output()
        .with_context(|| format!("running command: {display}"))?;
    if !output.status.success() {
        bail!(
            "command failed: {display}\nstderr: {}",
            decode_output(&output.stderr)
        );
    }
    Ok(output)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::process::{Command, Stdio};

    fn unit_path(system: bool) -> Result<PathBuf> {
        if system {
            return Ok(PathBuf::from("/etc/systemd/system/swing.service"));
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .context("cannot determine XDG_CONFIG_HOME or HOME")?;
        Ok(base.join("systemd").join("user").join("swing.service"))
    }

    fn systemctl(system: bool) -> Command {
        let mut cmd = Command::new("systemctl");
        if !system {
            cmd.arg("--user");
        }
        cmd
    }

    pub fn start(system: bool) -> Result<()> {
        run_command({
            let mut cmd = systemctl(system);
            cmd.args(["start", "swing"]);
            cmd
        })?;
        println!("Started swing with systemd.");
        Ok(())
    }

    pub fn is_installed(system: bool) -> bool {
        unit_path(system).is_ok_and(|p| p.exists())
    }

    pub fn install(
        exe: &Path,
        config: &Path,
        workdir: &Path,
        system: bool,
        no_start: bool,
    ) -> Result<()> {
        let path = unit_path(system)?;
        ensure_parent_dir(&path)?;
        let unit = systemd_unit(exe, config, workdir, system);
        write_service_file(&path, unit)?;
        println!("Wrote systemd unit to {}.", path.display());

        run_command({
            let mut cmd = systemctl(system);
            cmd.args(["daemon-reload"]);
            cmd
        })?;

        run_command({
            let mut cmd = systemctl(system);
            cmd.arg("enable");
            if !no_start {
                cmd.arg("--now");
            }
            cmd.arg("swing");
            cmd
        })?;
        println!(
            "Registered swing with systemd ({}).",
            if system { "system" } else { "user" }
        );

        if !system {
            let uid = unsafe { libc::getuid() }.to_string();
            let linger = Command::new("loginctl")
                .args(["enable-linger", &uid])
                .output();
            match linger {
                Ok(out) if out.status.success() => {
                    println!("Enabled linger so swing keeps running while logged out.");
                }
                _ => {
                    println!(
                        "Warning: could not run `loginctl enable-linger`. Run it yourself so swing keeps running while you are logged out."
                    );
                }
            }
        }

        if no_start {
            println!(
                "Service installed but not started (--no-start). Start it with `systemctl {}start swing`.",
                if system { "" } else { "--user " }
            );
        } else {
            println!(
                "Follow logs with `journalctl {}-u swing -f`.",
                if system { "" } else { "--user " }
            );
        }
        Ok(())
    }

    pub fn stop(system: bool) -> Result<()> {
        run_command({
            let mut cmd = systemctl(system);
            cmd.args(["stop", "swing"]);
            cmd
        })?;
        println!("Stopped swing.");
        Ok(())
    }

    pub fn uninstall(system: bool) -> Result<()> {
        let path = unit_path(system)?;
        let disable = {
            let mut cmd = systemctl(system);
            cmd.args(["disable", "--now", "swing"]);
            cmd.output()
        };
        if let Ok(out) = disable
            && !out.status.success()
        {
            println!(
                "Note: `systemctl disable --now swing` did not succeed (it may not have been loaded)."
            );
        }
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            println!("Removed {}.", path.display());
        } else {
            println!("No unit file found at {}.", path.display());
        }
        run_command({
            let mut cmd = systemctl(system);
            cmd.args(["daemon-reload"]);
            cmd
        })?;
        println!("Uninstalled swing from systemd.");
        Ok(())
    }

    pub fn status(system: bool) -> Result<()> {
        let path = unit_path(system)?;
        if !path.exists() {
            println!("not installed");
            return Ok(());
        }
        let mut cmd = systemctl(system);
        cmd.args(["status", "swing", "--no-pager"]);
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let _ = cmd.status();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::process::Command;

    const LABEL: &str = "jp.ne.ama.swing";

    const TRAY_LABEL: &str = "jp.ne.ama.swing-tray";

    fn agent_plist_path(label: &str) -> Result<PathBuf> {
        let home = std::env::var_os("HOME").context("cannot determine HOME")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{label}.plist")))
    }

    fn plist_path() -> Result<PathBuf> {
        agent_plist_path(LABEL)
    }

    fn log_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").context("cannot determine HOME")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Logs")
            .join("swing.log"))
    }

    fn uid() -> u32 {
        unsafe { libc::getuid() }
    }

    fn service_target() -> String {
        format!("gui/{}/{LABEL}", uid())
    }

    pub fn install(exe: &Path, config: &Path, workdir: &Path, no_start: bool) -> Result<()> {
        let path = plist_path()?;
        let log = log_path()?;
        ensure_parent_dir(&log)?;
        ensure_parent_dir(&path)?;

        if is_loaded() {
            let _ = Command::new("launchctl")
                .args(["bootout", &service_target()])
                .output();
        }

        let plist = launchd_plist(exe, config, workdir, &log);
        write_service_file(&path, plist)?;
        println!("Wrote launchd agent to {}.", path.display());

        if !no_start {
            run_command({
                let mut cmd = Command::new("launchctl");
                cmd.args([
                    "bootstrap",
                    &format!("gui/{}", uid()),
                    &path.to_string_lossy(),
                ]);
                cmd
            })?;
            println!("Loaded swing with launchd.");
        } else {
            println!(
                "Service installed but not started (--no-start). Load it with `launchctl bootstrap gui/{} {}`.",
                uid(),
                path.display()
            );
        }
        println!("Logs are written to {}.", log.display());
        Ok(())
    }

    pub fn install_tray(tray: &Path, config: &Path, workdir: &Path, no_start: bool) -> Result<()> {
        let path = agent_plist_path(TRAY_LABEL)?;
        let target = format!("gui/{}/{TRAY_LABEL}", uid());
        let _ = Command::new("launchctl")
            .args(["bootout", &target])
            .output();
        write_service_file(&path, launchd_tray_plist(tray, config, workdir))?;
        println!(
            "Registered swing-tray to start at login ({}).",
            path.display()
        );
        if !no_start {
            run_command({
                let mut cmd = Command::new("launchctl");
                cmd.args([
                    "bootstrap",
                    &format!("gui/{}", uid()),
                    &path.to_string_lossy(),
                ]);
                cmd
            })?;
            println!("Started swing-tray.");
        }
        Ok(())
    }

    pub fn uninstall_tray() -> Result<()> {
        let path = agent_plist_path(TRAY_LABEL)?;
        if !path.exists() {
            return Ok(());
        }
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("gui/{}/{TRAY_LABEL}", uid())])
            .output();
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        println!("Removed {}.", path.display());
        Ok(())
    }

    fn is_loaded() -> bool {
        Command::new("launchctl")
            .args(["print", &service_target()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn start(_system: bool) -> Result<()> {
        if is_loaded() {
            run_command({
                let mut cmd = Command::new("launchctl");
                cmd.args(["kickstart", &service_target()]);
                cmd
            })?;
        } else {
            let path = plist_path()?;
            if !path.exists() {
                bail!("swing is not registered as a service; run `swing service install` first");
            }
            run_command({
                let mut cmd = Command::new("launchctl");
                cmd.args([
                    "bootstrap",
                    &format!("gui/{}", uid()),
                    &path.to_string_lossy(),
                ]);
                cmd
            })?;
        }
        println!("Started swing with launchd.");
        Ok(())
    }

    pub fn is_installed(_system: bool) -> bool {
        plist_path().is_ok_and(|p| p.exists())
    }

    pub fn stop(_system: bool) -> Result<()> {
        run_command({
            let mut cmd = Command::new("launchctl");
            cmd.args(["kill", "SIGTERM", &service_target()]);
            cmd
        })?;
        println!(
            "Sent SIGTERM to swing via launchctl. It stays stopped until the next login; start it again with `launchctl kickstart -k gui/<uid>/jp.ne.ama.swing`."
        );
        Ok(())
    }

    pub fn uninstall(_system: bool) -> Result<()> {
        let path = plist_path()?;
        let _ = Command::new("launchctl")
            .args(["bootout", &service_target()])
            .output();
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            println!("Removed {}.", path.display());
        } else {
            println!("No launch agent found at {}.", path.display());
        }
        println!("Uninstalled swing from launchd.");
        Ok(())
    }

    pub fn status(_system: bool) -> Result<()> {
        let path = plist_path()?;
        if !path.exists() {
            println!("not installed");
            return Ok(());
        }
        let mut cmd = Command::new("launchctl");
        cmd.args(["print", &service_target()]);
        cmd.stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let _ = cmd.status();
        Ok(())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
    };
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    const TASK_NAME: &str = "swing";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE: &str = "swing-tray";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn install_tray(tray: &Path, config: &Path, workdir: &Path, no_start: bool) -> Result<()> {
        let data = wide(&tray_run_command(tray, config));
        let len = u32::try_from(data.len() * 2).context("swing-tray command line is too long")?;
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(RUN_VALUE).as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                len,
            )
        };
        if status != ERROR_SUCCESS {
            bail!("writing HKCU\\{RUN_KEY}\\{RUN_VALUE} failed (error {status})");
        }
        println!("Registered swing-tray to start at login (HKCU\\{RUN_KEY}\\{RUN_VALUE}).");
        if !no_start {
            Command::new(tray)
                .arg("--config")
                .arg(config)
                .current_dir(workdir)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .with_context(|| format!("starting {}", tray.display()))?;
            println!("Started swing-tray.");
        }
        Ok(())
    }

    pub fn uninstall_tray() -> Result<()> {
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(RUN_VALUE).as_ptr(),
            )
        };
        match status {
            ERROR_SUCCESS => {
                println!("Removed swing-tray from HKCU\\{RUN_KEY}.");
                Ok(())
            }
            ERROR_FILE_NOT_FOUND => Ok(()),
            other => bail!("removing HKCU\\{RUN_KEY}\\{RUN_VALUE} failed (error {other})"),
        }
    }

    // swing-tray is a GUI-subsystem process; without this each schtasks call flashes a console window.
    fn schtasks() -> Command {
        let mut cmd = Command::new("schtasks");
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }

    fn current_user() -> Result<String> {
        let username = std::env::var("USERNAME").context("USERNAME is not set")?;
        if let Ok(domain) = std::env::var("USERDOMAIN")
            && !domain.is_empty()
        {
            return Ok(format!("{domain}\\{username}"));
        }
        Ok(username)
    }

    fn log_path(workdir: &Path) -> PathBuf {
        workdir.join("swing.log")
    }

    // A predictable name in a shared temp dir lets another local user pre-create or symlink the
    // path; create_new() with a random suffix closes that race.
    fn write_service_file_new(path: &Path, content: impl AsRef<[u8]>) -> Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        file.write_all(content.as_ref())
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn install(exe: &Path, config: &Path, workdir: &Path, no_start: bool) -> Result<()> {
        let user = current_user()?;
        let log = log_path(workdir);
        let xml = schtasks_xml(exe, config, workdir, &log, &user);

        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!("swing-task-{}.xml", crate::auth::random_hex(8)));
        write_service_file_new(&tmp_path, xml)?;

        let result = run_command({
            let mut cmd = schtasks();
            cmd.args([
                "/Create",
                "/TN",
                TASK_NAME,
                "/XML",
                &tmp_path.to_string_lossy(),
                "/F",
            ]);
            cmd
        });
        let _ = std::fs::remove_file(&tmp_path);
        result?;
        println!("Registered swing with Task Scheduler as \"{TASK_NAME}\".");
        println!("Logs are written to {}.", log.display());

        if !no_start {
            run_command({
                let mut cmd = schtasks();
                cmd.args(["/Run", "/TN", TASK_NAME]);
                cmd
            })?;
            println!("Started the task.");
        } else {
            println!(
                "Service installed but not started (--no-start). Start it with `schtasks /Run /TN {TASK_NAME}`."
            );
        }
        println!("Check status with `schtasks /Query /TN {TASK_NAME}`.");
        Ok(())
    }

    pub fn start(_system: bool) -> Result<()> {
        run_command({
            let mut cmd = schtasks();
            cmd.args(["/Run", "/TN", TASK_NAME]);
            cmd
        })?;
        println!("Started the task.");
        Ok(())
    }

    pub fn is_installed(_system: bool) -> bool {
        schtasks()
            .args(["/Query", "/TN", TASK_NAME])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    pub async fn stop(_system: bool) -> Result<()> {
        let (config, _workdir, _exe) = resolve_service_paths(None)?;
        let cfg = crate::config::Config::load(Some(&config))?;
        match crate::stop::run(&cfg, false, std::time::Duration::from_secs(60)).await {
            Ok(()) => Ok(()),
            Err(e) => {
                println!("Warning: graceful stop failed ({e:#}); falling back to `schtasks /End`.");
                run_command({
                    let mut cmd = schtasks();
                    cmd.args(["/End", "/TN", TASK_NAME]);
                    cmd
                })?;
                Ok(())
            }
        }
    }

    pub async fn uninstall(_system: bool) -> Result<()> {
        if let Ok((config, _workdir, _exe)) = resolve_service_paths(None)
            && let Ok(cfg) = crate::config::Config::load(Some(&config))
        {
            let _ = crate::stop::run(&cfg, false, std::time::Duration::from_secs(60)).await;
        }
        let _ = schtasks().args(["/End", "/TN", TASK_NAME]).output();
        run_command({
            let mut cmd = schtasks();
            cmd.args(["/Delete", "/TN", TASK_NAME, "/F"]);
            cmd
        })?;
        println!("Uninstalled swing from Task Scheduler.");
        Ok(())
    }

    pub fn status(_system: bool) -> Result<()> {
        let output = schtasks()
            .args(["/Query", "/TN", TASK_NAME, "/FO", "LIST", "/V"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                print!("{}", decode_output(&out.stdout));
                Ok(())
            }
            _ => {
                println!("not installed");
                Ok(())
            }
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn install_tray(
    exe: &Path,
    config: &Path,
    workdir: &Path,
    no_start: bool,
    no_tray: bool,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    use macos as platform;
    #[cfg(windows)]
    use windows as platform;

    if no_tray {
        return platform::uninstall_tray();
    }
    let Some(tray) = tray_exe_path(exe) else {
        println!(
            "swing-tray was not found next to {}; skipping the tray icon.",
            exe.display()
        );
        return Ok(());
    };
    platform::install_tray(&tray, config, workdir, no_start)
}

pub fn install(
    config_path: Option<&Path>,
    system: bool,
    no_start: bool,
    no_tray: bool,
) -> Result<()> {
    require_system_supported(system)?;
    let (config, workdir, exe) = resolve_service_paths(config_path)?;

    #[cfg(target_os = "linux")]
    {
        let _ = no_tray;
        linux::install(&exe, &config, &workdir, system, no_start)
    }
    #[cfg(target_os = "macos")]
    {
        macos::install(&exe, &config, &workdir, no_start)?;
        install_tray(&exe, &config, &workdir, no_start, no_tray)
    }
    #[cfg(windows)]
    {
        windows::install(&exe, &config, &workdir, no_start)?;
        install_tray(&exe, &config, &workdir, no_start, no_tray)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        bail!("service management is not supported on this OS");
    }
}

pub async fn uninstall(system: bool) -> Result<()> {
    require_system_supported(system)?;

    #[cfg(target_os = "linux")]
    {
        linux::uninstall(system)
    }
    #[cfg(target_os = "macos")]
    {
        macos::uninstall_tray()?;
        macos::uninstall(system)
    }
    #[cfg(windows)]
    {
        windows::uninstall_tray()?;
        windows::uninstall(system).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        bail!("service management is not supported on this OS");
    }
}

pub async fn stop(system: bool) -> Result<()> {
    require_system_supported(system)?;

    #[cfg(target_os = "linux")]
    {
        linux::stop(system)
    }
    #[cfg(target_os = "macos")]
    {
        macos::stop(system)
    }
    #[cfg(windows)]
    {
        windows::stop(system).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        bail!("service management is not supported on this OS");
    }
}

pub fn start(system: bool) -> Result<()> {
    require_system_supported(system)?;

    #[cfg(target_os = "linux")]
    {
        linux::start(system)
    }
    #[cfg(target_os = "macos")]
    {
        macos::start(system)
    }
    #[cfg(windows)]
    {
        windows::start(system)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        bail!("service management is not supported on this OS");
    }
}

pub fn is_installed(system: bool) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::is_installed(system)
    }
    #[cfg(target_os = "macos")]
    {
        macos::is_installed(system)
    }
    #[cfg(windows)]
    {
        windows::is_installed(system)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = system;
        false
    }
}

pub fn status(system: bool) -> Result<()> {
    require_system_supported(system)?;

    #[cfg(target_os = "linux")]
    {
        linux::status(system)
    }
    #[cfg(target_os = "macos")]
    {
        macos::status(system)
    }
    #[cfg(windows)]
    {
        windows::status(system)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        bail!("service management is not supported on this OS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_plist_runs_the_tray_with_the_config_in_gui_sessions_only() {
        let plist = launchd_tray_plist(
            Path::new("/opt/swing/swing-tray"),
            Path::new("/Users/a & b/swing.toml"),
            Path::new("/Users/a & b"),
        );
        assert!(plist.contains("<string>jp.ne.ama.swing-tray</string>"));
        assert!(plist.contains(
            "<string>/opt/swing/swing-tray</string>\n        <string>--config</string>\n        <string>/Users/a &amp; b/swing.toml</string>"
        ));
        assert!(plist.contains("<key>LimitLoadToSessionType</key>\n    <string>Aqua</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(!plist.contains("KeepAlive"));
    }

    #[test]
    fn tray_run_command_quotes_paths_and_drops_verbatim_prefixes() {
        assert_eq!(
            tray_run_command(
                Path::new(r"\\?\C:\Program Files\swing\swing-tray.exe"),
                Path::new(r"\\?\C:\Users\a\swing.toml"),
            ),
            r#""C:\Program Files\swing\swing-tray.exe" --config "C:\Users\a\swing.toml""#
        );
        assert_eq!(
            tray_run_command(
                Path::new(r"\\?\UNC\server\share\swing-tray.exe"),
                Path::new(r"D:\swing.toml"),
            ),
            r#""\\server\share\swing-tray.exe" --config "D:\swing.toml""#
        );
    }

    #[test]
    fn tray_exe_is_found_only_next_to_swing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("swing");
        assert_eq!(tray_exe_path(&exe), None);
        let name = if cfg!(windows) {
            "swing-tray.exe"
        } else {
            "swing-tray"
        };
        std::fs::write(dir.path().join(name), b"").unwrap();
        assert_eq!(tray_exe_path(&exe), Some(dir.path().join(name)));
    }

    #[test]
    fn systemd_unit_user_scope() {
        let unit = systemd_unit(
            Path::new("/home/u/.cargo/bin/swing"),
            Path::new("/home/u/swing.toml"),
            Path::new("/home/u"),
            false,
        );
        assert!(unit.contains("Description=SWING mirror agent"));
        assert!(unit.contains("After=network-online.target"));
        assert!(unit.contains("Wants=network-online.target"));
        assert!(
            unit.contains(
                "ExecStart=\"/home/u/.cargo/bin/swing\" up --config \"/home/u/swing.toml\""
            )
        );
        assert!(unit.contains("WorkingDirectory=/home/u\n"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("RestartSec=5"));
        assert!(unit.contains("KillSignal=SIGTERM"));
        assert!(unit.contains("TimeoutStopSec=60"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_unit_system_scope_differs() {
        let unit = systemd_unit(
            Path::new("/usr/bin/swing"),
            Path::new("/etc/swing/swing.toml"),
            Path::new("/etc/swing"),
            true,
        );
        assert!(unit.contains("WantedBy=multi-user.target"));
        assert!(!unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_unit_quotes_paths_with_spaces_and_escapes() {
        let unit = systemd_unit(
            Path::new("/home/u/my apps/swing"),
            Path::new("/home/u/my dir/swing.toml"),
            Path::new("/home/u/my dir"),
            false,
        );
        assert!(unit.contains("\"/home/u/my apps/swing\""));
        assert!(unit.contains("\"/home/u/my dir/swing.toml\""));
        assert!(unit.contains("WorkingDirectory=/home/u/my dir\n"));
    }

    #[test]
    fn systemd_unit_escapes_quotes_and_backslashes() {
        let quoted = quote_systemd_arg("a\"b\\c");
        assert_eq!(quoted, "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn systemd_unit_escapes_specifiers_and_variables() {
        let unit = systemd_unit(
            Path::new("/home/u/$HOME/swing"),
            Path::new("/home/u/100%/swing.toml"),
            Path::new("/home/u/100%"),
            false,
        );
        assert!(unit.contains("\"/home/u/$$HOME/swing\""));
        assert!(unit.contains("\"/home/u/100%%/swing.toml\""));
        assert!(unit.contains("WorkingDirectory=/home/u/100%%\n"));
    }

    #[test]
    fn launchd_plist_has_expected_keys() {
        let plist = launchd_plist(
            Path::new("/usr/local/bin/swing"),
            Path::new("/Users/u/swing.toml"),
            Path::new("/Users/u"),
            Path::new("/Users/u/Library/Logs/swing.log"),
        );
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains("<string>jp.ne.ama.swing</string>"));
        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(plist.contains("<string>/usr/local/bin/swing</string>"));
        assert!(plist.contains("<string>up</string>"));
        assert!(plist.contains("<string>--config</string>"));
        assert!(plist.contains("<string>/Users/u/swing.toml</string>"));
        assert!(plist.contains("<key>WorkingDirectory</key>"));
        assert!(plist.contains("<string>/Users/u</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<true/>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<key>SuccessfulExit</key>\n        <false/>"));
        assert!(plist.contains("<key>StandardOutPath</key>"));
        assert!(plist.contains("<string>/Users/u/Library/Logs/swing.log</string>"));
        assert!(plist.contains("<key>StandardErrorPath</key>"));
        assert!(plist.contains("<key>EnvironmentVariables</key>"));
        assert!(plist.contains("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"));
    }

    #[test]
    fn launchd_plist_xml_escapes_values() {
        let plist = launchd_plist(
            Path::new("/usr/local/bin/swing"),
            Path::new("/Users/u/A&B<C>.toml"),
            Path::new("/Users/u"),
            Path::new("/Users/u/Library/Logs/swing.log"),
        );
        assert!(plist.contains("A&amp;B&lt;C&gt;.toml"));
        assert!(!plist.contains("A&B<C>.toml"));
    }

    #[test]
    fn schtasks_xml_has_expected_elements() {
        let xml = schtasks_xml(
            Path::new(r"C:\Program Files\swing\swing.exe"),
            Path::new(r"C:\Users\u\swing.toml"),
            Path::new(r"C:\Users\u"),
            Path::new(r"C:\Users\u\swing.log"),
            "DOMAIN\\user",
        );
        assert!(xml.contains(r#"xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task""#));
        assert!(xml.contains("<RegistrationInfo>"));
        assert!(xml.contains("<Description>SWING mirror agent</Description>"));
        assert!(xml.contains("<LogonTrigger>"));
        assert!(xml.contains("<UserId>DOMAIN\\user</UserId>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<Hidden>true</Hidden>"));
        assert!(!xml.contains("<RestartOnFailure>"));
        assert!(xml.contains("<Command>%SystemRoot%\\System32\\conhost.exe</Command>"));
        assert!(xml.contains(
            "<Arguments>--headless &quot;C:\\Program Files\\swing\\swing.exe&quot; up --config"
        ));
        assert!(xml.contains("<WorkingDirectory>C:\\Users\\u</WorkingDirectory>"));
        assert!(xml.contains("&quot;C:\\Users\\u\\swing.toml&quot;"));
        assert!(xml.contains("&quot;C:\\Users\\u\\swing.log&quot; --exit-with-parent</Arguments>"));
    }

    #[test]
    fn schtasks_xml_escapes_ampersand_and_angle_brackets() {
        let xml = schtasks_xml(
            Path::new(r"C:\swing.exe"),
            Path::new(r"C:\a&b<c>.toml"),
            Path::new(r"C:\dir"),
            Path::new(r"C:\log.txt"),
            "user<1>&",
        );
        assert!(xml.contains("a&amp;b&lt;c&gt;.toml"));
        assert!(xml.contains("<UserId>user&lt;1&gt;&amp;</UserId>"));
        assert!(!xml.contains("a&b<c>.toml"));
    }
}
