fn main() {
    println!("cargo::rerun-if-changed=assets/swing-tray.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/swing-tray.ico")
            .compile()
            .expect("embedding the Windows resources");
    }
}
