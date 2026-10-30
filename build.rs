//! Attaches the KernelSU icon to the executable when `KSUPATCH_ICON` is set.
//!
//! Windows takes an executable's icon from its resource section, so a small `.res` has to be
//! compiled and handed to the linker. `llvm-rc` (or `windres`) comes with the mingw-w64
//! toolchain, which avoids pulling in a build-dependency just for this. The icon is opt-in
//! because the resource costs about 6 KiB in the packed executable; without the variable the
//! build simply produces an executable with the default icon.

use std::path::PathBuf;
use std::process::Command;

const ICON: &str = "assets/icon.ico";

fn main() {
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed=assets/ksupatch.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=KSUPATCH_ICON");

    // Explicitly declare asInvoker. Without a manifest Windows applies Installer
    // Detection to 32-bit programs whose names contain words such as "patch", so
    // an x86 build named ksupatch.exe would demand elevation before it even runs.
    // The MSVC linker can embed a manifest directly; the gnullvm/zig path does
    // not need it because Installer Detection only applies to 32-bit images.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = PathBuf::from("assets/ksupatch.manifest");
        if manifest.exists() {
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
                manifest.canonicalize().unwrap_or(manifest).display()
            );
        }
    }

    // Smallest default build: only embed the icon when explicitly requested.
    if std::env::var_os("KSUPATCH_ICON").is_none() {
        return;
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = PathBuf::from(ICON);
    if !icon.exists() {
        println!("cargo:warning={ICON} is missing; building without a custom icon");
        return;
    }
    let Ok(icon) = icon.canonicalize() else {
        println!("cargo:warning=cannot resolve {ICON}; building without a custom icon");
        return;
    };

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc_file = out.join("icon.rc");
    let res_file = out.join("icon.res");
    let script = format!("1 ICON \"{}\"\n", icon.display());
    if std::fs::write(&rc_file, script).is_err() {
        println!("cargo:warning=cannot write the resource script; building without a custom icon");
        return;
    }

    // `llvm-rc` first (mingw-w64 ships it), then windres as a fallback.
    let attempts: [(&str, Vec<String>); 2] = [
        (
            "llvm-rc",
            vec![
                "/fo".to_string(),
                res_file.display().to_string(),
                rc_file.display().to_string(),
            ],
        ),
        (
            "windres",
            vec![
                "-I".to_string(),
                icon.parent().unwrap_or(&icon).display().to_string(),
                "-O".to_string(),
                "coff".to_string(),
                "-o".to_string(),
                res_file.display().to_string(),
                rc_file.display().to_string(),
            ],
        ),
    ];

    for (tool, args) in attempts {
        match Command::new(tool).args(&args).status() {
            Ok(s) if s.success() => {
                println!("cargo:rustc-link-arg={}", res_file.display());
                return;
            }
            Ok(s) => println!("cargo:warning={tool} failed ({s}); trying the next tool"),
            Err(_) => {}
        }
    }
    println!("cargo:warning=no resource compiler found; building without a custom icon");
}
