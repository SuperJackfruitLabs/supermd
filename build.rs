fn main() {
    // Embed the Windows icon + metadata; no-op elsewhere.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/windows/supermd.ico");
        res.set("ProductName", "SuperMD");
        res.set("FileDescription", "SuperMD — Markdown editor");
        if let Err(e) = res.compile() {
            println!("cargo:warning=winresource failed: {e}");
        }
    }
}
