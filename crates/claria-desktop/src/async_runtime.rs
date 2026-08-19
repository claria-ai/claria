//! The async runtime every Tauri command runs on.
//!
//! Tauri creates one lazily on first use if nobody hands it one. That default
//! gives each worker the 2 MiB stack a platform thread gets, which is not
//! enough for one AWS SDK request in an unoptimized build — see
//! [`WORKER_STACK_BYTES`] for why. This module builds the runtime instead, so
//! the depth is the app's decision rather than a default nobody chose.

use eyre::Result;

/// Stack for each async worker thread.
///
/// One AWS SDK call does not fit in 2 MiB with room to spare. A request
/// descends through the smithy orchestrator, the hyper connection pool, the
/// TLS connector and a rustls handshake, and each of those layers is a future
/// wrapping the next rather than calling it — so the stack at the bottom of a
/// handshake is roughly a hundred frames of nested `poll`, each holding its
/// inner future by value. Certificate-chain verification then parses DER at
/// the deepest point of all.
///
/// A release build folds most of that away: `opt-level = "z"` with LTO inlines
/// the combinators into each other. An unoptimized build cannot, and overflows
/// partway through verifying a certificate. That is a crash rather than an
/// error — the overflow lands on the guard page and the process aborts, losing
/// every in-flight draft, unwritten audit event and parked run with it.
///
/// Eight mebibytes is roughly four times the deepest stack observed. It costs
/// address space rather than memory: pages are committed as they are touched,
/// so an idle worker still occupies the few kilobytes it always did.
pub const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

/// Build the async runtime Tauri will use and install it as the global one.
///
/// Must run before anything spawns: [`tauri::async_runtime::set`] panics once
/// the global runtime exists, and the first `spawn` or `block_on` anywhere
/// creates it.
///
/// The runtime is leaked deliberately. Tauri keeps its own in a `OnceLock`
/// that is never dropped either, and dropping one here would block process
/// exit on whatever task happened to still be running.
pub fn install() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(WORKER_STACK_BYTES)
        .build()?;
    tauri::async_runtime::set(Box::leak(Box::new(runtime)).handle().clone());
    Ok(())
}
