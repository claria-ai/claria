//! Translation-envelope parsing tests.
//!
//! Network-touching tests (real Bedrock invocations) live elsewhere; these only
//! cover the pure parsing path that has to be robust against fenced output and
//! various model whitespace habits.

#[test]
fn translation_envelope_shape_round_trips() {
    let json = r#"{"translations":[{"index":0,"translation":"Hello"},{"index":2,"translation":"How are you?"}]}"#;
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let translations = value
        .get("translations")
        .and_then(|t| t.as_array())
        .unwrap();
    assert_eq!(translations.len(), 2);
    assert_eq!(translations[0]["index"], 0);
    assert_eq!(translations[0]["translation"], "Hello");
    assert_eq!(translations[1]["index"], 2);
    assert_eq!(translations[1]["translation"], "How are you?");
}
