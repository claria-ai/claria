//! Workers have to be deep enough for one AWS SDK request.
//!
//! The failure this guards against is not a test failure but an abort: a task
//! that outruns its stack lands on the guard page and takes the process down.
//! So the test spends a stack depth no 2 MiB worker could survive and asserts
//! it came back — on the unconfigured runtime this binary aborts instead.
//!
//! One test per binary on purpose. The global runtime lives in a `OnceLock`
//! and [`claria_desktop::async_runtime::install`] may only be called once in
//! a process.

use claria_desktop::async_runtime::{WORKER_STACK_BYTES, install};

/// Stack spent per frame of [`spend_stack`].
const FRAME_BYTES: usize = 64 * 1024;

/// Total stack the task spends: comfortably past 2 MiB, comfortably inside
/// the configured depth, so the test distinguishes the two rather than
/// probing the exact limit of either.
const SPEND_BYTES: usize = 4 * 1024 * 1024;

/// The spend has to exceed a default 2 MiB thread stack, or the test proves
/// nothing, and stay inside the configured depth, or it is measuring the
/// guard page rather than the setting. Checked at compile time: a spend that
/// drifts out of that band should fail the build, not one test run.
const _: () = assert!(SPEND_BYTES > 2 * 1024 * 1024);
const _: () = assert!(SPEND_BYTES < WORKER_STACK_BYTES);

/// Occupy `FRAME_BYTES` of stack per frame, recursing `frames` deep.
///
/// Every frame touches both ends of its buffer so the pages are really
/// committed, and the sum is returned so nothing here can be optimized away.
#[inline(never)]
fn spend_stack(frames: usize) -> u64 {
    let mut frame = [0u8; FRAME_BYTES];
    let last = frame.len() - 1;
    frame[0] = frames as u8;
    frame[last] = frames as u8;
    let touched =
        u64::from(std::hint::black_box(frame[0])) + u64::from(std::hint::black_box(frame[last]));
    if frames == 0 {
        touched
    } else {
        touched + spend_stack(frames - 1)
    }
}

#[test]
fn a_worker_survives_more_stack_than_the_default_thread_has() {
    install().expect("install the async runtime");

    // Spawned, not `block_on`: `block_on` would run the task on this test's
    // own thread and measure the harness's stack instead of a worker's.
    let task = tauri::async_runtime::spawn(async { spend_stack(SPEND_BYTES / FRAME_BYTES) });
    let spent = tauri::async_runtime::block_on(task).expect("the task ran to completion");

    assert!(spent > 0, "the recursion was optimized away: {spent}");
}
