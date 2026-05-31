//! SDK-level integration tests: drive `claria_bedrock::agreements` against the
//! in-process mock AWS server over real HTTP. These exercise the exact wire
//! contract (URIs, request/response bodies, the FTU base64 blob) that
//! router-level mock tests can't.

use aws_credential_types::provider::SharedCredentialsProvider;
use aws_credential_types::Credentials;

use claria_bedrock::agreements::{self, EnrollmentStatus, UseCaseForm};
use claria_bedrock::error::{BedrockError, ModelAccessReason};
use claria_mock_aws::testing::MockServer;

const OPUS: &str = "anthropic.claude-opus-4-6-20260301-v1:0";

fn build_sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let creds = Credentials::new(
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        None,
        None,
        "claria-test",
    );
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(creds))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn load_scenario(server: &MockServer, name: &str) {
    let mut st = server.state.write().await;
    claria_mock_aws::scenarios::load(name, &mut st).expect("scenario");
}

fn sample_form() -> UseCaseForm {
    UseCaseForm {
        company_name: "Acme Health".to_string(),
        company_website: "https://acme.example".to_string(),
        intended_users: 0,
        industry_option: "Healthcare".to_string(),
        other_industry_option: None,
        use_cases: "Clinical note drafting".to_string(),
    }
}

/// Fresh-ish account with the FTU form not submitted: the form reads as
/// unsubmitted and available models surface as `UseCaseFormRequired`.
#[tokio::test]
async fn ftu_required_gates_enrollment() {
    let server = MockServer::spawn().await;
    load_scenario(&server, "ftu-required").await;
    let sdk = build_sdk_config(&server.endpoint);

    let form = agreements::get_use_case_form(&sdk).await.expect("get form");
    assert!(!form.submitted);

    let enrollments = agreements::list_enrollments(&sdk).await.expect("list");
    let opus = enrollments.iter().find(|m| m.model_id == OPUS).expect("opus");
    assert_eq!(opus.status, EnrollmentStatus::UseCaseFormRequired);

    // Executing before the form is filled maps to a structured ModelAccess error.
    match agreements::execute_agreement(&sdk, OPUS).await {
        Err(BedrockError::ModelAccess { reason, .. }) => {
            assert_eq!(reason, ModelAccessReason::UseCaseFormRequired);
        }
        other => panic!("expected ModelAccess error, got {other:?}"),
    }
}

/// Submitting the FTU form round-trips through the base64 blob and unblocks
/// enrollment.
#[tokio::test]
async fn submit_form_then_enroll_and_execute() {
    let server = MockServer::spawn().await;
    load_scenario(&server, "ftu-required").await;
    let sdk = build_sdk_config(&server.endpoint);

    agreements::submit_use_case_form(&sdk, &sample_form())
        .await
        .expect("submit");

    let form = agreements::get_use_case_form(&sdk).await.expect("get form");
    assert!(form.submitted);
    assert_eq!(form.form.as_ref().map(|f| f.company_name.as_str()), Some("Acme Health"));

    // Now models are Available with terms + pricing.
    let enrollments = agreements::list_enrollments(&sdk).await.expect("list");
    let opus = enrollments.iter().find(|m| m.model_id == OPUS).expect("opus");
    assert_eq!(opus.status, EnrollmentStatus::Available);
    let offer = opus.offer.as_ref().expect("offer");
    assert!(offer.legal_terms_url.is_some());
    assert_eq!(offer.pricing.len(), 2);

    // Execute → Pending; promote → Executed.
    agreements::execute_agreement(&sdk, OPUS).await.expect("execute");
    let pending = agreements::get_enrollment(&sdk, OPUS).await.expect("get");
    assert_eq!(pending.status, EnrollmentStatus::Pending);

    {
        let mut st = server.state.write().await;
        st.model_agreement_status.insert(OPUS.to_string(), "EXECUTED".to_string());
    }

    let executed = agreements::get_enrollment(&sdk, OPUS).await.expect("get");
    assert_eq!(executed.status, EnrollmentStatus::Executed);
}

/// A fully-provisioned account reports every model as enrolled.
#[tokio::test]
async fn fully_provisioned_models_executed() {
    let server = MockServer::spawn().await;
    load_scenario(&server, "fully-provisioned").await;
    let sdk = build_sdk_config(&server.endpoint);

    let enrollments = agreements::list_enrollments(&sdk).await.expect("list");
    assert!(!enrollments.is_empty());
    assert!(enrollments.iter().all(|m| m.status == EnrollmentStatus::Executed));
}
