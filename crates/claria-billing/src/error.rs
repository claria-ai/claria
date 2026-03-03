use thiserror::Error;

#[derive(Debug, Error)]
pub enum BillingError {
    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("Cost Explorer is not enabled in the AWS console")]
    CostExplorerNotEnabled,

    #[error("access denied — missing ce:GetCostAndUsage permission")]
    AccessDenied,

    #[error("AWS API error: {0}")]
    ApiError(String),
}
