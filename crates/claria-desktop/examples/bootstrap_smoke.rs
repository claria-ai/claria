//! Smoke test for the account setup flow.
//!
//! Reads credentials from environment variables, classifies them via the
//! provisioner, and (if root or admin) runs the full IAM bootstrap.
//!
//! Usage:
//!   AWS_ACCESS_KEY_ID=AKIA... \
//!   AWS_SECRET_ACCESS_KEY=... \
//!   AWS_REGION=us-east-1 \
//!   CLARIA_SYSTEM_NAME=claria \
//!   cargo run -p claria-desktop --example bootstrap_smoke

use claria_desktop::aws;
use claria_desktop::config::CredentialSource;
use claria_provisioner::account_setup::{self, CredentialClass, StepStatus};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
        .map_err(|_| eyre::eyre!("set AWS_ACCESS_KEY_ID env var"))?;
    let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
        .map_err(|_| eyre::eyre!("set AWS_SECRET_ACCESS_KEY env var"))?;
    let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
    let system_name =
        std::env::var("CLARIA_SYSTEM_NAME").unwrap_or_else(|_| "claria".to_string());

    let key_hint = format!(
        "{}...{}",
        &access_key_id[..4],
        &access_key_id[access_key_id.len() - 4..]
    );

    println!("╔══════════════════════════════════════════════════╗");
    println!("║      Claria Account Setup — Smoke Test           ║");
    println!("╠══════════════════════════════════════════════════╣");
    println!("║  Region:      {:<34} ║", region);
    println!("║  System name: {:<34} ║", system_name);
    println!("║  Access key:  {:<34} ║", key_hint);
    println!("╚══════════════════════════════════════════════════╝");
    println!();

    // Build an SDK config from the provided credentials.
    let creds = CredentialSource::Inline {
        access_key_id: access_key_id.clone(),
        secret_access_key: secret_access_key.clone(),
        session_token: None,
    };
    let sdk_config = aws::build_aws_config(&region, &creds).await;

    // Step 1: Assess credentials via the provisioner.
    println!("Assessing credentials...");
    let assessment = account_setup::assess_credentials(&sdk_config).await?;

    println!("  Account:  {}", assessment.identity.account_id);
    println!("  ARN:      {}", assessment.identity.arn);
    println!("  Is root:  {}", assessment.identity.is_root);
    println!("  Class:    {:?}", assessment.credential_class);
    println!("  Reason:   {}", assessment.reason);
    println!();

    match assessment.credential_class {
        CredentialClass::Root | CredentialClass::IamAdmin => {
            let label = match assessment.credential_class {
                CredentialClass::Root => "root",
                CredentialClass::IamAdmin => "IAM admin",
                _ => unreachable!(),
            };

            println!(
                "Detected {label} credentials. Running bootstrap to create \
                 a scoped IAM user..."
            );

            if assessment.credential_class != CredentialClass::Root {
                println!();
                println!(
                    "⚠  These are admin (not root) credentials. The bootstrap will");
                println!(
                    "   create a scoped user but will NOT delete your current key.");
                println!();
                println!("   Press Enter to continue, or Ctrl-C to abort.");
                let mut buf = String::new();
                std::io::stdin().read_line(&mut buf)?;
            }

            println!();

            let result = account_setup::bootstrap_account(
                &sdk_config,
                &system_name,
                &access_key_id,
                assessment.credential_class,
            )
            .await;

            // Print step-by-step results.
            for step in &result.steps {
                let icon = match step.status {
                    StepStatus::Succeeded => "✅",
                    StepStatus::Failed => "❌",
                    StepStatus::InProgress => "⏳",
                    StepStatus::Pending => "⬜",
                };
                print!("  {icon} {}", step.name);
                if let Some(detail) = &step.detail {
                    print!("  — {detail}");
                }
                println!();
            }
            println!();

            if result.success {
                println!("✅ Bootstrap succeeded!");
                if let Some(acct) = &result.account_id {
                    println!("   Account ID: {acct}");
                }
                if let Some(new_creds) = &result.new_credentials {
                    println!("   IAM user:   {}", new_creds.iam_user_arn);
                    println!("   New key:    {}...", &new_creds.access_key_id[..8]);
                }
                if assessment.credential_class == CredentialClass::Root {
                    println!("   Root access key deleted from AWS.");
                }
                println!();
                println!(
                    "   Note: this smoke test does NOT write config to disk."
                );
                println!(
                    "   In the real app, the desktop controller would persist"
                );
                println!("   the new credentials now.");
            } else {
                println!("❌ Bootstrap failed.");
                if let Some(err) = &result.error {
                    println!("   Error: {err}");
                }
                println!();
                println!(
                    "   Review the steps above. You may need to clean up"
                );
                println!(
                    "   partially-created resources in the IAM console."
                );
            }
        }

        CredentialClass::ScopedClaria => {
            println!("✅ Credentials are already scoped for Claria.");
            println!("   No bootstrap needed — ready for resource provisioning.");
        }

        CredentialClass::Insufficient => {
            println!("❌ Credentials are insufficient for Claria.");
            println!("   {}", assessment.reason);
            println!();
            println!(
                "   Provide root or admin credentials so Claria can create"
            );
            println!("   a properly scoped IAM user.");
        }
    }

    Ok(())
}