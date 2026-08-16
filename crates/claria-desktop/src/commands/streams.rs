//! The registry of streams a reader can stop, and the one command that stops
//! them.
//!
//! Every surface that drives Bedrock through a long-running command — chat,
//! the writer's targeted turns, whole-report drafting runs — mints a stream
//! id before it invokes, registers a [`StopSignal`] under it for the life of
//! the call, and hands the signal down to the loop that watches it. One
//! registry and one command mean a Stop button behaves identically wherever
//! it appears.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use tauri::State;

use claria_bedrock::converse::StopSignal;

use super::{CommandError, parse_uuid};
use crate::state::DesktopState;

/// Live stop signals, keyed by the stream id the frontend minted.
pub(crate) type StopRegistry = Mutex<HashMap<uuid::Uuid, StopSignal>>;

/// The registry is only ever held across map operations, never across an
/// await, so a poisoned lock means a panic elsewhere rather than torn state —
/// recover the map instead of taking the process down with it.
pub(crate) fn lock_stops(
    registry: &StopRegistry,
) -> MutexGuard<'_, HashMap<uuid::Uuid, StopSignal>> {
    registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A stream's stop signal, registered for as long as that stream runs.
///
/// The registration is an RAII guard rather than a pair of calls because
/// every `?` in a command body is an exit path; a leaked entry would keep a
/// finished stream's id addressable and grow the map for the life of the
/// process.
pub(crate) struct StopRegistration {
    registry: Arc<StopRegistry>,
    stream_id: uuid::Uuid,
    pub(crate) signal: StopSignal,
}

impl StopRegistration {
    /// Register a fresh signal for `stream_id`. A repeated id replaces the
    /// previous entry, which can only happen if the frontend reused one.
    pub(crate) fn open(state: &DesktopState, stream_id: &str) -> Result<Self, CommandError> {
        let stream_id = parse_uuid(stream_id)?;
        let signal = StopSignal::new();
        let registry = state.stream_stops.clone();
        lock_stops(&registry).insert(stream_id, signal.clone());
        Ok(Self {
            registry,
            stream_id,
            signal,
        })
    }
}

impl Drop for StopRegistration {
    fn drop(&mut self) {
        let mut registry = lock_stops(&self.registry);
        // Only ever retire this stream's own signal — a reused id would
        // otherwise let a finishing stream deregister a running one.
        if registry
            .get(&self.stream_id)
            .is_some_and(|current| current.is_same(&self.signal))
        {
            registry.remove(&self.stream_id);
        }
    }
}

/// End an in-flight streamed turn early: a chat reply, a writer turn, or a
/// whole-report drafting run.
///
/// What stopping costs is the caller's business, not this command's — chat
/// keeps the text that arrived, and a drafting run keeps every section it had
/// already saved.
///
/// A `stream_id` with no live stream behind it is not an error: the work may
/// have completed between the click and this call.
#[tauri::command]
#[specta::specta]
pub fn stop_stream(state: State<'_, DesktopState>, stream_id: String) -> Result<(), String> {
    super::flatten(
        "stop_stream",
        parse_uuid(&stream_id).map(|id| match lock_stops(&state.stream_stops).get(&id) {
            Some(signal) => {
                signal.stop();
                tracing::info!(stream_id = %id, "stopping stream");
            }
            None => tracing::debug!(stream_id = %id, "no stream to stop"),
        }),
    )
}
