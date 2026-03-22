mod params;
mod router;
mod scenarios;
mod services;
mod state;
mod xml;

use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(name = "claria-mock-aws")]
#[command(about = "Mock AWS service for Claria E2E testing")]
struct Cli {
    /// Port to listen on
    #[arg(short, long, default_value = "9000")]
    port: u16,

    /// Load a scenario on startup
    #[arg(short, long)]
    scenario: Option<String>,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    let shared_state = state::new_shared_state();

    if let Some(scenario) = &cli.scenario {
        let mut st = shared_state.write().await;
        scenarios::load(scenario, &mut st)
            .map_err(|e| eyre::eyre!("Unknown scenario: {e}"))?;
        info!("Loaded scenario: {scenario}");
    }

    let app = router::build_router(shared_state);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], cli.port));
    info!("Mock AWS listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
