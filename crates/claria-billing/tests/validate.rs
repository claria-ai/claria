use claria_billing::{validate_query, CostGranularity, CostQuery};

#[test]
fn test_valid_daily_query() {
    let query = CostQuery {
        start_date: "2025-01-01".into(),
        end_date: "2025-01-31".into(),
        granularity: CostGranularity::Daily,
        group_by_service: false,
    };
    assert!(validate_query(&query).is_ok());
}

#[test]
fn test_valid_hourly_query() {
    let query = CostQuery {
        start_date: "2025-03-01".into(),
        end_date: "2025-03-08".into(),
        granularity: CostGranularity::Hourly,
        group_by_service: false,
    };
    assert!(validate_query(&query).is_ok());
}

#[test]
fn test_valid_monthly_query() {
    let query = CostQuery {
        start_date: "2024-03-01".into(),
        end_date: "2025-03-01".into(),
        granularity: CostGranularity::Monthly,
        group_by_service: true,
    };
    assert!(validate_query(&query).is_ok());
}

#[test]
fn test_hourly_rejects_over_14_days() {
    let query = CostQuery {
        start_date: "2025-03-01".into(),
        end_date: "2025-03-16".into(),
        granularity: CostGranularity::Hourly,
        group_by_service: false,
    };
    let err = validate_query(&query).unwrap_err();
    assert!(err.to_string().contains("14 days"));
}

#[test]
fn test_rejects_start_after_end() {
    let query = CostQuery {
        start_date: "2025-03-15".into(),
        end_date: "2025-03-01".into(),
        granularity: CostGranularity::Daily,
        group_by_service: false,
    };
    let err = validate_query(&query).unwrap_err();
    assert!(err.to_string().contains("before"));
}

#[test]
fn test_rejects_range_over_13_months() {
    let query = CostQuery {
        start_date: "2023-01-01".into(),
        end_date: "2025-03-01".into(),
        granularity: CostGranularity::Monthly,
        group_by_service: false,
    };
    let err = validate_query(&query).unwrap_err();
    assert!(err.to_string().contains("13 months"));
}

#[test]
fn test_rejects_invalid_date_format() {
    let query = CostQuery {
        start_date: "not-a-date".into(),
        end_date: "2025-03-01".into(),
        granularity: CostGranularity::Daily,
        group_by_service: false,
    };
    let err = validate_query(&query).unwrap_err();
    assert!(err.to_string().contains("invalid"));
}

#[test]
fn test_same_start_end_rejected() {
    let query = CostQuery {
        start_date: "2025-03-01".into(),
        end_date: "2025-03-01".into(),
        granularity: CostGranularity::Daily,
        group_by_service: false,
    };
    let err = validate_query(&query).unwrap_err();
    assert!(err.to_string().contains("before"));
}
