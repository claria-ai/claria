use aws_sdk_costexplorer::types::{DateInterval, Group, MetricValue, ResultByTime};
use claria_billing::parse_response;

fn date_interval(start: &str, end: &str) -> DateInterval {
    DateInterval::builder()
        .start(start)
        .end(end)
        .build()
        .unwrap()
}

fn metric(amount: &str, unit: &str) -> MetricValue {
    MetricValue::builder().amount(amount).unit(unit).build()
}

#[test]
fn test_parse_ungrouped_single_period() {
    let results = vec![ResultByTime::builder()
        .time_period(date_interval("2025-03-01", "2025-03-02"))
        .total("UnblendedCost", metric("12.34", "USD"))
        .build()];

    let parsed = parse_response(&results);
    assert_eq!(parsed.periods.len(), 1);
    assert_eq!(parsed.periods[0].start, "2025-03-01");
    assert_eq!(parsed.periods[0].end, "2025-03-02");
    assert_eq!(parsed.periods[0].groups.len(), 1);
    assert_eq!(parsed.periods[0].groups[0].key, "UnblendedCost");
    assert_eq!(parsed.periods[0].groups[0].amount, "12.34");
    assert_eq!(parsed.periods[0].groups[0].unit, "USD");
}

#[test]
fn test_parse_grouped_by_service() {
    let results = vec![ResultByTime::builder()
        .time_period(date_interval("2025-03-01", "2025-03-02"))
        .groups(
            Group::builder()
                .keys("Amazon Bedrock")
                .metrics("UnblendedCost", metric("8.50", "USD"))
                .build(),
        )
        .groups(
            Group::builder()
                .keys("Amazon S3")
                .metrics("UnblendedCost", metric("1.20", "USD"))
                .build(),
        )
        .build()];

    let parsed = parse_response(&results);
    assert_eq!(parsed.periods.len(), 1);
    assert_eq!(parsed.periods[0].groups.len(), 2);
    assert_eq!(parsed.periods[0].groups[0].key, "Amazon Bedrock");
    assert_eq!(parsed.periods[0].groups[0].amount, "8.50");
    assert_eq!(parsed.periods[0].groups[1].key, "Amazon S3");
    assert_eq!(parsed.periods[0].groups[1].amount, "1.20");
}

#[test]
fn test_parse_multiple_periods() {
    let results = vec![
        ResultByTime::builder()
            .time_period(date_interval("2025-03-01", "2025-03-02"))
            .total("UnblendedCost", metric("5.00", "USD"))
            .build(),
        ResultByTime::builder()
            .time_period(date_interval("2025-03-02", "2025-03-03"))
            .total("UnblendedCost", metric("7.50", "USD"))
            .build(),
        ResultByTime::builder()
            .time_period(date_interval("2025-03-03", "2025-03-04"))
            .total("UnblendedCost", metric("3.25", "USD"))
            .build(),
    ];

    let parsed = parse_response(&results);
    assert_eq!(parsed.periods.len(), 3);
    assert_eq!(parsed.periods[0].start, "2025-03-01");
    assert_eq!(parsed.periods[1].start, "2025-03-02");
    assert_eq!(parsed.periods[2].start, "2025-03-03");
}

#[test]
fn test_parse_empty_response() {
    let parsed = parse_response(&[]);
    assert!(parsed.periods.is_empty());
}

#[test]
fn test_parse_missing_amount_defaults_to_zero() {
    let results = vec![ResultByTime::builder()
        .time_period(date_interval("2025-03-01", "2025-03-02"))
        .groups(
            Group::builder()
                .keys("Amazon Bedrock")
                // No metrics set — should default to "0.00"
                .build(),
        )
        .build()];

    let parsed = parse_response(&results);
    assert_eq!(parsed.periods[0].groups[0].amount, "0.00");
    assert_eq!(parsed.periods[0].groups[0].unit, "USD");
}
