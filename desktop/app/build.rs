use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = root
        .join("../assets/CursorCue.ico")
        .canonicalize()
        .expect("CursorCue icon");
    println!("cargo:rerun-if-changed={}", icon.display());
    let manifest = root
        .join("app.manifest")
        .canonicalize()
        .expect("CursorCue manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let output = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc = output.join("CursorCue.rc");
    let resource = output.join("CursorCue.res");
    fs::write(&rc, format!("1 ICON \"{}\"\n1 VERSIONINFO\n FILEVERSION 0,1,3,0\n PRODUCTVERSION 0,1,3,0\n FILEOS 0x40004\n FILETYPE 1\nBEGIN\n BLOCK \"StringFileInfo\"\n BEGIN\n  BLOCK \"040904B0\"\n  BEGIN\n   VALUE \"FileDescription\", \"CursorCue\"\n   VALUE \"ProductName\", \"CursorCue\"\n   VALUE \"FileVersion\", \"0.1.3\"\n   VALUE \"ProductVersion\", \"0.1.3\"\n  END\n END\n BLOCK \"VarFileInfo\"\n BEGIN\n  VALUE \"Translation\", 0x409, 1200\n END\nEND\n", icon.to_string_lossy().replace('\\', "/"))).unwrap();
    let manifest_resource = format!(
        "1 24 \"{}\"\n",
        manifest.to_string_lossy().replace('\\', "/")
    );
    let mut rc_content = fs::read_to_string(&rc).unwrap();
    rc_content.push_str(&manifest_resource);
    fs::write(&rc, rc_content).unwrap();
    let sdk = PathBuf::from(env::var("ProgramFiles(x86)").expect("Windows SDK root"))
        .join("Windows Kits/10/bin");
    let mut versions: Vec<_> = fs::read_dir(sdk)
        .expect("Windows SDK")
        .filter_map(|item| item.ok().map(|entry| entry.path().join("x64/rc.exe")))
        .filter(|path| path.is_file())
        .collect();
    versions.sort();
    let compiler = versions.last().expect("Windows SDK resource compiler");
    let status = Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(&rc)
        .status()
        .expect("compile icon resource");
    assert!(status.success(), "CursorCue resource compilation failed");
    println!("cargo:rustc-link-arg={}", resource.display());
}
