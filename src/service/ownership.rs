use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Service,
    Tray,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub part: Part,
    pub what: String,
    pub exe: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    NotRegistered,
    Inside,
    Outside,
}

impl Placement {
    pub fn exit_code(self) -> i32 {
        match self {
            Placement::Inside => 0,
            Placement::NotRegistered => 3,
            Placement::Outside => 4,
        }
    }
}

pub(super) fn summarize(inside: &[bool]) -> Placement {
    if inside.is_empty() {
        Placement::NotRegistered
    } else if inside.iter().all(|&i| i) {
        Placement::Inside
    } else {
        Placement::Outside
    }
}

fn components(path: &str, windows: bool) -> Option<Vec<String>> {
    let (path, absolute) = if windows {
        let path = super::templates::strip_verbatim(&path.replace('/', "\\"));
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\';
        let absolute = drive || path.starts_with(r"\\");
        (path.to_lowercase(), absolute)
    } else {
        (path.to_owned(), path.starts_with('/'))
    };
    if !absolute {
        return None;
    }
    let sep = if windows { '\\' } else { '/' };
    let mut out: Vec<String> = Vec::new();
    for part in path.split(sep) {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other.to_owned()),
        }
    }
    Some(out)
}

pub(super) fn path_is_under(path: &str, dir: &str, windows: bool) -> bool {
    match (components(path, windows), components(dir, windows)) {
        (Some(path), Some(dir)) => path.len() > dir.len() && path.starts_with(&dir),
        _ => false,
    }
}

pub(super) fn exe_points_into(exe: &str, dir: &Path) -> bool {
    let windows = cfg!(windows);
    let exes = [
        Some(exe.to_owned()),
        std::fs::canonicalize(exe)
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned)),
    ];
    let dirs = [
        dir.to_str().map(str::to_owned),
        std::fs::canonicalize(dir)
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned)),
    ];
    exes.iter().flatten().any(|exe| {
        dirs.iter()
            .flatten()
            .any(|dir| path_is_under(exe, dir, windows))
    })
}

#[cfg(any(windows, target_os = "macos", test))]
fn xml_unescape(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let end = rest[start..].find(';')? + start;
        let entity = &rest[start + 1..end];
        let ch = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    entity.strip_prefix('#')?.parse().ok()?
                };
                char::from_u32(code)?
            }
        };
        out.push(ch);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(any(windows, test))]
fn element_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

#[cfg(any(windows, test))]
fn windows_args(command_line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    for c in command_line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            c => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(current);
    }
    args
}

#[cfg(any(windows, test))]
pub(super) fn exe_from_task_xml(xml: &str) -> Option<String> {
    let exec = element_text(xml, "Exec")?;
    let command = xml_unescape(element_text(exec, "Command")?.trim())?;
    let command = command.trim_matches('"');
    let is_conhost = command
        .rsplit(['\\', '/'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("conhost.exe"));
    if !is_conhost {
        return Some(command.to_owned()).filter(|c| !c.is_empty());
    }
    let arguments = xml_unescape(element_text(exec, "Arguments")?)?;
    windows_args(&arguments)
        .into_iter()
        .find(|arg| !arg.starts_with('-'))
}

#[cfg(any(windows, test))]
pub(super) fn exe_from_run_command(command_line: &str) -> Option<String> {
    windows_args(command_line).into_iter().next()
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn exe_from_systemd_unit(unit: &str) -> Option<String> {
    let value = unit
        .lines()
        .map(str::trim_start)
        .find_map(|line| line.strip_prefix("ExecStart="))?;
    let value = value.trim_start_matches(['-', '@', ':', '+', '!']);
    let mut raw = String::new();
    if let Some(quoted) = value.strip_prefix('"') {
        let mut chars = quoted.chars();
        loop {
            match chars.next()? {
                '"' => break,
                '\\' => match chars.next()? {
                    c @ ('\\' | '"') => raw.push(c),
                    c => {
                        raw.push('\\');
                        raw.push(c);
                    }
                },
                c => raw.push(c),
            }
        }
    } else {
        raw = value.split_whitespace().next()?.to_owned();
    }
    Some(raw.replace("$$", "$").replace("%%", "%"))
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn exe_from_launchd_plist(plist: &str) -> Option<String> {
    let string_after = |key: &str| -> Option<String> {
        let after = &plist[plist.find(key)? + key.len()..];
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")? + start;
        xml_unescape(&after[start..end])
    };
    if plist.contains("<key>Program</key>") {
        return string_after("<key>Program</key>");
    }
    let key = "<key>ProgramArguments</key>";
    let after = &plist[plist.find(key)? + key.len()..];
    let array_end = after.find("</array>")?;
    let array = &after[..array_end];
    let start = array.find("<string>")? + "<string>".len();
    let end = array[start..].find("</string>")? + start;
    xml_unescape(&array[start..end])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::templates::{
        SystemdScope, launchd_plist, launchd_tray_plist, schtasks_xml, systemd_unit,
        tray_run_command,
    };
    use super::*;

    #[test]
    fn windows_paths_match_by_component_ignoring_case_and_verbatim_prefixes() {
        let app = r"C:\Users\a\AppData\Local\Programs\SWING";
        for exe in [
            r"C:\Users\a\AppData\Local\Programs\SWING\swing.exe",
            r"c:\users\A\appdata\local\programs\swing\SWING.EXE",
            r"\\?\C:\Users\a\AppData\Local\Programs\SWING\swing.exe",
            r"C:/Users/a/AppData/Local/Programs/SWING/swing.exe",
            r"C:\Users\a\AppData\Local\Programs\SWING\.\bin\..\swing.exe",
        ] {
            assert!(path_is_under(exe, app, true), "{exe}");
        }
        assert!(path_is_under(
            r"C:\Users\a\AppData\Local\Programs\SWING\swing.exe",
            &format!(r"\\?\{app}\"),
            true
        ));
        for exe in [
            r"C:\Users\a\AppData\Local\Programs\SWING-old\swing.exe",
            r"C:\Users\a\Downloads\swing\swing.exe",
            r"C:\Users\a\AppData\Local\Programs\SWING\..\swing.exe",
            r"D:\Users\a\AppData\Local\Programs\SWING\swing.exe",
            r"swing.exe",
            r"Programs\SWING\swing.exe",
            app,
        ] {
            assert!(!path_is_under(exe, app, true), "{exe}");
        }
        assert!(path_is_under(
            r"\\?\UNC\server\share\SWING\swing.exe",
            r"\\server\share\swing",
            true
        ));
    }

    #[test]
    fn unix_paths_match_by_component_and_case() {
        let lib = "/home/u/.local/lib/swing";
        assert!(path_is_under("/home/u/.local/lib/swing/swing", lib, false));
        assert!(path_is_under(
            "/home/u/.local/lib/swing/swing",
            "/home/u/.local/lib/swing/",
            false
        ));
        assert!(!path_is_under(
            "/home/u/.local/lib/swing-old/swing",
            lib,
            false
        ));
        assert!(!path_is_under("/home/u/.local/lib/Swing/swing", lib, false));
        assert!(!path_is_under("/home/u/swing/swing", lib, false));
        assert!(!path_is_under("swing", lib, false));
        assert!(!path_is_under(lib, lib, false));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dirs_and_executables_match_their_targets() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("swing"), b"").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let exe = real.join("swing");
        assert!(exe_points_into(exe.to_str().unwrap(), &link));
        let via_link = link.join("swing");
        assert!(exe_points_into(via_link.to_str().unwrap(), &real));
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(!exe_points_into(exe.to_str().unwrap(), &other));
    }

    #[test]
    fn placement_needs_every_registered_part_inside() {
        assert_eq!(summarize(&[]), Placement::NotRegistered);
        assert_eq!(summarize(&[true, true]), Placement::Inside);
        assert_eq!(summarize(&[true, false]), Placement::Outside);
        assert_eq!(summarize(&[false]), Placement::Outside);
        assert_eq!(Placement::Inside.exit_code(), 0);
        assert_eq!(Placement::NotRegistered.exit_code(), 3);
        assert_eq!(Placement::Outside.exit_code(), 4);
    }

    #[test]
    fn task_xml_round_trips_the_executable() {
        for exe in [
            r"C:\Program Files\SWING\swing.exe",
            r"C:\Users\a & b\it's <here>\swing.exe",
        ] {
            let xml = schtasks_xml(
                Path::new(exe),
                Path::new(r"C:\Users\u\swing.toml"),
                Path::new(r"C:\Users\u"),
                Path::new(r"C:\Users\u\swing.log"),
                "u",
            )
            .unwrap();
            assert_eq!(exe_from_task_xml(&xml).as_deref(), Some(exe));
            let reformatted = xml.replace("&quot;", "\"");
            assert_eq!(exe_from_task_xml(&reformatted).as_deref(), Some(exe));
        }
    }

    #[test]
    fn task_xml_without_conhost_uses_the_command() {
        let xml = "<Actions><Exec><Command>\"C:\\x\\swing.exe\"</Command><Arguments>up</Arguments></Exec></Actions>";
        assert_eq!(exe_from_task_xml(xml).as_deref(), Some(r"C:\x\swing.exe"));
        assert_eq!(exe_from_task_xml("<Task></Task>"), None);
    }

    #[test]
    fn run_command_round_trips_the_tray_executable() {
        let cmd = tray_run_command(
            Path::new(r"\\?\C:\Program Files\SWING\swing-tray.exe"),
            Path::new(r"C:\Users\u\swing.toml"),
        )
        .unwrap();
        assert_eq!(
            exe_from_run_command(&cmd).as_deref(),
            Some(r"C:\Program Files\SWING\swing-tray.exe")
        );
        assert_eq!(
            exe_from_run_command(r"C:\x\swing-tray.exe --config a").as_deref(),
            Some(r"C:\x\swing-tray.exe")
        );
        assert_eq!(exe_from_run_command("  "), None);
    }

    #[test]
    fn systemd_unit_round_trips_the_executable() {
        let system = SystemdScope::System {
            user: "swing",
            writable: &[],
        };
        for exe in [
            "/home/u/.local/lib/swing/swing",
            "/home/u/my apps/100% \"real\" $HOME\\swing",
        ] {
            for scope in [&SystemdScope::User, &system] {
                let unit = systemd_unit(
                    Path::new(exe),
                    Path::new("/home/u/swing.toml"),
                    Path::new("/home/u"),
                    scope,
                )
                .unwrap();
                assert_eq!(exe_from_systemd_unit(&unit).as_deref(), Some(exe));
            }
        }
        assert_eq!(
            exe_from_systemd_unit("[Service]\nExecStart=-/usr/bin/swing up\n").as_deref(),
            Some("/usr/bin/swing")
        );
        assert_eq!(exe_from_systemd_unit("[Service]\n"), None);
        assert_eq!(exe_from_systemd_unit("ExecStart=\"/unterminated"), None);
    }

    #[test]
    fn launchd_plists_round_trip_the_executable() {
        let exe = PathBuf::from("/opt/homebrew/opt/swing/bin/a & <b>");
        let plist = launchd_plist(
            &exe,
            Path::new("/Users/u/swing.toml"),
            Path::new("/Users/u"),
            Path::new("/Users/u/Library/Logs/swing.log"),
        )
        .unwrap();
        assert_eq!(exe_from_launchd_plist(&plist).as_deref(), exe.to_str());
        let tray = "/Applications/SWING.app/Contents/MacOS/swing-tray";
        let plist = launchd_tray_plist(
            Path::new(tray),
            Path::new("/Users/u/swing.toml"),
            Path::new("/Users/u"),
        )
        .unwrap();
        assert_eq!(exe_from_launchd_plist(&plist).as_deref(), Some(tray));
        assert_eq!(
            exe_from_launchd_plist(
                "<dict><key>Program</key><string>/x/swing</string><key>ProgramArguments</key><array><string>swing</string></array></dict>"
            )
            .as_deref(),
            Some("/x/swing")
        );
        assert_eq!(exe_from_launchd_plist("bplist00"), None);
    }

    #[test]
    fn xml_unescape_handles_named_and_numeric_entities() {
        assert_eq!(
            xml_unescape("a&amp;b&lt;&gt;&quot;&apos;&#65;&#x42;").as_deref(),
            Some("a&b<>\"'AB")
        );
        assert_eq!(xml_unescape("&amp;lt;").as_deref(), Some("&lt;"));
        assert_eq!(xml_unescape("a&b"), None);
    }
}
