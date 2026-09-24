fn main() {
    println!("cargo::rerun-if-changed=assets/swing.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/swing.ico")
            .compile()
            .expect("embedding the Windows resources");
    }
}
