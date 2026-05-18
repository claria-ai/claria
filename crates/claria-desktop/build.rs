use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    ensure_frontend_built();
    tauri_build::build();
}

// `tauri.conf.json`'s `beforeBuildCommand` only runs under the Tauri CLI
// (`cargo tauri dev|build`). Plain `cargo run -p claria-desktop` does not,
// which means a stale `dist/` happily ships to the WebView and the bundled
// JS can be talking to a different `@tauri-apps/api` version than the Rust
// `tauri` crate the binary just linked against. The IPC drift terminates
// the web content process on launch with no error in the Rust logs.
//
// To keep `cargo run` working out of the box, rebuild the frontend whenever
// `package-lock.json` is newer than `dist/index.html`, or `dist/` is missing.
// CI sets `CI=true` and runs its own explicit `npm` steps, so skip there.
fn ensure_frontend_built() {
    let frontend_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../claria-desktop-frontend")
        .canonicalize()
        .expect("claria-desktop-frontend directory should exist next to the workspace");

    let lockfile = frontend_dir.join("package-lock.json");
    let dist_index = frontend_dir.join("dist").join("index.html");

    println!("cargo:rerun-if-changed={}", lockfile.display());
    println!("cargo:rerun-if-changed={}", dist_index.display());
    println!("cargo:rerun-if-env-changed=CI");
    println!("cargo:rerun-if-env-changed=CLARIA_SKIP_FRONTEND_BUILD");
    println!("cargo:rerun-if-env-changed=TAURI_ENV_PLATFORM");

    if std::env::var_os("CI").is_some() || std::env::var_os("CLARIA_SKIP_FRONTEND_BUILD").is_some()
    {
        return;
    }

    if !needs_rebuild(&lockfile, &dist_index) {
        return;
    }

    println!(
        "cargo:warning=claria-desktop-frontend/dist is stale or missing; running npm install + npm run build"
    );

    // The Tauri CLI sets TAURI_ENV_PLATFORM when it invokes cargo. Plain
    // `cargo build`/`cargo run` doesn't — and that's the path that just
    // forced an expensive frontend rebuild. Nudge the dev toward the CLI,
    // where `beforeDevCommand` gives them Vite hot-reload for free.
    if std::env::var_os("TAURI_ENV_PLATFORM").is_none() {
        println!(
            "cargo:warning=tip: `cargo tauri dev` runs the Vite dev server with hot-reload, so you don't pay this rebuild cost on every iteration"
        );
    }

    run_npm(&frontend_dir, &["install"]);
    run_npm(&frontend_dir, &["run", "build"]);
}

fn needs_rebuild(lockfile: &Path, dist_index: &Path) -> bool {
    let dist_mtime = match dist_index.metadata().and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let lock_mtime = match lockfile.metadata().and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return false,
    };
    lock_mtime > dist_mtime
}

fn run_npm(dir: &Path, args: &[&str]) {
    let program = if cfg!(target_os = "windows") {
        "npm.cmd"
    } else {
        "npm"
    };
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `npm {}`: {e}", args.join(" ")));
    if !status.success() {
        panic!("`npm {}` exited with {status}", args.join(" "));
    }
}
