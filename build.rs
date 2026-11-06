use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    for name in ["assets/icon.rc", "assets/quby-seemdissolver.ico"] {
        println!("cargo:rerun-if-changed={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK directory"))
        .join("Windows Kits/10/bin");
    let mut compilers: Vec<_> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("x64/rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    compilers.sort();
    let compiler = compilers.pop().unwrap_or_else(|| PathBuf::from("rc.exe"));
    let resource = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("icon.res");
    let status = Command::new(compiler)
        .args(["/nologo", "/fo"])
        .arg(&resource)
        .arg("assets/icon.rc")
        .status()
        .expect("Run the Windows SDK resource compiler");
    assert!(status.success(), "Icon resource compilation failed");
    println!("cargo:rustc-link-arg={}", resource.display());
}
