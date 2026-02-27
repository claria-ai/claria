use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

/// Audit logging middleware.
///
/// Logs every API request as a structured audit event using `tracing`.
/// In production, these events flow to CloudTrail via the configured
/// tracing subscriber.
pub async fn audit_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().path().to_string();

    let response = next.run(req).await;

    let status = response.status().as_u16();
    tracing::info!(
        method = %method,
        path = %uri,
        status = status,
        "api_request"
    );

    response
}
