use aws_sdk_s3::Client;

/// Build an S3 client from an existing `SdkConfig`.
///
/// When a custom `endpoint_url` is set (e.g. for local mock services),
/// enables path-style addressing so requests go to
/// `http://host:port/bucket/key` instead of `http://bucket.host:port/key`.
pub fn from_config(config: &aws_config::SdkConfig) -> Client {
    let mut builder = aws_sdk_s3::config::Builder::from(config);
    if config.endpoint_url().is_some() {
        builder = builder.force_path_style(true);
    }
    Client::from_conf(builder.build())
}
