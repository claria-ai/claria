use tauri::State;

use claria_provisioner::plan::ExecutionPlan;
use claria_provisioner::state::ProvisionerState;

use crate::state::{AwsConfig, DesktopState};

async fn build_clients(
    config: &AwsConfig,
) -> Result<
    (
        aws_sdk_s3::Client,
        Vec<Box<dyn claria_provisioner::resource::Resource>>,
    ),
    String,
> {
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(config.region.clone()))
        .load()
        .await;

    let s3 = aws_sdk_s3::Client::new(&aws_config);

    let resources: Vec<Box<dyn claria_provisioner::resource::Resource>> = vec![
        Box::new(claria_provisioner::resources::s3_bucket::S3BucketResource::new(
            s3.clone(),
            config.bucket.clone(),
        )),
        Box::new(claria_provisioner::resources::iam::IamRoleResource::new(
            aws_sdk_iam::Client::new(&aws_config),
            format!("{}-lambda-role", config.bucket),
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [{
                    "Effect": "Allow",
                    "Principal": { "Service": "lambda.amazonaws.com" },
                    "Action": "sts:AssumeRole"
                }]
            })
            .to_string(),
        )),
        Box::new(claria_provisioner::resources::cognito::CognitoResource::new(
            aws_sdk_cognitoidentityprovider::Client::new(&aws_config),
            format!("{}-users", config.bucket),
        )),
        Box::new(claria_provisioner::resources::lambda::LambdaResource::new(
            aws_sdk_lambda::Client::new(&aws_config),
            format!("{}-api", config.bucket),
            format!("arn:aws:iam::role/{}-lambda-role", config.bucket),
        )),
        Box::new(claria_provisioner::resources::api_gateway::ApiGatewayResource::new(
            aws_sdk_apigatewayv2::Client::new(&aws_config),
            format!("{}-api", config.bucket),
        )),
        Box::new(claria_provisioner::resources::bedrock_access::BedrockAccessResource::new(
            aws_sdk_bedrock::Client::new(&aws_config),
            vec!["anthropic.claude".to_string()],
        )),
        Box::new(claria_provisioner::resources::cloudtrail::CloudTrailResource::new(
            aws_sdk_cloudtrail::Client::new(&aws_config),
            format!("{}-audit", config.bucket),
            config.bucket.clone(),
        )),
    ];

    Ok((s3, resources))
}

async fn require_config(state: &DesktopState) -> Result<AwsConfig, String> {
    state
        .config
        .lock()
        .await
        .clone()
        .ok_or_else(|| "not configured: call configure() first".to_string())
}

#[tauri::command]
pub async fn configure(
    state: State<'_, DesktopState>,
    region: String,
    bucket: String,
) -> Result<(), String> {
    let mut config = state.config.lock().await;
    *config = Some(AwsConfig { region, bucket });
    Ok(())
}

#[tauri::command]
pub async fn get_state(state: State<'_, DesktopState>) -> Result<ProvisionerState, String> {
    let config = require_config(&state).await?;
    let (s3, _resources) = build_clients(&config).await?;

    claria_provisioner::load_provisioner_state(&s3, &config.bucket)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preview_plan(state: State<'_, DesktopState>) -> Result<ExecutionPlan, String> {
    let config = require_config(&state).await?;
    let (s3, resources) = build_clients(&config).await?;

    let pstate = claria_provisioner::load_provisioner_state(&s3, &config.bucket)
        .await
        .map_err(|e| e.to_string())?;

    claria_provisioner::drift::detect_drift(&pstate, &resources)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn provision(state: State<'_, DesktopState>) -> Result<ProvisionerState, String> {
    let config = require_config(&state).await?;
    let (s3, resources) = build_clients(&config).await?;

    claria_provisioner::provision(&s3, &config.bucket, &resources)
        .await
        .map_err(|e| e.to_string())?;

    claria_provisioner::load_provisioner_state(&s3, &config.bucket)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn destroy(state: State<'_, DesktopState>) -> Result<(), String> {
    let config = require_config(&state).await?;
    let (s3, resources) = build_clients(&config).await?;

    claria_provisioner::destroy(&s3, &config.bucket, &resources)
        .await
        .map_err(|e| e.to_string())
}
