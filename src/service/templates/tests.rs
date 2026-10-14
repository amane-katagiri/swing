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
    assert_eq!(unit, USER_UNIT);
}

const USER_UNIT: &str = r#"[Unit]
Description=SWING mirror agent
After=network-online.target
Wants=network-online.target

[Service]
ExecStart="/home/u/.cargo/bin/swing" up --config "/home/u/swing.toml"
WorkingDirectory=/home/u
Restart=on-failure
RestartSec=5
KillSignal=SIGTERM
TimeoutStopSec=90

[Install]
WantedBy=default.target
"#;

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
    assert!(unit.contains("ReadWritePaths=\"/etc/swing\" \"-/srv/swing state\" \"-/srv/kubo\"\n"));
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
fn systemd_unit_quotes_and_escapes_paths() {
    let cases = [
        (
            "spaces",
            [
                "/home/u/my apps/swing",
                "/home/u/my dir/swing.toml",
                "/home/u/my dir",
            ],
            [
                "\"/home/u/my apps/swing\"",
                "\"/home/u/my dir/swing.toml\"",
                "WorkingDirectory=/home/u/my dir\n",
                "ReadWritePaths=\"/home/u/my dir\"\n",
            ],
        ),
        (
            "specifiers and variables",
            [
                "/home/u/$HOME/swing",
                "/home/u/100%/swing.toml",
                "/home/u/100%",
            ],
            [
                "\"/home/u/$$HOME/swing\"",
                "\"/home/u/100%%/swing.toml\"",
                "WorkingDirectory=/home/u/100%%\n",
                "ReadWritePaths=\"/home/u/100%%\"\n",
            ],
        ),
    ];
    for (case, [exe, config, workdir], parts) in cases {
        let unit = systemd_unit(
            Path::new(exe),
            Path::new(config),
            Path::new(workdir),
            &SYSTEM,
        )
        .unwrap();
        for part in parts {
            assert!(unit.contains(part), "{case}: {part:?} in {unit}");
        }
    }
    assert_eq!(quote_systemd_arg("a\"b\\c"), "\"a\\\"b\\\\c\"");
    assert_eq!(quote_systemd_path("a\"b\\c$"), "\"a\\\"b\\\\c$\"");
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
fn launchd_plist_matches_the_expected_text() {
    let plist = launchd_plist(
        Path::new("/usr/local/bin/swing"),
        Path::new("/Users/u/swing.toml"),
        Path::new("/Users/u"),
        Path::new("/Users/u/Library/Logs/swing.log"),
    )
    .unwrap();
    assert_eq!(plist, LAUNCHD_PLIST);
}

const LAUNCHD_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>jp.ne.ama.swing</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/swing</string>
        <string>up</string>
        <string>--config</string>
        <string>/Users/u/swing.toml</string>
    </array>
    <key>WorkingDirectory</key>
    <string>/Users/u</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ExitTimeOut</key>
    <integer>90</integer>
    <key>StandardOutPath</key>
    <string>/Users/u/Library/Logs/swing.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/u/Library/Logs/swing.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    </dict>
    <key>AssociatedBundleIdentifiers</key>
    <array>
        <string>jp.ne.ama.swing</string>
    </array>
</dict>
</plist>
"#;

#[test]
fn xml_templates_escape_their_values() {
    let plist = launchd_plist(
        Path::new("/usr/local/bin/swing"),
        Path::new("/Users/u/A&B<C>.toml"),
        Path::new("/Users/u"),
        Path::new("/Users/u/Library/Logs/swing.log"),
    )
    .unwrap();
    assert!(plist.contains("A&amp;B&lt;C&gt;.toml"), "{plist}");
    assert!(!plist.contains("A&B<C>.toml"), "{plist}");

    let xml = schtasks_xml(
        Path::new(r"C:\swing.exe"),
        Path::new(r"C:\a&b<c>.toml"),
        Path::new(r"C:\dir"),
        Path::new(r"C:\log.txt"),
        "user<1>&",
    )
    .unwrap();
    assert!(xml.contains("a&amp;b&lt;c&gt;.toml"), "{xml}");
    assert!(xml.contains("<UserId>user&lt;1&gt;&amp;</UserId>"), "{xml}");
    assert!(!xml.contains("a&b<c>.toml"), "{xml}");
}

#[test]
fn schtasks_xml_matches_the_expected_text() {
    let xml = schtasks_xml(
        Path::new(r"C:\Program Files\swing\swing.exe"),
        Path::new(r"C:\Users\u\swing.toml"),
        Path::new(r"C:\Users\u"),
        Path::new(r"C:\Users\u\swing.log"),
        "DOMAIN\\user",
    )
    .unwrap();
    assert_eq!(xml, SCHTASKS_XML);
}

const SCHTASKS_XML: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>SWING mirror agent</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>DOMAIN\user</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>DOMAIN\user</UserId>
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
      <Arguments>--headless &quot;C:\Program Files\swing\swing.exe&quot; up --config &quot;C:\Users\u\swing.toml&quot; --log-file &quot;C:\Users\u\swing.log&quot; --exit-with-parent</Arguments>
      <WorkingDirectory>C:\Users\u</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#;
