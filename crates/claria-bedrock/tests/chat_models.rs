//! Integration tests for chat model discovery.
//!
//! These tests call real AWS APIs and require valid credentials in the
//! environment (e.g. `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`).
//!
//! Run with: `cargo test -p claria-bedrock --test chat_models -- --ignored`

use claria_bedrock::chat::list_chat_models;

async fn build_config() -> aws_config::SdkConfig {
    aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new("us-east-1"))
        .load()
        .await
}

/// Diagnostic test: call the raw AWS APIs directly to see what we get back.
#[tokio::test]
#[ignore]
async fn diagnostic_raw_api_calls() {
    let config = build_config().await;
    let client = aws_sdk_bedrock::Client::new(&config);

    println!("=== ListFoundationModels (anthropic) ===");
    match client
        .list_foundation_models()
        .by_provider("anthropic")
        .send()
        .await
    {
        Ok(resp) => {
            for m in resp.model_summaries() {
                let lifecycle = m
                    .model_lifecycle()
                    .map(|lc| format!("{:?}", lc.status()))
                    .unwrap_or_else(|| "NONE".to_string());
                println!("  {} — lifecycle: {}", m.model_id(), lifecycle);
            }
        }
        Err(e) => {
            println!("  ERROR: {e:?}");
        }
    }

    println!("\n=== ListInferenceProfiles (system-defined, Claude only) ===");
    match client
        .list_inference_profiles()
        .type_equals(aws_sdk_bedrock::types::InferenceProfileType::SystemDefined)
        .max_results(100)
        .send()
        .await
    {
        Ok(resp) => {
            for p in resp.inference_profile_summaries() {
                let id = p.inference_profile_id();
                if id.contains("anthropic.claude") {
                    println!(
                        "  {} — {} — status: {:?}",
                        id,
                        p.inference_profile_name(),
                        p.status(),
                    );
                }
            }
        }
        Err(e) => {
            println!("  ERROR: {e:?}");
        }
    }
}

/// All model IDs must be inference profile IDs (prefixed with `us.`), not
/// bare foundation model IDs.
#[tokio::test]
#[ignore]
async fn list_chat_models_all_have_us_prefix() {
    let config = build_config().await;
    let models = list_chat_models(&config).await.expect("list_chat_models should succeed");

    for m in &models {
        assert!(
            m.model_id.starts_with("us."),
            "model ID should start with 'us.' but got: {}",
            m.model_id
        );
    }
}

/// Opus 4.6 must appear as a `us.` inference profile.
#[tokio::test]
#[ignore]
async fn list_chat_models_includes_opus_4_6() {
    let config = build_config().await;
    let models = list_chat_models(&config).await.expect("list_chat_models should succeed");

    println!("Discovered {} models:", models.len());
    for m in &models {
        println!("  {} — {}", m.model_id, m.name);
    }

    assert!(
        models
            .iter()
            .any(|m| m.model_id.contains("claude-opus-4-6")),
        "expected Opus 4.6 in model list, got: {:?}",
        models.iter().map(|m| &m.model_id).collect::<Vec<_>>()
    );
}

/// Sonnet 4.6 must appear.
#[tokio::test]
#[ignore]
async fn list_chat_models_includes_sonnet_4_6() {
    let config = build_config().await;
    let models = list_chat_models(&config).await.expect("list_chat_models should succeed");

    assert!(
        models
            .iter()
            .any(|m| m.model_id.contains("claude-sonnet-4-6")),
        "expected Sonnet 4.6 in model list, got: {:?}",
        models.iter().map(|m| &m.model_id).collect::<Vec<_>>()
    );
}

/// Claude 3 Opus is absent from the foundation model registry and must not
/// appear in the results.
#[tokio::test]
#[ignore]
async fn list_chat_models_excludes_legacy_opus_3() {
    let config = build_config().await;
    let models = list_chat_models(&config).await.expect("list_chat_models should succeed");

    assert!(
        !models.iter().any(|m| m.model_id.contains("claude-3-opus")),
        "Claude 3 Opus should not appear, but found it in: {:?}",
        models.iter().map(|m| &m.model_id).collect::<Vec<_>>()
    );
}

/// No legacy models should appear (e.g. Claude 3 Sonnet, 3.5 Sonnet, 3.7 Sonnet).
#[tokio::test]
#[ignore]
async fn list_chat_models_excludes_legacy_models() {
    let config = build_config().await;
    let models = list_chat_models(&config).await.expect("list_chat_models should succeed");

    let legacy_fragments = [
        "claude-3-sonnet",
        "claude-3-5-sonnet",
        "claude-3-7-sonnet",
        "claude-opus-4-20250514", // original Opus 4 is now LEGACY
    ];

    for fragment in &legacy_fragments {
        assert!(
            !models.iter().any(|m| m.model_id.contains(fragment)),
            "legacy model containing '{fragment}' should not appear, got: {:?}",
            models.iter().map(|m| &m.model_id).collect::<Vec<_>>()
        );
    }
}
