// Embeds the RAMForge icon and version info into the Windows EXE.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/logo/out/ramforge.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/logo/out/ramforge.ico");
        res.set("ProductName", "RAMForge");
        res.set("FileDescription", env!("CARGO_PKG_DESCRIPTION"));
        res.set(
            "LegalCopyright",
            "© 2026 Daniel Airich und RAMForge-Mitwirkende · GPL-3.0-or-later",
        );
        if let Err(e) = res.compile() {
            // A missing resource compiler must not break the build; only the icon is lost.
            println!("cargo:warning=Icon nicht eingebettet: {e}");
        }
    }
}
