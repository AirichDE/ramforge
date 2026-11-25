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
        // Partitioning and formatting a stick needs administrator rights,
        // like every disk tool: Windows asks once at program start.
        res.set_manifest(
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges>
      <requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>
    </requestedPrivileges></security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings><dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware></windowsSettings>
  </application>
</assembly>"#,
        );
        if let Err(e) = res.compile() {
            // A missing resource compiler must not break the build; only the icon is lost.
            println!("cargo:warning=Icon nicht eingebettet: {e}");
        }
    }
}
