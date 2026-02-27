use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, Message, SystemContentBlock,
};
use tracing::info;
use uuid::Uuid;

use claria_core::models::anonymize::AnonymizationResult;
use claria_core::models::answer::SchematizedAnswer;
use claria_core::models::token_count::TokenUsage;
use claria_core::models::transaction::{TransactionStatus, TransactionType};

use crate::error::BedrockError;
use crate::tokens;

/// The result of a Bedrock transaction, before it is persisted.
pub struct TransactionResult<T> {
    pub id: Uuid,
    pub transaction_type: TransactionType,
    pub model_id: String,
    pub usage: TokenUsage,
    pub status: TransactionStatus,
    pub output: T,
}

/// Invoke Bedrock for report generation.
///
/// Sends the assembled inputs with a system prompt instructing the model
/// to return a `SchematizedAnswer` as JSON.
pub async fn generate_report(
    client: &Client,
    model_id: &str,
    system_prompt: &str,
    user_message: &str,
) -> Result<TransactionResult<SchematizedAnswer>, BedrockError> {
    let transaction_id = Uuid::new_v4();
    info!(transaction_id = %transaction_id, model = model_id, "starting report generation");

    let (response_text, usage) = invoke_converse(client, model_id, system_prompt, user_message).await?;

    let answer: SchematizedAnswer = serde_json::from_str(&response_text)
        .map_err(|e| BedrockError::SchemaViolation(format!(
            "failed to parse SchematizedAnswer: {e}. Response: {response_text}"
        )))?;

    info!(transaction_id = %transaction_id, "report generation complete");

    Ok(TransactionResult {
        id: transaction_id,
        transaction_type: TransactionType::ReportGeneration,
        model_id: model_id.to_string(),
        usage,
        status: TransactionStatus::Complete,
        output: answer,
    })
}

/// Invoke Bedrock for document anonymization.
///
/// Sends the document with a system prompt instructing the model
/// to identify and replace PII, returning an `AnonymizationResult`.
pub async fn anonymize_document(
    client: &Client,
    model_id: &str,
    system_prompt: &str,
    document_text: &str,
) -> Result<TransactionResult<AnonymizationResult>, BedrockError> {
    let transaction_id = Uuid::new_v4();
    info!(transaction_id = %transaction_id, model = model_id, "starting anonymization");

    let (response_text, usage) = invoke_converse(client, model_id, system_prompt, document_text).await?;

    let mut result: AnonymizationResult = serde_json::from_str(&response_text)
        .map_err(|e| BedrockError::SchemaViolation(format!(
            "failed to parse AnonymizationResult: {e}. Response: {response_text}"
        )))?;

    // Ensure the transaction ID in the result matches
    result.transaction_id = transaction_id;

    info!(transaction_id = %transaction_id, "anonymization complete");

    Ok(TransactionResult {
        id: transaction_id,
        transaction_type: TransactionType::Anonymization,
        model_id: model_id.to_string(),
        usage,
        status: TransactionStatus::Complete,
        output: result,
    })
}

/// Core invocation using the Bedrock Converse API.
/// Returns the response text and token usage.
async fn invoke_converse(
    client: &Client,
    model_id: &str,
    system_prompt: &str,
    user_message: &str,
) -> Result<(String, TokenUsage), BedrockError> {
    let pricing = tokens::get_pricing(model_id);

    let response = client
        .converse()
        .model_id(model_id)
        .system(SystemContentBlock::Text(system_prompt.to_string()))
        .messages(
            Message::builder()
                .role(ConversationRole::User)
                .content(ContentBlock::Text(user_message.to_string()))
                .build()
                .map_err(|e| BedrockError::Invocation(e.to_string()))?,
        )
        .send()
        .await
        .map_err(|e| BedrockError::Invocation(e.into_service_error().to_string()))?;

    // Extract response text
    let output_message = response
        .output()
        .and_then(|o| o.as_message().ok())
        .ok_or_else(|| BedrockError::ResponseParse("no message in response".to_string()))?;

    let response_text = output_message
        .content()
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text(text) = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    // Extract token usage
    let usage = response
        .usage()
        .map(|u| {
            let token_count = tokens::extract_token_usage(u);
            if let Some(p) = &pricing {
                tokens::calculate_cost(token_count, p)
            } else {
                TokenUsage {
                    tokens: token_count,
                    cost_usd: 0.0,
                }
            }
        })
        .unwrap_or(TokenUsage {
            tokens: claria_core::models::token_count::TokenCount {
                input: 0,
                output: 0,
            },
            cost_usd: 0.0,
        });

    Ok((response_text, usage))
}
