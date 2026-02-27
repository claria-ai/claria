use std::path::Path;
use std::process::Command;

fn main() {
    let frontend_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../claria-desktop-frontend");

    println!("cargo:rerun-if-changed=../../claria-desktop-frontend/src");
    println!("cargo:rerun-if-changed=../../claria-desktop-frontend/index.html");
    println!("cargo:rerun-if-changed=../../claria-desktop-frontend/package.json");
    println!("cargo:rerun-if-changed=../../claria-desktop-frontend/vite.config.ts");
    println!("cargo:rerun-if-changed=../../claria-desktop-frontend/tsconfig.json");

    // npm install (skip if node_modules exists and package.json hasn't changed)
    let status = Command::new("npm")
        .arg("install")
        .current_dir(&frontend_dir)
        .status()
        .expect("failed to run `npm install` — is Node.js installed?");
    assert!(status.success(), "npm install failed");

    // npm run build
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(&frontend_dir)
        .status()
        .expect("failed to run `npm run build`");
    assert!(status.success(), "npm run build failed");

    tauri_build::build();
}
