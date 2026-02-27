use std::future::Future;
use std::pin::Pin;

use aws_sdk_lambda::Client;

use crate::error::ProvisionerError;
use crate::resource::{Resource, ResourceResult};

type Bf<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct LambdaResource {
    client: Client,
    function_name: String,
    role_arn: String,
}

impl LambdaResource {
    pub fn new(client: Client, function_name: String, role_arn: String) -> Self {
        Self {
            client,
            function_name,
            role_arn,
        }
    }
}

impl Resource for LambdaResource {
    fn resource_type(&self) -> &str {
        "lambda_function"
    }

    fn current_state(&self) -> Bf<'_, Result<Option<serde_json::Value>, ProvisionerError>> {
        Box::pin(async {
            match self
                .client
                .get_function()
                .function_name(&self.function_name)
                .send()
                .await
            {
                Ok(resp) => {
                    let config = resp.configuration();
                    Ok(Some(serde_json::json!({
                        "function_name": self.function_name,
                        "function_arn": config.map(|c| c.function_arn().unwrap_or_default()),
                        "runtime": config.and_then(|c| c.runtime().map(|r| r.as_str())),
                    })))
                }
                Err(_) => Ok(None),
            }
        })
    }

    fn create(&self) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        Box::pin(async {
            let result = self
                .client
                .create_function()
                .function_name(&self.function_name)
                .runtime(aws_sdk_lambda::types::Runtime::Providedal2023)
                .role(&self.role_arn)
                .handler("bootstrap")
                .code(
                    aws_sdk_lambda::types::FunctionCode::builder()
                        .zip_file(aws_sdk_lambda::primitives::Blob::new(Vec::new()))
                        .build(),
                )
                .send()
                .await
                .map_err(|e| ProvisionerError::CreateFailed(e.to_string()))?;

            let function_arn = result.function_arn().unwrap_or_default().to_string();

            self.client
                .put_function_concurrency()
                .function_name(&self.function_name)
                .reserved_concurrent_executions(1)
                .send()
                .await
                .map_err(|e| ProvisionerError::UpdateFailed(e.to_string()))?;

            tracing::info!(
                function_name = %self.function_name,
                function_arn = %function_arn,
                "Lambda function created with reserved concurrency = 1"
            );

            Ok(ResourceResult {
                resource_id: function_arn.clone(),
                properties: serde_json::json!({
                    "function_name": self.function_name,
                    "function_arn": function_arn,
                }),
            })
        })
    }

    fn update(&self, resource_id: &str) -> Bf<'_, Result<ResourceResult, ProvisionerError>> {
        let rid = resource_id.to_string();
        let fname = self.function_name.clone();
        Box::pin(async move {
            Ok(ResourceResult {
                resource_id: rid,
                properties: serde_json::json!({ "function_name": fname }),
            })
        })
    }

    fn delete(&self, _resource_id: &str) -> Bf<'_, Result<(), ProvisionerError>> {
        Box::pin(async {
            self.client
                .delete_function()
                .function_name(&self.function_name)
                .send()
                .await
                .map_err(|e| ProvisionerError::DeleteFailed(e.to_string()))?;
            tracing::info!(function_name = %self.function_name, "Lambda function deleted");
            Ok(())
        })
    }
}
