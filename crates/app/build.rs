fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");

    // Check the target OS (not the host): only embed the icon for Windows targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    // Default icon id (1), which GPUI's Windows backend loads for the window/taskbar icon.
    res.set_icon("icons/icon.ico");
    res.set("ProductName", "GTNH Updater");
    res.set("FileDescription", "GTNH Updater");
    res.compile().expect("failed to embed Windows resources");
}
