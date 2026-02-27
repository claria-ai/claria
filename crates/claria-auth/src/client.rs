use aws_sdk_cognitoidentityprovider::Client;

/// Build a Cognito Identity Provider client from the default AWS config.
pub async fn build_client() -> Client {
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    Client::new(&config)
}

/// Build a Cognito Identity Provider client with a specific region.
pub async fn build_client_with_region(region: &str) -> Client {
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .load()
        .await;
    Client::new(&config)
}
