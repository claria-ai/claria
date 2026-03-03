use aws_sdk_costexplorer::types::{DateInterval, Granularity, GroupDefinition, GroupDefinitionType};

use crate::error::BillingError;
use crate::query::{parse_response, validate_query};
use crate::types::{CostAndUsageResult, CostGranularity, CostQuery};

/// Fetch cost and usage data from AWS Cost Explorer.
pub async fn get_cost_and_usage(
    config: &aws_config::SdkConfig,
    query: &CostQuery,
) -> Result<CostAndUsageResult, BillingError> {
    validate_query(query)?;

    let client = aws_sdk_costexplorer::Client::new(config);

    let granularity = match query.granularity {
        CostGranularity::Hourly => Granularity::Hourly,
        CostGranularity::Daily => Granularity::Daily,
        CostGranularity::Monthly => Granularity::Monthly,
    };

    // HOURLY granularity requires ISO 8601 datetime format (yyyy-MM-ddTHH:mm:ssZ).
    // DAILY and MONTHLY accept plain dates (yyyy-MM-dd).
    let (start, end) = if matches!(query.granularity, CostGranularity::Hourly) {
        (
            format!("{}T00:00:00Z", query.start_date),
            format!("{}T00:00:00Z", query.end_date),
        )
    } else {
        (query.start_date.clone(), query.end_date.clone())
    };

    let time_period = DateInterval::builder()
        .start(start)
        .end(end)
        .build()
        .map_err(|e| BillingError::ApiError(format!("invalid time period: {e}")))?;

    let mut req = client
        .get_cost_and_usage()
        .time_period(time_period)
        .granularity(granularity)
        .metrics("UnblendedCost");

    if query.group_by_service {
        req = req.group_by(
            GroupDefinition::builder()
                .r#type(GroupDefinitionType::Dimension)
                .key("SERVICE")
                .build(),
        );
    }

    let resp = req.send().await.map_err(classify_sdk_error)?;

    Ok(parse_response(resp.results_by_time()))
}

/// Lightweight probe to verify Cost Explorer is enabled.
///
/// Makes a single GetCostAndUsage call with a 1-day window (yesterday → today).
/// Discards the response on success — we only care whether the call errors.
pub async fn probe_cost_explorer(config: &aws_config::SdkConfig) -> Result<(), BillingError> {
    let today = jiff::Zoned::now().date();
    let yesterday = today
        .checked_sub(jiff::Span::new().days(1))
        .unwrap_or(today);

    let time_period = DateInterval::builder()
        .start(yesterday.strftime("%Y-%m-%d").to_string())
        .end(today.strftime("%Y-%m-%d").to_string())
        .build()
        .map_err(|e| BillingError::ApiError(format!("invalid time period: {e}")))?;

    let client = aws_sdk_costexplorer::Client::new(config);

    client
        .get_cost_and_usage()
        .time_period(time_period)
        .granularity(Granularity::Daily)
        .metrics("UnblendedCost")
        .send()
        .await
        .map_err(classify_sdk_error)?;

    Ok(())
}

fn classify_sdk_error(
    err: aws_sdk_costexplorer::error::SdkError<
        aws_sdk_costexplorer::operation::get_cost_and_usage::GetCostAndUsageError,
    >,
) -> BillingError {
    use aws_sdk_costexplorer::operation::get_cost_and_usage::GetCostAndUsageError;
    use aws_smithy_runtime_api::client::result::SdkError;

    // Try structured matching first (most reliable).
    if let SdkError::ServiceError(ref service_err) = err {
        let inner = service_err.err();

        // DataUnavailableException is a modeled variant.
        if matches!(inner, GetCostAndUsageError::DataUnavailableException(_)) {
            return BillingError::CostExplorerNotEnabled;
        }

        // AccessDeniedException is NOT modeled — it arrives as Unhandled.
        // Check the error code from the metadata.
        if let Some(code) = aws_smithy_types::error::metadata::ProvideErrorMetadata::code(inner) {
            tracing::debug!(error_code = code, "Cost Explorer service error code");
            if code == "AccessDeniedException" {
                return BillingError::AccessDenied;
            }
        }

        // Extract the HTTP response body for better diagnostics.
        let raw = service_err.raw();
        let body = std::str::from_utf8(raw.body().bytes().unwrap_or_default())
            .unwrap_or("<non-utf8>");
        let status = raw.status().as_u16();
        tracing::warn!(
            status,
            body,
            inner_debug = ?inner,
            "Cost Explorer service error — unclassified"
        );

        // Match on HTTP body as last resort for service errors.
        if body.contains("AccessDenied") || body.contains("is not authorized") {
            return BillingError::AccessDenied;
        }
        if body.contains("DataUnavailable") {
            return BillingError::CostExplorerNotEnabled;
        }

        return BillingError::ApiError(body.to_string());
    }

    // Non-service errors (dispatch failure, timeout, etc.)
    let msg = format!("{err:#}");
    tracing::warn!(error = %msg, "Cost Explorer non-service error");
    BillingError::ApiError(msg)
}
