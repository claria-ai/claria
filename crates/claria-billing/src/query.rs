use aws_sdk_costexplorer::types::ResultByTime;

use crate::error::BillingError;
use crate::types::{
    CostAndUsageResult, CostGranularity, CostQuery, CostResultGroup, CostTimePeriod,
};

/// Validate a cost query before sending it to AWS.
///
/// Pure function — no network calls.
pub fn validate_query(query: &CostQuery) -> Result<(), BillingError> {
    let start = jiff::civil::Date::strptime("%Y-%m-%d", &query.start_date).map_err(|_| {
        BillingError::InvalidQuery(format!("invalid start_date format: {}", query.start_date))
    })?;
    let end = jiff::civil::Date::strptime("%Y-%m-%d", &query.end_date).map_err(|_| {
        BillingError::InvalidQuery(format!("invalid end_date format: {}", query.end_date))
    })?;

    if start >= end {
        return Err(BillingError::InvalidQuery(
            "start_date must be before end_date".into(),
        ));
    }

    let span = start.until(end).map_err(|e| {
        BillingError::InvalidQuery(format!("failed to compute date range: {e}"))
    })?;

    let days = span.get_days();

    if matches!(query.granularity, CostGranularity::Hourly) && days > 14 {
        return Err(BillingError::InvalidQuery(
            "hourly granularity supports a maximum of 14 days".into(),
        ));
    }

    // 13 months ≈ 396 days. Be slightly generous to avoid off-by-one on month boundaries.
    if days > 396 {
        return Err(BillingError::InvalidQuery(
            "time range cannot exceed 13 months".into(),
        ));
    }

    Ok(())
}

/// Parse the AWS SDK `ResultByTime` response into our typed result.
///
/// Pure function — no network calls.
pub fn parse_response(results_by_time: &[ResultByTime]) -> CostAndUsageResult {
    let periods = results_by_time
        .iter()
        .map(|rbt| {
            let (start, end) = match rbt.time_period() {
                Some(tp) => (tp.start().to_string(), tp.end().to_string()),
                None => (String::new(), String::new()),
            };

            let groups = if rbt.groups().is_empty() {
                // Ungrouped response — total is in `total`
                rbt.total()
                    .map(|totals| {
                        totals
                            .iter()
                            .map(|(metric_name, metric_value)| CostResultGroup {
                                key: metric_name.clone(),
                                amount: metric_value
                                    .amount()
                                    .unwrap_or("0.00")
                                    .to_string(),
                                unit: metric_value.unit().unwrap_or("USD").to_string(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                // Grouped response — each group has keys + metrics
                rbt.groups()
                    .iter()
                    .map(|group| {
                        let key = group
                            .keys()
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "Unknown".to_string());
                        let (amount, unit) = group
                            .metrics()
                            .and_then(|m| m.get("UnblendedCost"))
                            .map(|mv| {
                                (
                                    mv.amount().unwrap_or("0.00").to_string(),
                                    mv.unit().unwrap_or("USD").to_string(),
                                )
                            })
                            .unwrap_or_else(|| ("0.00".to_string(), "USD".to_string()));

                        CostResultGroup { key, amount, unit }
                    })
                    .collect()
            };

            CostTimePeriod {
                start,
                end,
                groups,
            }
        })
        .collect();

    CostAndUsageResult { periods }
}
