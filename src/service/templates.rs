use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::{MACOS_BUNDLE_ID, MACOS_LABEL, MACOS_TRAY_LABEL, STOP_TIMEOUT};

fn reject_control_chars(what: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        bail!(
            "{what} contains a control character and cannot be written into a service definition: {value:?}"
        );
    }
    Ok(())
}

fn text(what: &str, path: &Path) -> Result<String> {
    let Some(value) = path.to_str() else {
        bail!(
            "{what} is not valid UTF-8 and cannot be written into a service definition: {}",
            path.display()
        );
    };
    reject_control_chars(what, value)?;
    Ok(value.to_string())
}

fn escape_systemd_specifiers(value: &str) -> String {
    value.replace('%', "%%")
}

fn escape_systemd_quoted(value: &str) -> String {
    escape_systemd_specifiers(value)
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn quote_systemd_path(path: &str) -> String {
    format!("\"{}\"", escape_systemd_quoted(path))
}

fn quote_systemd_arg(arg: &str) -> String {
    format!("\"{}\"", escape_systemd_quoted(arg).replace('$', "$$"))
}

pub enum SystemdScope<'a> {
    User,
    System {
        user: &'a str,
        writable: &'a [PathBuf],
    },
}

pub fn systemd_unit(
    exe: &Path,
    config: &Path,
    workdir: &Path,
    scope: &SystemdScope<'_>,
) -> Result<String> {
    let exe = text("executable path", exe)?;
    let config = text("config path", config)?;
    let workdir = text("working directory", workdir)?;
    let exec_start = format!(
        "{} up --config {}",
        quote_systemd_arg(&exe),
        quote_systemd_arg(&config),
    );
    let (system_directives, wanted_by) = match scope {
        SystemdScope::User => (String::new(), "default.target"),
        SystemdScope::System { user, writable } => {
            reject_control_chars("service user", user)?;
            let mut read_write = vec![quote_systemd_path(&workdir)];
            for path in writable.iter().filter(|p| !p.starts_with(&workdir)) {
                let path = text("writable path", path)?;
                read_write.push(quote_systemd_path(&format!("-{path}")));
            }
            (
                format!(
                    "User={user}\n\
NoNewPrivileges=yes\n\
PrivateTmp=yes\n\
ProtectSystem=full\n\
ReadWritePaths={read_write}\n",
                    user = escape_systemd_specifiers(user),
                    read_write = read_write.join(" "),
                ),
                "multi-user.target",
            )
        }
    };
    Ok(format!(
        "[Unit]\n\
Description=SWING mirror agent\n\
After=network-online.target\n\
Wants=network-online.target\n\
\n\
[Service]\n\
ExecStart={exec_start}\n\
WorkingDirectory={workdir}\n\
{system_directives}\
Restart=on-failure\n\
RestartSec=5\n\
KillSignal=SIGTERM\n\
TimeoutStopSec={stop_timeout}\n\
\n\
[Install]\n\
WantedBy={wanted_by}\n",
        workdir = escape_systemd_specifiers(&workdir),
        stop_timeout = STOP_TIMEOUT.as_secs(),
    ))
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_text(what: &str, path: &Path) -> Result<String> {
    Ok(xml_escape(&text(what, path)?))
}

fn launchd_plist_for(label: &str, args: &[&str], workdir: &str, keys: &str) -> String {
    let args: String = args
        .iter()
        .map(|arg| format!("        <string>{arg}</string>\n"))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
    <key>WorkingDirectory</key>
    <string>{workdir}</string>
    <key>RunAtLoad</key>
    <true/>
{keys}    <key>AssociatedBundleIdentifiers</key>
    <array>
        <string>{MACOS_BUNDLE_ID}</string>
    </array>
</dict>
</plist>
"#
    )
}

pub fn launchd_plist(exe: &Path, config: &Path, workdir: &Path, log: &Path) -> Result<String> {
    let exe = xml_text("executable path", exe)?;
    let config = xml_text("config path", config)?;
    let workdir = xml_text("working directory", workdir)?;
    let log = xml_text("log path", log)?;
    let keys = format!(
        r#"    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ExitTimeOut</key>
    <integer>{exit_timeout}</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    </dict>
"#,
        exit_timeout = STOP_TIMEOUT.as_secs(),
    );
    Ok(launchd_plist_for(
        MACOS_LABEL,
        &[&exe, "up", "--config", &config],
        &workdir,
        &keys,
    ))
}

pub fn launchd_tray_plist(tray: &Path, config: &Path, workdir: &Path) -> Result<String> {
    let tray = xml_text("swing-tray path", tray)?;
    let config = xml_text("config path", config)?;
    let workdir = xml_text("working directory", workdir)?;
    let keys = r#"    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>ProcessType</key>
    <string>Interactive</string>
"#;
    Ok(launchd_plist_for(
        MACOS_TRAY_LABEL,
        &[&tray, "--config", &config],
        &workdir,
        keys,
    ))
}

// Explorer starts Run entries through CreateProcess, which is not documented to accept \\?\ paths.
pub(super) fn strip_verbatim(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
}

pub fn tray_run_command(tray: &Path, config: &Path) -> Result<String> {
    Ok(format!(
        "\"{}\" --config \"{}\"",
        strip_verbatim(&text("swing-tray path", tray)?),
        strip_verbatim(&text("config path", config)?)
    ))
}

fn quote_schtasks_arg(arg: &str) -> String {
    format!("&quot;{}&quot;", xml_escape(arg))
}

pub fn schtasks_xml(
    exe: &Path,
    config: &Path,
    workdir: &Path,
    log: &Path,
    user: &str,
) -> Result<String> {
    reject_control_chars("user name", user)?;
    let workdir_str = xml_text("working directory", workdir)?;
    let user_str = xml_escape(user);
    // S4U needs elevation to register; InteractiveToken would open a console window without conhost --headless.
    let arguments = format!(
        "--headless {} up --config {} --log-file {} --exit-with-parent",
        quote_schtasks_arg(&text("executable path", exe)?),
        quote_schtasks_arg(&text("config path", config)?),
        quote_schtasks_arg(&text("log path", log)?),
    );
    Ok(format!(
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
    ))
}

#[cfg(any(windows, test))]
pub(super) fn task_xml_bytes(xml: &str) -> Vec<u8> {
    [0xff, 0xfe]
        .into_iter()
        .chain(xml.encode_utf16().flat_map(u16::to_le_bytes))
        .collect()
}

#[cfg(test)]
mod tests;
