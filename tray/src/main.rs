#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "macos"))]
mod app;
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
mod status;
#[cfg(any(windows, target_os = "macos"))]
mod worker;

#[cfg(any(windows, target_os = "macos"))]
fn config_arg() -> Result<Option<std::path::PathBuf>, &'static str> {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next(), args.next()) {
        (None, _, _) => Ok(None),
        (Some(flag), Some(path), None) if flag == "--config" => Ok(Some(path.into())),
        _ => Err("usage: swing-tray [--config <path>]"),
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn single_instance(config_path: Option<&std::path::Path>) -> Option<std::fs::File> {
    let config = swing::config::Config::load(config_path).ok()?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.agent.state_dir.join("swing-tray.lock"))
        .ok()?;
    match file.try_lock() {
        Ok(()) => Some(file),
        Err(std::fs::TryLockError::WouldBlock) => std::process::exit(0),
        Err(std::fs::TryLockError::Error(_)) => None,
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    let config_path = config_arg().unwrap_or_else(|usage| {
        eprintln!("{usage}");
        std::process::exit(2);
    });
    let lock = single_instance(config_path.as_deref());
    app::run(config_path, lock)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("swing-tray supports only Windows and macOS");
    std::process::exit(1);
}
