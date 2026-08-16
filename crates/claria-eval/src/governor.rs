//! The spend governor: a durable allowance on how many Bedrock-invoking runs
//! this harness may start.
//!
//! An agent driving the writer against a real account can spend real money in
//! a loop. Every Bedrock-invoking subcommand claims one attempt before it
//! calls anything, and the claim is written to disk *before* the call goes
//! out, so a crashed or killed run still costs its attempt. When the
//! allowance is gone the harness refuses to start until a human runs
//! `claria-eval grant <n>`.

use std::path::{Path, PathBuf};

use eyre::{Context, Result, eyre};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Attempts a fresh state file starts with.
pub const DEFAULT_ATTEMPTS_GRANTED: u32 = 10;

/// What a run's attempt is recorded as before it finishes.
pub const OUTCOME_STARTED: &str = "started";

/// The durable state file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendState {
    pub attempts_used: u32,
    pub attempts_granted: u32,
    pub total_cost_usd: f64,
    pub runs: Vec<SpendRun>,
}

impl Default for SpendState {
    fn default() -> Self {
        Self {
            attempts_used: 0,
            attempts_granted: DEFAULT_ATTEMPTS_GRANTED,
            total_cost_usd: 0.0,
            runs: Vec::new(),
        }
    }
}

impl SpendState {
    pub fn attempts_remaining(&self) -> u32 {
        self.attempts_granted.saturating_sub(self.attempts_used)
    }
}

/// One recorded attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendRun {
    pub timestamp: jiff::Timestamp,
    pub command: String,
    pub client_id: Option<Uuid>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub outcome: String,
}

/// The state file plus the path it came from.
#[derive(Debug)]
pub struct Governor {
    path: PathBuf,
    state: SpendState,
}

impl Governor {
    /// Open the state file, defaulting a fresh allowance when it does not
    /// exist yet. A file that exists but does not parse is an error — the
    /// alternative is silently handing back a full allowance.
    pub fn open(path: PathBuf) -> Result<Self> {
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .wrap_err("the spend state file exists but did not parse")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SpendState::default(),
            Err(error) => {
                return Err(error).wrap_err("could not read the spend state file");
            }
        };
        Ok(Self { path, state })
    }

    pub fn state(&self) -> &SpendState {
        &self.state
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Claim one attempt for a Bedrock-invoking run, refusing once the
    /// allowance is spent. The claim is persisted before this returns.
    pub fn claim(&mut self, command: &str, client_id: Option<Uuid>) -> Result<()> {
        if self.state.attempts_used >= self.state.attempts_granted {
            return Err(eyre!(
                "spend governor: {} of {} attempts used, so this run will not start. \
                 Raise the allowance with `claria-eval grant <n>` (a human action) — \
                 ${:.4} spent so far.",
                self.state.attempts_used,
                self.state.attempts_granted,
                self.state.total_cost_usd
            ));
        }
        self.state.attempts_used += 1;
        self.state.runs.push(SpendRun {
            timestamp: jiff::Timestamp::now(),
            command: command.to_string(),
            client_id,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            outcome: OUTCOME_STARTED.to_string(),
        });
        self.save()
    }

    /// Record what the claimed attempt actually cost. No-ops when nothing was
    /// claimed, so a settle without a claim cannot invent a run record.
    pub fn settle(
        &mut self,
        tokens_in: u64,
        tokens_out: u64,
        cost_usd: f64,
        outcome: &str,
    ) -> Result<()> {
        let Some(run) = self.state.runs.last_mut() else {
            return Ok(());
        };
        run.tokens_in = tokens_in;
        run.tokens_out = tokens_out;
        run.cost_usd = cost_usd;
        run.outcome = outcome.to_string();
        self.state.total_cost_usd += cost_usd;
        self.save()
    }

    /// Raise the allowance by `additional` attempts. The one human-only
    /// action in the harness.
    pub fn grant(&mut self, additional: u32) -> Result<u32> {
        self.state.attempts_granted = self.state.attempts_granted.saturating_add(additional);
        self.save()?;
        Ok(self.state.attempts_granted)
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .wrap_err("could not create the spend state directory")?;
        }
        let bytes = serde_json::to_vec_pretty(&self.state)
            .wrap_err("could not serialize the spend state")?;
        write_private_atomic(&self.path, &bytes)
    }
}

/// The directory the spend state lives in: the platform state directory when
/// there is one, otherwise the config directory.
pub fn state_dir() -> Result<PathBuf> {
    let base = dirs::state_dir()
        .or_else(dirs::config_dir)
        .ok_or_else(|| eyre!("no state or config directory found"))?;
    Ok(base.join("claria-eval"))
}

/// The default `--state` path.
pub fn default_state_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("spend.json"))
}

/// Write through a `0o600` temporary in the same directory and rename it into
/// place.
///
/// A trimmed copy of `claria_desktop::local_export::write_private_atomic`,
/// replicated because depending on `claria-desktop` would drag Tauri into
/// this binary. The file records what an agent spent against a real AWS
/// account, so it is owner-only like every other Claria-written file.
pub fn write_private_atomic(destination: &Path, bytes: &[u8]) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| eyre!("the spend state path has no parent directory"))?;
    let filename = destination
        .file_name()
        .ok_or_else(|| eyre!("the spend state path has no filename"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        filename.to_string_lossy(),
        Uuid::new_v4()
    ));

    let result = (|| -> Result<()> {
        {
            let mut file = create_private_new(&temporary)?;
            std::io::Write::write_all(&mut file, bytes)
                .wrap_err("could not write the spend state")?;
            std::io::Write::flush(&mut file).wrap_err("could not flush the spend state")?;
            file.sync_all()
                .wrap_err("could not sync the spend state to local storage")?;
        }
        std::fs::rename(&temporary, destination)
            .wrap_err("could not atomically place the spend state")?;
        Ok(())
    })();

    if result.is_err()
        && let Err(cleanup) = std::fs::remove_file(&temporary)
        && cleanup.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(error = %cleanup, "could not remove the failed spend state temporary");
    }
    result
}

#[cfg(unix)]
fn create_private_new(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .wrap_err("could not create a private spend state temporary")
}

/// Windows has no permission bits and this is a developer tool, not a
/// shipped surface, so the file is merely created fresh rather than ACL'd.
#[cfg(not(unix))]
fn create_private_new(path: &Path) -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .wrap_err("could not create a private spend state temporary")
}
