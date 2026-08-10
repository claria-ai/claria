//! In-process mock server helper for SDK-level tests.
//!
//! Spins up the axum router on an ephemeral port so AWS SDK clients can hit it
//! over real HTTP. Hands back the shared state so the test can preload
//! cassettes and inspect captured requests after the SDK call returns.

use std::net::SocketAddr;

use tokio::{net::TcpListener, task::JoinHandle};

use crate::{router, state};

pub struct MockServer {
    pub state: state::SharedState,
    pub endpoint: String,
    _server: JoinHandle<()>,
}

impl MockServer {
    /// Bind to `127.0.0.1:0`, start serving, return a handle. The server lives
    /// until the returned `MockServer` is dropped (the spawned task is
    /// detached but the bound socket releases on drop of this struct via the
    /// `JoinHandle` going out of scope and tokio cancelling).
    pub async fn spawn() -> Self {
        let shared = state::new_shared_state();
        let app = router::build_router(shared.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("local_addr");
        let endpoint = format!("http://{addr}");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            state: shared,
            endpoint,
            _server: server,
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self._server.abort();
    }
}
