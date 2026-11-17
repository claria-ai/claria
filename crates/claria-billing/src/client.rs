use aws_sdk_costexplorer::{
    operation::RequestId,
    types::{DateInterval, Granularity, GroupDefinition, GroupDefinitionType},
};

use crate::{
    error::BillingError,
    query::{parse_response, validate_query},
    types::{CostAndUsageResult, CostGranularity, CostQuery},
};

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

    if let SdkError::ServiceError(ref service_err) = err {
        let inner = service_err.err();
        // Classify using the structured metadata first. The raw body is used
        // only for older unmodeled responses, never copied into a log.
        let raw = service_err.raw();
        let body =
            std::str::from_utf8(raw.body().bytes().unwrap_or_default()).unwrap_or("<non-utf8>");
        let status = raw.status().as_u16();
        let metadata = aws_smithy_types::error::metadata::ProvideErrorMetadata::meta(inner);
        let code = metadata.code().unwrap_or("UnknownServiceError");
        let request_id = metadata.request_id();
        tracing::warn!(
            status,
            code = claria_core::provider_diagnostic::safe_service_code(code),
            request_id = request_id.map(claria_core::provider_diagnostic::safe_request_id),
            provider_reason = claria_core::provider_diagnostic::message_reason(body),
            provider_field = claria_core::provider_diagnostic::message_field(body),
            message_sha256 = claria_core::provider_diagnostic::message_sha256(body),
            "Cost Explorer service error"
        );

        if matches!(inner, GetCostAndUsageError::DataUnavailableException(_)) {
            return BillingError::CostExplorerNotEnabled;
        }
        if code == "AccessDeniedException" {
            return BillingError::AccessDenied;
        }

        // Match on HTTP body as last resort for service errors.
        if body.contains("AccessDenied") || body.contains("is not authorized") {
            return BillingError::AccessDenied;
        }
        if body.contains("DataUnavailable") {
            return BillingError::CostExplorerNotEnabled;
        }

        return BillingError::ApiError(format!(
            "Cost Explorer returned {code} (HTTP {status}, request ID {})",
            request_id.unwrap_or("unavailable")
        ));
    }

    // Non-service errors (dispatch failure, timeout, etc.)
    let msg = format!("{err:#}");
    tracing::warn!(
        message_sha256 = claria_core::provider_diagnostic::message_sha256(&msg),
        "Cost Explorer non-service error"
    );
    BillingError::ApiError("Cost Explorer transport error".to_string())
}
