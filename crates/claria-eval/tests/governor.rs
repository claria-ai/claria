//! The spend governor: what it refuses, when, and what unblocks it.

use claria_eval::governor::{DEFAULT_ATTEMPTS_GRANTED, Governor, OUTCOME_STARTED};
use uuid::Uuid;

fn governor(directory: &tempfile::TempDir) -> Governor {
    Governor::open(directory.path().join("spend.json")).expect("open a fresh governor")
}

#[test]
fn a_fresh_state_file_starts_with_the_default_allowance() {
    let directory = tempfile::tempdir().expect("temp dir");
    let governor = governor(&directory);
    assert_eq!(governor.state().attempts_used, 0);
    assert_eq!(governor.state().attempts_granted, DEFAULT_ATTEMPTS_GRANTED);
    assert_eq!(governor.state().total_cost_usd, 0.0);
    assert!(governor.state().runs.is_empty());
}

/// The claim has to survive the process, or a killed run costs nothing and an
/// agent in a loop spends without bound.
#[test]
fn a_claim_is_durable_before_the_run_starts() {
    let directory = tempfile::tempdir().expect("temp dir");
    let client_id = Uuid::new_v4();
    {
        let mut governor = governor(&directory);
        governor.claim("plan", Some(client_id)).expect("claim");
    }

    let reopened = governor(&directory);
    assert_eq!(reopened.state().attempts_used, 1);
    let run = reopened.state().runs.last().expect("one recorded run");
    assert_eq!(run.command, "plan");
    assert_eq!(run.client_id, Some(client_id));
    assert_eq!(run.outcome, OUTCOME_STARTED);
    assert_eq!(run.cost_usd, 0.0);
}

#[test]
fn settling_a_claim_records_what_it_cost() {
    let directory = tempfile::tempdir().expect("temp dir");
    let mut governor = governor(&directory);
    governor.claim("run", None).expect("claim");
    governor.settle(1_200, 340, 0.0195, "ok").expect("settle");

    let reopened = Governor::open(directory.path().join("spend.json")).expect("reopen");
    let run = reopened.state().runs.last().expect("one recorded run");
    assert_eq!(run.tokens_in, 1_200);
    assert_eq!(run.tokens_out, 340);
    assert_eq!(run.outcome, "ok");
    assert!((reopened.state().total_cost_usd - 0.0195).abs() < 1e-9);
}

#[test]
fn the_governor_refuses_once_the_allowance_is_spent_and_grant_unblocks_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let mut governor = governor(&directory);
    for _ in 0..DEFAULT_ATTEMPTS_GRANTED {
        governor
            .claim("plan", None)
            .expect("claim within allowance");
        governor.settle(10, 10, 0.001, "ok").expect("settle");
    }

    let refusal = governor
        .claim("plan", None)
        .expect_err("the eleventh claim is refused");
    let message = format!("{refusal}");
    assert!(
        message.contains("grant"),
        "the refusal must name the command that unblocks it: {message}"
    );
    assert_eq!(governor.state().attempts_used, DEFAULT_ATTEMPTS_GRANTED);
    assert_eq!(
        governor.state().runs.len() as u32,
        DEFAULT_ATTEMPTS_GRANTED,
        "a refused claim records no run"
    );

    let granted = governor.grant(3).expect("grant");
    assert_eq!(granted, DEFAULT_ATTEMPTS_GRANTED + 3);
    assert_eq!(governor.state().attempts_remaining(), 3);
    governor.claim("plan", None).expect("claim after grant");
}

/// A refusal must not be recoverable by deleting nothing and rerunning: the
/// refusal is read back from disk, not from memory.
#[test]
fn the_refusal_survives_a_reopen() {
    let directory = tempfile::tempdir().expect("temp dir");
    {
        let mut governor = governor(&directory);
        governor.grant(0).expect("persist the default state");
        for _ in 0..DEFAULT_ATTEMPTS_GRANTED {
            governor.claim("run", None).expect("claim");
        }
    }
    let mut reopened = governor(&directory);
    assert!(reopened.claim("run", None).is_err());
}

#[cfg(unix)]
#[test]
fn the_state_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().expect("temp dir");
    let mut governor = governor(&directory);
    governor.claim("plan", None).expect("claim");

    let mode = std::fs::metadata(directory.path().join("spend.json"))
        .expect("state file")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "state file mode was {:o}",
        mode & 0o777
    );
}
