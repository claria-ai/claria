//! `claria-eval` — drive the writer pipeline headlessly against a real AWS
//! environment.
//!
//! See the crate root of the library target for the config-boundary exception
//! this tool relies on and the PHI rules it follows.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use claria_core::models::report_run::SectionIntent;
use claria_eval::{
    EvalContext, config, cost::RunCost, governor::Governor, pipeline, preferences, progress,
    telemetry,
};
use eyre::{Result, WrapErr};
use tracing::{Instrument as _, field};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "claria-eval",
    version,
    about = "Headless eval harness for the Claria writer pipeline"
)]
struct Cli {
    /// The desktop `config.json` to read. Read-only; never written.
    #[arg(long, value_name = "PATH", global = true)]
    config: Option<PathBuf>,

    /// The spend governor's state file.
    #[arg(long, value_name = "PATH", global = true)]
    state: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List the clients in the bucket with their record counts. No Bedrock.
    ListClients,

    /// Run one planning pass and print the plan it produced.
    Plan {
        #[arg(long, value_name = "UUID")]
        client: Uuid,
        /// Start a fresh Writing session from this managed writer template
        /// instead of reusing the client's newest one.
        #[arg(long, value_name = "UUID")]
        template: Option<Uuid>,
        /// Guidance handed to the planner.
        #[arg(long, default_value = DEFAULT_INSTRUCTIONS)]
        instructions: String,
        #[arg(long, value_name = "MODEL_ID")]
        planner_model: Option<String>,
        #[arg(long, value_name = "MODEL_ID")]
        writer_model: Option<String>,
    },

    /// Plan, approve the plan as-is, draft every section, then run the
    /// completion gate.
    Run {
        #[arg(long, value_name = "UUID")]
        client: Uuid,
        #[arg(long, value_name = "UUID")]
        template: Option<Uuid>,
        #[arg(long, default_value = DEFAULT_INSTRUCTIONS)]
        instructions: String,
        #[arg(long, value_name = "MODEL_ID")]
        planner_model: Option<String>,
        #[arg(long, value_name = "MODEL_ID")]
        writer_model: Option<String>,
    },

    /// Print attempts used, dollars spent, and the run history. No Bedrock.
    Report,

    /// Raise the attempt allowance by `n`. A human action.
    Grant {
        #[arg(value_name = "N")]
        n: u32,
    },
}

const DEFAULT_INSTRUCTIONS: &str = "Draft the whole report from the client's readable records.";

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let telemetry = telemetry::init()?;

    let result = dispatch(cli).await;

    // The root span is dropped before this point (it lives inside
    // `dispatch`), so the flush has everything the run produced. A flush
    // failure fails the command even when the run itself succeeded: spans
    // that never reached the receiver are spans nobody can read.
    let flushed = telemetry.shutdown();
    result.and(flushed)
}

async fn dispatch(cli: Cli) -> Result<()> {
    let state_path = match cli.state {
        Some(path) => path,
        None => claria_eval::governor::default_state_path()?,
    };

    match cli.command {
        Command::Report => report(&state_path),
        Command::Grant { n } => grant(&state_path, n),
        Command::ListClients => {
            let context = context(cli.config.as_deref()).await?;
            list_clients(&context).await
        }
        Command::Plan {
            client,
            template,
            instructions,
            planner_model,
            writer_model,
        } => {
            let context = context(cli.config.as_deref()).await?;
            let span = tracing::info_span!(
                "eval.plan",
                client_id = %client,
                outcome = field::Empty,
                cost_usd = field::Empty,
                converse_calls = field::Empty
            );
            plan(
                &context,
                &state_path,
                client,
                template,
                &instructions,
                planner_model.as_deref(),
                writer_model.as_deref(),
            )
            .instrument(span)
            .await
        }
        Command::Run {
            client,
            template,
            instructions,
            planner_model,
            writer_model,
        } => {
            let context = context(cli.config.as_deref()).await?;
            let span = tracing::info_span!(
                "eval.run",
                client_id = %client,
                outcome = field::Empty,
                cost_usd = field::Empty
            );
            run(
                &context,
                &state_path,
                client,
                template,
                &instructions,
                planner_model.as_deref(),
                writer_model.as_deref(),
            )
            .instrument(span)
            .await
        }
    }
}

/// Load the desktop's config and build the AWS handles from it.
async fn context(config_path: Option<&std::path::Path>) -> Result<EvalContext> {
    let path = match config_path {
        Some(path) => path.to_path_buf(),
        None => config::default_config_path()?,
    };
    let config = config::load(&path)?;
    let bucket = config.bucket()?;
    let sdk_config = config::build_aws_config(&config).await;
    let s3 = claria_storage::client::from_config(&sdk_config);
    eprintln!("config: {} | bucket: {bucket}", path.display());
    Ok(EvalContext {
        sdk_config,
        s3,
        bucket,
    })
}

async fn list_clients(context: &EvalContext) -> Result<()> {
    let clients = pipeline::list_clients(context).await?;
    println!(
        "{:<38} {:>7} {:>10}  name",
        "client_id", "records", "sessions"
    );
    for client in &clients {
        println!(
            "{:<38} {:>7} {:>10}  {}",
            client.client_id, client.record_count, client.workspace_count, client.name
        );
    }
    println!("\n{} clients", clients.len());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn plan(
    context: &EvalContext,
    state_path: &std::path::Path,
    client_id: Uuid,
    template: Option<Uuid>,
    instructions: &str,
    planner_override: Option<&str>,
    writer_override: Option<&str>,
) -> Result<()> {
    let mut governor = Governor::open(state_path.to_path_buf())?;
    governor.claim("plan", Some(client_id))?;
    announce_claim(&governor);

    let models = preferences::resolve(context, planner_override, writer_override).await?;
    println!(
        "planner: {} | writer: {}",
        models.planner_model_id, models.writer_model_id
    );
    let workspace = pipeline::prepare_workspace(context, client_id, choice(template)).await?;
    println!(
        "report {} at revision {} — {} sections",
        workspace.report_id,
        workspace.draft.revision,
        workspace.draft.content.sections.len()
    );

    let recorder = progress::ProgressRecorder::new(true);
    let outcome = pipeline::plan(
        context,
        &workspace,
        &models.planner_model_id,
        &models.writer_model_id,
        instructions,
        &recorder,
    )
    .await;

    settle(
        &mut governor,
        &outcome.as_ref().map(|plan| plan.cost).ok(),
        outcome.is_ok(),
    )?;
    let plan = outcome?;
    print_plan(&plan);
    print_cost("plan", &plan.cost, plan.converse_calls, &governor);
    tracing::Span::current().record("outcome", "ok");
    tracing::Span::current().record("cost_usd", plan.cost.cost_usd);
    tracing::Span::current().record("converse_calls", plan.converse_calls);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run(
    context: &EvalContext,
    state_path: &std::path::Path,
    client_id: Uuid,
    template: Option<Uuid>,
    instructions: &str,
    planner_override: Option<&str>,
    writer_override: Option<&str>,
) -> Result<()> {
    let mut governor = Governor::open(state_path.to_path_buf())?;
    governor.claim("run", Some(client_id))?;
    announce_claim(&governor);

    let models = preferences::resolve(context, planner_override, writer_override).await?;
    println!(
        "planner: {} | writer: {}",
        models.planner_model_id, models.writer_model_id
    );
    let workspace = pipeline::prepare_workspace(context, client_id, choice(template)).await?;
    println!(
        "report {} at revision {} — {} sections",
        workspace.report_id,
        workspace.draft.revision,
        workspace.draft.content.sections.len()
    );

    let outcome = draft_run(context, &workspace, &models, instructions).await;
    settle(
        &mut governor,
        &outcome.as_ref().map(|report| report.cost).ok(),
        outcome.is_ok(),
    )?;
    let report = outcome?;

    print_plan(&report.plan);
    println!("\nsections");
    let mut sections = report
        .outcome
        .workspace
        .draft
        .content
        .sections
        .iter()
        .collect::<Vec<_>>();
    sections.sort_by_key(|section| section.heading.clone());
    for section in sections {
        println!(
            "  {} — {} blocks{}",
            section.heading,
            section.blocks.len(),
            if section.skipped { " (skipped)" } else { "" }
        );
    }
    println!("\ntitle: {}", report.outcome.workspace.draft.content.title);
    println!(
        "completion gate: {}",
        if report.completion.complete {
            "complete".to_string()
        } else {
            format!("{} failing checks", report.completion.checks.len())
        }
    );
    for check in &report.completion.checks {
        println!("  {:?}: {}", check.kind, check.detail);
    }
    print_cost(
        "run",
        &report.cost,
        report.plan.converse_calls + report.outcome.attempt.converse_calls,
        &governor,
    );
    tracing::Span::current().record("outcome", "ok");
    tracing::Span::current().record("cost_usd", report.cost.cost_usd);
    Ok(())
}

/// Plan, auto-approve, draft. Split out so the pipeline calls sit under their
/// own spans inside the command's root span.
async fn draft_run(
    context: &EvalContext,
    workspace: &claria_core::models::report::ReportWorkspace,
    models: &preferences::ResolvedModels,
    instructions: &str,
) -> Result<pipeline::DraftReport> {
    let plan_recorder = progress::ProgressRecorder::new(true);
    let plan_span = tracing::info_span!("eval.plan", outcome = field::Empty);
    let plan = pipeline::plan(
        context,
        workspace,
        &models.planner_model_id,
        &models.writer_model_id,
        instructions,
        &plan_recorder,
    )
    .instrument(plan_span.clone())
    .await;
    plan_span.record("outcome", if plan.is_ok() { "ok" } else { "failed" });
    let plan = plan?;
    let run_id = plan.run.run_id;
    println!("\nplan approved as-is (run {run_id}); drafting\n");

    let draft_recorder = progress::ProgressRecorder::new(true);
    let batch_span = tracing::info_span!(
        "eval.batch",
        run_id = %run_id,
        sections = plan.run.sections.len(),
        outcome = field::Empty
    );
    let drafted = pipeline::draft(
        context,
        workspace,
        run_id,
        &models.writer_model_id,
        plan,
        &draft_recorder,
    )
    .instrument(batch_span.clone())
    .await;
    batch_span.record("outcome", if drafted.is_ok() { "ok" } else { "failed" });
    drafted
}

fn choice(template: Option<Uuid>) -> pipeline::WorkspaceChoice {
    match template {
        Some(id) => pipeline::WorkspaceChoice::FreshFromTemplate(id),
        None => pipeline::WorkspaceChoice::Existing,
    }
}

/// Record what the claimed attempt cost. A failed pass still spent whatever
/// Bedrock had already billed, which is why the failure branch settles too.
fn settle(governor: &mut Governor, cost: &Option<RunCost>, ok: bool) -> Result<()> {
    let cost = cost.unwrap_or_default();
    governor.settle(
        cost.total_input_tokens(),
        cost.output_tokens,
        cost.cost_usd,
        if ok { "ok" } else { "failed" },
    )
}

fn announce_claim(governor: &Governor) {
    let state = governor.state();
    eprintln!(
        "spend governor: attempt {} of {} ({} left)",
        state.attempts_used,
        state.attempts_granted,
        state.attempts_remaining()
    );
}

fn print_plan(plan: &pipeline::PlanReport) {
    let Some(run_plan) = plan.run.plan.as_ref() else {
        println!("\nno plan on the run");
        return;
    };
    println!("\nplan ({} entries)", run_plan.entries.len());
    for entry in &run_plan.entries {
        let intent = match entry.intent {
            SectionIntent::Draft => "draft",
            SectionIntent::Rewrite => "rewrite",
            SectionIntent::Keep => "keep",
            SectionIntent::Skip => "skip",
        };
        println!(
            "  [{intent}{}] {} ({})",
            if entry.required { ", required" } else { "" },
            entry.heading,
            entry.section_id
        );
        if !entry.scope.is_empty() {
            println!("      scope: {}", entry.scope);
        }
        for evidence in &entry.evidence {
            match &evidence.note {
                Some(note) => println!("      evidence: {} — {note}", evidence.filename),
                None => println!("      evidence: {}", evidence.filename),
            }
        }
    }
    if !run_plan.plan_warnings.is_empty() {
        println!("  warnings: {}", run_plan.plan_warnings.join("; "));
    }
}

fn print_cost(label: &str, cost: &RunCost, converse_calls: u32, governor: &Governor) {
    let state = governor.state();
    println!(
        "\n{label}: {converse_calls} Bedrock calls | in {} (of which {} cache read, {} cache write) \
         | out {} | ${:.4}{}",
        cost.total_input_tokens(),
        cost.cache_read_tokens,
        cost.cache_write_tokens,
        cost.output_tokens,
        cost.cost_usd,
        if cost.unpriced_calls {
            " (at least — some calls used a model the pricing table does not know)"
        } else {
            ""
        }
    );
    println!(
        "cumulative: {} of {} attempts used, ${:.4} spent",
        state.attempts_used, state.attempts_granted, state.total_cost_usd
    );
}

fn report(state_path: &std::path::Path) -> Result<()> {
    let governor = Governor::open(state_path.to_path_buf())?;
    let state = governor.state();
    println!("state file: {}", governor.path().display());
    println!(
        "attempts: {} used of {} granted ({} left)",
        state.attempts_used,
        state.attempts_granted,
        state.attempts_remaining()
    );
    println!("total: ${:.4}", state.total_cost_usd);
    if state.runs.is_empty() {
        println!("\nno runs recorded");
        return Ok(());
    }
    println!(
        "\n{:<25} {:<12} {:<38} {:>9} {:>9} {:>10}  outcome",
        "when", "command", "client", "tokens_in", "tokens_out", "cost_usd"
    );
    for run in &state.runs {
        println!(
            "{:<25} {:<12} {:<38} {:>9} {:>9} {:>10.4}  {}",
            run.timestamp.to_string(),
            run.command,
            run.client_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.tokens_in,
            run.tokens_out,
            run.cost_usd,
            run.outcome
        );
    }
    Ok(())
}

fn grant(state_path: &std::path::Path, additional: u32) -> Result<()> {
    let mut governor =
        Governor::open(state_path.to_path_buf()).wrap_err("could not open the spend state")?;
    let granted = governor.grant(additional)?;
    let state = governor.state();
    println!(
        "granted {additional} more attempts: {} used of {granted} ({} left)",
        state.attempts_used,
        state.attempts_remaining()
    );
    Ok(())
}
