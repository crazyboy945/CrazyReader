use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app_icon.o");
    let status = Command::new("windres")
        .arg(manifest.join("assets/app.rc"))
        .args(["-O", "coff", "-o"])
        .arg(&out)
        .arg(format!("-I{}", manifest.join("assets").display()))
        .status()
        .expect("failed to run windres (install binutils / MinGW-w64)");
    assert!(status.success(), "windres failed to compile the icon resource");

    println!("cargo:rustc-link-arg={}", out.display());
}
