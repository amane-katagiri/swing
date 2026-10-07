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
mod tests {
    use super::*;

    #[test]
    fn task_xml_is_utf16le_with_a_bom_to_match_its_declaration() {
        let xml = schtasks_xml(
            Path::new(r"C:\Users\片桐\AppData\Local\Programs\SWING\swing.exe"),
            Path::new(r"C:\Users\片桐\swing.toml"),
            Path::new(r"C:\Users\片桐"),
            Path::new(r"C:\Users\片桐\swing.log"),
            "片桐",
        )
        .unwrap();
        assert!(xml.starts_with(r#"<?xml version="1.0" encoding="UTF-16"?>"#));
        let bytes = task_xml_bytes(&xml);
        assert_eq!(&bytes[..4], &[0xff, 0xfe, b'<', 0]);
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), xml);
    }

    const SYSTEM: SystemdScope<'static> = SystemdScope::System {
        user: "swing",
        writable: &[],
    };

    #[test]
    fn tray_plist_runs_the_tray_with_the_config_in_gui_sessions_only() {
        let plist = launchd_tray_plist(
            Path::new("/opt/swing/swing-tray"),
            Path::new("/Users/a & b/swing.toml"),
            Path::new("/Users/a & b"),
        )
        .unwrap();
        assert!(plist.contains("<string>jp.ne.ama.swing-tray</string>"));
        assert!(plist.contains(
            "<string>/opt/swing/swing-tray</string>\n        <string>--config</string>\n        <string>/Users/a &amp; b/swing.toml</string>"
        ));
        assert!(plist.contains("<key>LimitLoadToSessionType</key>\n    <string>Aqua</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(!plist.contains("KeepAlive"));
        assert!(plist.contains(
            "<key>AssociatedBundleIdentifiers</key>\n    <array>\n        <string>jp.ne.ama.swing</string>"
        ));
    }

    #[test]
    fn tray_run_command_quotes_paths_and_drops_verbatim_prefixes() {
        assert_eq!(
            tray_run_command(
                Path::new(r"\\?\C:\Program Files\swing\swing-tray.exe"),
                Path::new(r"\\?\C:\Users\a\swing.toml"),
            )
            .unwrap(),
            r#""C:\Program Files\swing\swing-tray.exe" --config "C:\Users\a\swing.toml""#
        );
        assert_eq!(
            tray_run_command(
                Path::new(r"\\?\UNC\server\share\swing-tray.exe"),
                Path::new(r"D:\swing.toml"),
            )
            .unwrap(),
            r#""\\server\share\swing-tray.exe" --config "D:\swing.toml""#
        );
    }

    #[test]
    fn systemd_unit_user_scope() {
        let unit = systemd_unit(
            Path::new("/home/u/.cargo/bin/swing"),
            Path::new("/home/u/swing.toml"),
            Path::new("/home/u"),
            &SystemdScope::User,
        )
        .unwrap();
        assert!(unit.contains("Description=SWING mirror agent"));
        assert!(unit.contains("After=network-online.target"));
        assert!(unit.contains("Wants=network-online.target"));
        assert!(
            unit.contains(
                "ExecStart=\"/home/u/.cargo/bin/swing\" up --config \"/home/u/swing.toml\""
            )
        );
        assert!(unit.contains("WorkingDirectory=/home/u\nRestart=on-failure\n"));
        assert!(unit.contains("RestartSec=5"));
        assert!(unit.contains("KillSignal=SIGTERM"));
        assert!(unit.contains("TimeoutStopSec=90\n"));
        assert!(unit.contains("WantedBy=default.target"));
        assert!(!unit.contains("User="));
        assert!(!unit.contains("ProtectSystem"));
    }

    #[test]
    fn systemd_unit_system_scope_runs_as_the_user_with_hardening() {
        let unit = systemd_unit(
            Path::new("/usr/bin/swing"),
            Path::new("/etc/swing/swing.toml"),
            Path::new("/etc/swing"),
            &SYSTEM,
        )
        .unwrap();
        assert!(unit.contains(
            "WorkingDirectory=/etc/swing\nUser=swing\nNoNewPrivileges=yes\nPrivateTmp=yes\nProtectSystem=full\nReadWritePaths=\"/etc/swing\"\nRestart=on-failure\n"
        ));
        assert!(!unit.contains("ProtectHome"));
        assert!(unit.contains("WantedBy=multi-user.target"));
        assert!(!unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_unit_system_scope_adds_writable_paths_outside_the_workdir() {
        let writable = [
            PathBuf::from("/etc/swing/data"),
            PathBuf::from("/srv/swing state"),
            PathBuf::from("/srv/kubo"),
        ];
        let unit = systemd_unit(
            Path::new("/usr/bin/swing"),
            Path::new("/etc/swing/swing.toml"),
            Path::new("/etc/swing"),
            &SystemdScope::System {
                user: "swing",
                writable: &writable,
            },
        )
        .unwrap();
        assert!(
            unit.contains("ReadWritePaths=\"/etc/swing\" \"-/srv/swing state\" \"-/srv/kubo\"\n")
        );
        let injected = [PathBuf::from("/srv/x\nUser=root")];
        assert!(
            systemd_unit(
                Path::new("/usr/bin/swing"),
                Path::new("/etc/swing/swing.toml"),
                Path::new("/etc/swing"),
                &SystemdScope::System {
                    user: "swing",
                    writable: &injected,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn systemd_unit_quotes_paths_with_spaces_and_escapes() {
        let unit = systemd_unit(
            Path::new("/home/u/my apps/swing"),
            Path::new("/home/u/my dir/swing.toml"),
            Path::new("/home/u/my dir"),
            &SYSTEM,
        )
        .unwrap();
        assert!(unit.contains("\"/home/u/my apps/swing\""));
        assert!(unit.contains("\"/home/u/my dir/swing.toml\""));
        assert!(unit.contains("WorkingDirectory=/home/u/my dir\n"));
        assert!(unit.contains("ReadWritePaths=\"/home/u/my dir\"\n"));
    }

    #[test]
    fn systemd_unit_escapes_quotes_and_backslashes() {
        let quoted = quote_systemd_arg("a\"b\\c");
        assert_eq!(quoted, "\"a\\\"b\\\\c\"");
        assert_eq!(quote_systemd_path("a\"b\\c$"), "\"a\\\"b\\\\c$\"");
    }

    #[test]
    fn systemd_unit_escapes_specifiers_and_variables() {
        let unit = systemd_unit(
            Path::new("/home/u/$HOME/swing"),
            Path::new("/home/u/100%/swing.toml"),
            Path::new("/home/u/100%"),
            &SYSTEM,
        )
        .unwrap();
        assert!(unit.contains("\"/home/u/$$HOME/swing\""));
        assert!(unit.contains("\"/home/u/100%%/swing.toml\""));
        assert!(unit.contains("WorkingDirectory=/home/u/100%%\n"));
        assert!(unit.contains("ReadWritePaths=\"/home/u/100%%\"\n"));
    }

    #[test]
    fn templates_reject_control_characters_in_paths_and_users() {
        let good = Path::new("/home/u/swing.toml");
        let injected = Path::new("/home/u/x\nExecStartPre=/bin/sh");
        for scope in [SystemdScope::User, SYSTEM] {
            assert!(systemd_unit(injected, good, good, &scope).is_err());
            assert!(systemd_unit(good, injected, good, &scope).is_err());
            assert!(systemd_unit(good, good, injected, &scope).is_err());
        }
        let err = systemd_unit(
            good,
            good,
            good,
            &SystemdScope::System {
                user: "u\nUser=root",
                writable: &[],
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("control character"), "{err:#}");

        assert!(launchd_plist(good, good, good, Path::new("/tmp/a\rb")).is_err());
        assert!(launchd_tray_plist(good, Path::new("/a\tb"), good).is_err());
        assert!(tray_run_command(good, Path::new("C:\\a\u{7f}b")).is_err());
        assert!(schtasks_xml(good, good, good, Path::new("C:\\a\0b"), "u").is_err());
        assert!(schtasks_xml(good, good, good, good, "u\n").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn templates_reject_non_utf8_paths() {
        use std::os::unix::ffi::OsStrExt;

        let good = Path::new("/home/u/swing.toml");
        let bad = Path::new(std::ffi::OsStr::from_bytes(b"/home/u/\xff.toml"));
        let err = systemd_unit(good, bad, good, &SystemdScope::User).unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"), "{err:#}");
        assert!(launchd_plist(good, good, bad, good).is_err());
    }

    #[test]
    fn launchd_plist_has_expected_keys() {
        let plist = launchd_plist(
            Path::new("/usr/local/bin/swing"),
            Path::new("/Users/u/swing.toml"),
            Path::new("/Users/u"),
            Path::new("/Users/u/Library/Logs/swing.log"),
        )
        .unwrap();
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
        assert!(plist.contains("<key>ExitTimeOut</key>\n    <integer>90</integer>"));
        assert!(plist.contains("<key>StandardOutPath</key>"));
        assert!(plist.contains("<string>/Users/u/Library/Logs/swing.log</string>"));
        assert!(plist.contains("<key>StandardErrorPath</key>"));
        assert!(plist.contains("<key>EnvironmentVariables</key>"));
        assert!(plist.contains("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"));
        assert!(plist.contains(
            "<key>AssociatedBundleIdentifiers</key>\n    <array>\n        <string>jp.ne.ama.swing</string>"
        ));
    }

    #[test]
    fn launchd_plist_xml_escapes_values() {
        let plist = launchd_plist(
            Path::new("/usr/local/bin/swing"),
            Path::new("/Users/u/A&B<C>.toml"),
            Path::new("/Users/u"),
            Path::new("/Users/u/Library/Logs/swing.log"),
        )
        .unwrap();
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
        )
        .unwrap();
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
        )
        .unwrap();
        assert!(xml.contains("a&amp;b&lt;c&gt;.toml"));
        assert!(xml.contains("<UserId>user&lt;1&gt;&amp;</UserId>"));
        assert!(!xml.contains("a&b<c>.toml"));
    }
}
