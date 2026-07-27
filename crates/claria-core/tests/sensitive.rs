//! `Sensitive` exists to make a PHI leak into a log require intent. These
//! tests pin the two properties that claim rests on: the renderings say
//! nothing, and the wire form says everything.

use claria_core::sensitive::Sensitive;

const QUERY: &str = "recurring migraines since the collision";

#[test]
fn display_renders_redacted() {
    let value = Sensitive::new(QUERY.to_string());
    assert_eq!(format!("{value}"), "[redacted]");
}

#[test]
fn debug_renders_redacted() {
    let value = Sensitive::new(QUERY.to_string());
    assert_eq!(format!("{value:?}"), "[redacted]");
}

/// The realistic failure mode: a future `tracing::info!(query = %q)` or
/// `?q`. Both formatting paths must produce nothing useful.
#[test]
fn neither_formatting_path_leaks_the_value() {
    let value = Sensitive::new(QUERY.to_string());
    let rendered = format!("{value} {value:?} {:?}", vec![&value]);
    assert!(!rendered.contains("migraines"), "{rendered}");
    assert!(!rendered.contains("collision"), "{rendered}");
}

/// A struct that derives `Debug` must not leak through its own derive.
#[test]
fn a_derived_debug_on_a_containing_struct_stays_redacted() {
    // Read only through the derived `Debug`, which dead-code analysis
    // deliberately ignores.
    #[allow(dead_code)]
    #[derive(Debug)]
    struct SearchRequest {
        client_id: u32,
        query: Sensitive<String>,
    }

    let req = SearchRequest {
        client_id: 7,
        query: Sensitive::new(QUERY.to_string()),
    };
    let rendered = format!("{req:?}");
    assert!(rendered.contains("client_id: 7"), "{rendered}");
    assert!(rendered.contains("[redacted]"), "{rendered}");
    assert!(!rendered.contains("migraines"), "{rendered}");
}

#[test]
fn reveal_is_the_escape_hatch() {
    let value = Sensitive::new(QUERY.to_string());
    assert_eq!(value.reveal(), QUERY);
    assert_eq!(value.reveal_into(), QUERY);
}

/// A length is not PHI, and is often all an operator needs.
#[test]
fn a_length_can_be_rendered_without_revealing_the_value() {
    let value = Sensitive::new(QUERY.to_string());
    assert_eq!(
        value.redacted_with_len(),
        format!("[redacted; {} chars]", QUERY.chars().count())
    );
    assert!(!value.redacted_with_len().contains("migraines"));
}

/// Serialization is transparent: putting a field behind `Sensitive` must not
/// change the JSON, so an audit event's `phi` payload reaches S3 in full.
#[test]
fn serialization_is_transparent() {
    let value = Sensitive::new(QUERY.to_string());
    let encoded = serde_json::to_string(&value).expect("serialize");
    assert_eq!(
        encoded,
        serde_json::to_string(QUERY).expect("serialize str")
    );

    let decoded: Sensitive<String> = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded.reveal(), QUERY);
}

#[test]
fn it_round_trips_inside_a_struct_without_changing_the_wire_form() {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Phi {
        query: Sensitive<String>,
        result_count: u32,
    }

    let encoded = serde_json::to_string(&Phi {
        query: Sensitive::new(QUERY.to_string()),
        result_count: 3,
    })
    .expect("serialize");

    // Indistinguishable from a plain `String` field on the wire.
    assert_eq!(
        encoded,
        format!(r#"{{"query":"{QUERY}","result_count":3}}"#)
    );

    let decoded: Phi = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded.query.reveal(), QUERY);
    assert_eq!(decoded.result_count, 3);
}

#[test]
fn map_keeps_the_result_wrapped() {
    let value = Sensitive::new(QUERY.to_string());
    let upper = value.map(|s| s.to_uppercase());
    assert_eq!(format!("{upper}"), "[redacted]");
    assert_eq!(upper.reveal(), &QUERY.to_uppercase());
}
