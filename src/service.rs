use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::resolve_config_path;

fn resolve_service_paths(config_path: Option<&Path>) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let config = resolve_config_path(config_path)
        .filter(|p| p.exists())
        .context(
            "service install needs a config file (swing.toml): pass --config or set SWING_CONFIG",
        )?;
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

fn quote_systemd_arg(arg: &str) -> String {
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"");
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
        workdir = quote_systemd_arg(&workdir.to_string_lossy()),
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

fn quote_schtasks_arg(arg: &str) -> String {
    let escaped = xml_escape(arg);
    format!("&quot;{escaped}&quot;")
}

pub fn schtasks_xml(exe: &Path, config: &Path, workdir: &Path, log: &Path, user: &str) -> String {
    let exe_str = xml_escape(&exe.to_string_lossy());
    let workdir_str = xml_escape(&workdir.to_string_lossy());
    let user_str = xml_escape(user);
    let arguments = format!(
        "up --config {} --log-file {}",
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
      <LogonType>S4U</LogonType>
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
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe_str}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{workdir_str}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn run_command(mut cmd: std::process::Command) -> Result<std::process::Output> {
    let display = format!("{cmd:?}");
    let output = cmd
        .output()
        .with_context(|| format!("running command: {display}"))?;
    if !output.status.success() {
        bail!(
            "command failed: {display}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
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

    pub fn install(
        exe: &Path,
        config: &Path,
        workdir: &Path,
        system: bool,
        no_start: bool,
    ) -> Result<()> {
        let path = unit_path(system)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        let unit = systemd_unit(exe, config, workdir, system);
        std::fs::write(&path, unit).with_context(|| format!("writing {}", path.display()))?;
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
            let linger = Command::new("loginctl").args(["enable-linger"]).output();
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

    fn plist_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").context("cannot determine HOME")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist")))
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
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }

        let already_loaded = Command::new("launchctl")
            .args(["print", &service_target()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if already_loaded {
            let _ = Command::new("launchctl")
                .args(["bootout", &service_target()])
                .output();
        }

        let plist = launchd_plist(exe, config, workdir, &log);
        std::fs::write(&path, plist).with_context(|| format!("writing {}", path.display()))?;
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
    use std::process::Command;

    const TASK_NAME: &str = "swing";

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

    pub fn install(exe: &Path, config: &Path, workdir: &Path, no_start: bool) -> Result<()> {
        let user = current_user()?;
        let log = log_path(workdir);
        let xml = schtasks_xml(exe, config, workdir, &log, &user);

        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!("swing-task-{}.xml", std::process::id()));
        std::fs::write(&tmp_path, xml)
            .with_context(|| format!("writing {}", tmp_path.display()))?;

        let result = run_command({
            let mut cmd = Command::new("schtasks");
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
                let mut cmd = Command::new("schtasks");
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

    pub async fn stop(_system: bool) -> Result<()> {
        let (config, _workdir, _exe) = resolve_service_paths(None)?;
        let cfg = crate::config::Config::load(Some(&config))?;
        match crate::stop::run(&cfg, false, std::time::Duration::from_secs(60)).await {
            Ok(()) => Ok(()),
            Err(e) => {
                println!("Warning: graceful stop failed ({e:#}); falling back to `schtasks /End`.");
                run_command({
                    let mut cmd = Command::new("schtasks");
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
        let _ = Command::new("schtasks")
            .args(["/End", "/TN", TASK_NAME])
            .output();
        run_command({
            let mut cmd = Command::new("schtasks");
            cmd.args(["/Delete", "/TN", TASK_NAME, "/F"]);
            cmd
        })?;
        println!("Uninstalled swing from Task Scheduler.");
        Ok(())
    }

    pub fn status(_system: bool) -> Result<()> {
        let output = Command::new("schtasks")
            .args(["/Query", "/TN", TASK_NAME, "/FO", "LIST", "/V"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                print!("{}", String::from_utf8_lossy(&out.stdout));
                Ok(())
            }
            _ => {
                println!("not installed");
                Ok(())
            }
        }
    }
}

pub fn install(config_path: Option<&Path>, system: bool, no_start: bool) -> Result<()> {
    require_system_supported(system)?;
    let (config, workdir, exe) = resolve_service_paths(config_path)?;

    #[cfg(target_os = "linux")]
    {
        linux::install(&exe, &config, &workdir, system, no_start)
    }
    #[cfg(target_os = "macos")]
    {
        macos::install(&exe, &config, &workdir, no_start)
    }
    #[cfg(windows)]
    {
        windows::install(&exe, &config, &workdir, no_start)
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
        macos::uninstall(system)
    }
    #[cfg(windows)]
    {
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
        assert!(unit.contains("WorkingDirectory=\"/home/u\""));
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
        assert!(unit.contains("WorkingDirectory=\"/home/u/my dir\""));
    }

    #[test]
    fn systemd_unit_escapes_quotes_and_backslashes() {
        let quoted = quote_systemd_arg("a\"b\\c");
        assert_eq!(quoted, "\"a\\\"b\\\\c\"");
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
        assert!(xml.contains("<LogonType>S4U</LogonType>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<Hidden>true</Hidden>"));
        assert!(xml.contains("<RestartOnFailure>"));
        assert!(xml.contains("<Interval>PT1M</Interval>"));
        assert!(xml.contains("<Count>999</Count>"));
        assert!(xml.contains("<Command>C:\\Program Files\\swing\\swing.exe</Command>"));
        assert!(xml.contains("<WorkingDirectory>C:\\Users\\u</WorkingDirectory>"));
        assert!(xml.contains("&quot;C:\\Users\\u\\swing.toml&quot;"));
        assert!(xml.contains("&quot;C:\\Users\\u\\swing.log&quot;"));
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
