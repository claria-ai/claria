//! The harness's `ReportTurnProgress` listener.
//!
//! Every event the pipeline emits is stamped with the elapsed time since the
//! pass started, printed as it arrives, and kept so the run can be summarized
//! afterwards. This is the desktop's IPC channel replaced by a terminal.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use claria_report_pipeline::ReportTurnProgress;

/// One event and when it landed.
#[derive(Debug, Clone)]
pub struct TimedEvent {
    pub elapsed: Duration,
    pub event: ReportTurnProgress,
}

/// Records progress events and echoes them to stdout.
///
/// Cloneable and `Send + Sync` so it can back the `&dyn Fn` the pipeline
/// takes while the caller keeps reading it.
#[derive(Clone)]
pub struct ProgressRecorder {
    started: Instant,
    events: Arc<Mutex<Vec<TimedEvent>>>,
    echo: bool,
}

impl ProgressRecorder {
    pub fn new(echo: bool) -> Self {
        Self {
            started: Instant::now(),
            events: Arc::new(Mutex::new(Vec::new())),
            echo,
        }
    }

    /// Feed one event in. Poisoning is impossible here — nothing panics while
    /// the lock is held — but the harness prefers a dropped event to a
    /// panicking progress callback inside the pipeline, so it is not
    /// unwrapped.
    pub fn record(&self, event: ReportTurnProgress) {
        let elapsed = self.started.elapsed();
        if self.echo {
            println!("  [{:>7.2}s] {}", elapsed.as_secs_f64(), describe(&event));
        }
        if let Ok(mut events) = self.events.lock() {
            events.push(TimedEvent { elapsed, event });
        }
    }

    /// Every event so far, in arrival order.
    pub fn events(&self) -> Vec<TimedEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }

    /// How many Bedrock calls the pipeline announced.
    pub fn model_calls(&self) -> u32 {
        self.events()
            .iter()
            .filter(|timed| matches!(timed.event, ReportTurnProgress::ModelCallStarted { .. }))
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }
}

/// A one-line rendering of an event.
///
/// Section text rides along on `SectionCompleted`; it is deliberately reduced
/// to a block count here so the live feed stays scannable. The full sections
/// are printed once at the end from the returned workspace.
pub fn describe(event: &ReportTurnProgress) -> String {
    match event {
        ReportTurnProgress::RecordContextPrepared {
            included_files,
            unavailable_files,
            total_characters,
        } => format!(
            "record context: {included_files} files, {unavailable_files} unavailable, \
             {total_characters} characters"
        ),
        ReportTurnProgress::ModelCallStarted { call_number } => {
            format!("model call {call_number}")
        }
        ReportTurnProgress::ModelCallRetrying {
            call_number,
            attempt,
            max_attempts,
            delay_ms,
        } => format!(
            "model call {call_number} RETRYING (attempt {attempt}/{max_attempts}, \
             after {delay_ms}ms)"
        ),
        ReportTurnProgress::ToolStarted { name, context } => match context {
            Some(context) => format!("tool {name} started ({context})"),
            None => format!("tool {name} started"),
        },
        ReportTurnProgress::ToolFinished {
            name,
            context,
            status,
        } => match context {
            Some(context) => format!("tool {name} finished ({context}) — {status:?}"),
            None => format!("tool {name} finished — {status:?}"),
        },
        ReportTurnProgress::PlanRowPlanned { planned, total } => {
            format!("planned {planned}/{total} rows")
        }
        ReportTurnProgress::PlanBatchPlanned { first, last, total } => {
            format!("plan batch decided: sections {first}–{last} of {total}")
        }
        ReportTurnProgress::PlanReady { section_count } => {
            format!("plan ready: {section_count} sections")
        }
        ReportTurnProgress::SectionStarted {
            section_id,
            index,
            total,
        } => format!("section {section_id} started ({}/{total})", index + 1),
        ReportTurnProgress::SectionCompleted {
            section_id,
            section,
            drafted,
            total,
        } => format!(
            "section {section_id} completed ({drafted}/{total}, {} blocks)",
            section.blocks.len()
        ),
        ReportTurnProgress::SectionSkipped {
            section_id,
            drafted,
            total,
        } => format!("section {section_id} skipped ({drafted}/{total})"),
        ReportTurnProgress::SectionFailed {
            section_id,
            message,
            drafted,
            total,
        } => format!("section {section_id} FAILED ({drafted}/{total}): {message}"),
        ReportTurnProgress::TitleSet { title } => format!("title set: {title}"),
        ReportTurnProgress::ReviewPassStarted {
            property,
            index,
            total,
        } => format!("review {property} started ({index}/{total})"),
        ReportTurnProgress::ReviewPassCompleted {
            property,
            findings,
            completed,
            total,
        } => format!("review {property} done ({completed}/{total}, {findings} findings)"),
    }
}
