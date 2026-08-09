use std::{env, path::PathBuf, process::Command};

fn main() {
    let is_macos = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let metal_enabled = env::var_os("CARGO_FEATURE_METAL").is_some();
    if !is_macos || !metal_enabled {
        return;
    }

    // transcribe.cpp's Metal backend uses Objective-C availability checks.
    // Cargo asks the C linker to omit its default libraries, so the Apple
    // compiler runtime that implements those checks must be linked explicitly.
    let compiler = cc::Build::new().get_compiler();
    let output = Command::new(compiler.path())
        .args(["-print-libgcc-file-name", "--rtlib=compiler-rt"])
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to locate the Apple compiler runtime with {}: {error}",
                compiler.path().display()
            )
        });
    if !output.status.success() {
        panic!(
            "{} could not locate the Apple compiler runtime: {}",
            compiler.path().display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let runtime = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("Apple compiler runtime path was not UTF-8: {error}"));
    let runtime = PathBuf::from(runtime.trim());
    if runtime.file_name().and_then(|name| name.to_str()) != Some("libclang_rt.osx.a")
        || !runtime.is_file()
    {
        panic!(
            "Apple compiler returned an invalid compiler runtime path: {}",
            runtime.display()
        );
    }
    let library_dir = runtime
        .parent()
        .unwrap_or_else(|| panic!("Apple compiler runtime has no parent directory"));

    println!("cargo:rustc-link-search=native={}", library_dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
