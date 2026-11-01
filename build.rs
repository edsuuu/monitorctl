// Windows: embed the app icon (drawn by src/logo.rs) and the "monitorctl" name into the .exe,
// so Explorer, shortcuts, the taskbar and Task Manager show them.
include!("src/logo.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/logo.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let icon = output.join("monitorctl.ico");
    std::fs::write(&icon, ico(&[16, 24, 32, 48, 64, 128, 256])).expect("gravar monitorctl.ico");
    let version = env!("CARGO_PKG_VERSION").replace('.', ",");
    let resource = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {version},0
PRODUCTVERSION {version},0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "monitorctl"
      VALUE "ProductName", "monitorctl"
      VALUE "ProductVersion", "{package_version}"
      VALUE "OriginalFilename", "monitorctl.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "\\\\"),
        package_version = env!("CARGO_PKG_VERSION"),
    );
    let script = output.join("monitorctl.rc");
    std::fs::write(&script, resource).expect("gravar monitorctl.rc");
    embed_resource::compile(&script, embed_resource::NONE).manifest_optional().expect("compilar recursos do Windows");
}
