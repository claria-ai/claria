//! Credential selection runs in subprocesses so AWS environment variables and
//! profile files are entirely synthetic, without racing other integration tests.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use aws_credential_types::provider::ProvideCredentials;
use claria_desktop::{aws, config::CredentialSource};

const CASE_ENV: &str = "CLARIA_PROFILE_CONTRACT_CASE";
const CONFIG: &str = "\
[default]
region = us-west-2
[profile selected]
region = eu-west-1
max_attempts = 4
[profile token-only]
sso_session = synthetic
[sso-session synthetic]
sso_region = us-east-1
sso_start_url = https://synthetic.invalid/start
";
const CREDENTIALS: &str = "\
[selected]
aws_access_key_id = synthetic-profile-key
aws_secret_access_key = synthetic-profile-secret
aws_session_token = synthetic-profile-token
[unrelated]
aws_access_key_id = synthetic-unrelated-key
aws_secret_access_key = synthetic-unrelated-secret
";

#[test]
fn selected_profile_beats_environment_credentials() {
    run_isolated("selected", true, 0);
}

#[test]
fn missing_profile_does_not_use_environment_credentials() {
    run_isolated("missing-env", true, 0);
}

#[test]
fn missing_profile_does_not_fall_back_to_container_credentials() {
    run_isolated("missing-container", false, 0);
}

#[test]
fn token_only_profile_does_not_fall_back_to_container_credentials() {
    run_isolated("token-only", false, 0);
}

#[test]
fn default_chain_still_uses_environment_credentials() {
    run_isolated("default-env", true, 0);
}

#[test]
fn default_chain_still_reaches_container_credentials() {
    run_isolated("default-container", false, 1);
}

#[test]
fn inline_credentials_still_override_environment_and_profiles() {
    run_isolated("inline", true, 0);
}

#[test]
fn selected_profile_signs_bedrock_despite_ambient_bearer_token() {
    run_isolated("selected-bearer", true, 0);
}

#[test]
fn missing_profile_cannot_call_bedrock_with_ambient_bearer_token() {
    run_isolated("missing-bearer", false, 0);
}

#[test]
fn default_chain_keeps_ambient_bedrock_bearer_behavior() {
    run_isolated("default-bearer", true, 0);
}

#[test]
fn inline_credentials_keep_ambient_bedrock_bearer_behavior() {
    run_isolated("inline-bearer", true, 0);
}

#[cfg(unix)]
#[test]
fn selected_profile_keeps_credential_process_lazy() {
    run_isolated("process", true, 0);
}

fn run_isolated(case: &str, environment_credentials: bool, expected_container_requests: usize) {
    let fixture = tempfile::tempdir().expect("synthetic profile directory");
    let config_path = fixture.path().join("config");
    let credentials_path = fixture.path().join("credentials");
    let process_marker = fixture.path().join("process-called");
    #[cfg(unix)]
    let mut config = CONFIG.to_string();
    #[cfg(not(unix))]
    let config = CONFIG.to_string();

    #[cfg(unix)]
    if case == "process" {
        let script_path = fixture.path().join("credential-process.sh");
        std::fs::write(
            &script_path,
            "printf called > \"$CLARIA_PROFILE_PROCESS_MARKER\"\n\
             printf '%s\\n' '{\"Version\":1,\"AccessKeyId\":\"synthetic-process-key\",\"SecretAccessKey\":\"synthetic-process-secret\",\"SessionToken\":\"synthetic-process-token\"}'\n",
        )
        .expect("synthetic credential process");
        config.push_str(&format!(
            "\n[profile process]\ncredential_process = /bin/sh '{}'\n",
            script_path.display()
        ));
    }

    std::fs::write(&config_path, config).expect("synthetic AWS config");
    std::fs::write(&credentials_path, CREDENTIALS).expect("synthetic AWS credentials");
    let container = LoopbackEndpoint::start(
        "200 OK",
        r#"{"AccessKeyId":"synthetic-container-key","SecretAccessKey":"synthetic-container-secret","Token":"synthetic-container-token","Expiration":"2099-01-01T00:00:00Z"}"#,
    );
    let bedrock = LoopbackEndpoint::start("400 Bad Request", r#"{"message":"synthetic refusal"}"#);
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args(["--exact", "profile_contract_child", "--nocapture"])
        .env_clear()
        .env(CASE_ENV, case)
        .env("AWS_CONFIG_FILE", &config_path)
        .env("AWS_SHARED_CREDENTIALS_FILE", &credentials_path)
        .env("AWS_PROFILE", "default")
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .env("AWS_CONTAINER_CREDENTIALS_FULL_URI", container.url())
        .env("CLARIA_PROFILE_PROCESS_MARKER", &process_marker);
    if case.ends_with("-bearer") {
        command
            .env("AWS_BEARER_TOKEN_BEDROCK", "synthetic-bearer-token")
            .env("CLARIA_PROFILE_BEDROCK_ENDPOINT", bedrock.url());
    }
    if environment_credentials {
        command
            .env("AWS_ACCESS_KEY_ID", "synthetic-environment-key")
            .env("AWS_SECRET_ACCESS_KEY", "synthetic-environment-secret")
            .env("AWS_SESSION_TOKEN", "synthetic-environment-token");
    }
    let output = command.output().expect("credential contract subprocess");
    assert!(
        output.status.success(),
        "case {case} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        container.requests.load(Ordering::SeqCst),
        expected_container_requests,
        "case {case}: unexpected container credential lookup"
    );
    let authorization = bedrock
        .authorization
        .lock()
        .expect("captured authorization");
    match case {
        "selected-bearer" => {
            assert_eq!(authorization.len(), 1);
            assert!(
                authorization[0].starts_with("AWS4-HMAC-SHA256 Credential=synthetic-profile-key/")
            );
            assert!(authorization[0].contains("/us-east-1/bedrock/aws4_request"));
        }
        "default-bearer" | "inline-bearer" => {
            assert_eq!(&*authorization, &["Bearer synthetic-bearer-token"]);
        }
        _ => assert!(
            authorization.is_empty(),
            "case {case}: unexpected Bedrock request"
        ),
    }
    assert_eq!(bedrock.requests.load(Ordering::SeqCst), authorization.len());
}

#[tokio::test]
async fn profile_contract_child() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let source = match case.as_str() {
        "selected" | "process" | "token-only" => CredentialSource::Profile {
            profile_name: case.clone(),
        },
        "selected-bearer" => CredentialSource::Profile {
            profile_name: "selected".to_string(),
        },
        "missing-env" | "missing-container" | "missing-bearer" => CredentialSource::Profile {
            profile_name: "missing".to_string(),
        },
        "default-env" | "default-container" | "default-bearer" => CredentialSource::DefaultChain,
        "inline" | "inline-bearer" => CredentialSource::Inline {
            access_key_id: "synthetic-inline-key".to_string(),
            secret_access_key: "synthetic-inline-secret".to_string(),
            session_token: Some("synthetic-inline-token".to_string()),
        },
        _ => panic!("unknown credential fixture"),
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let config = aws::build_aws_config("us-east-1", &source).await;
        assert_eq!(config.region().expect("region").as_ref(), "us-east-1");
        let timeouts = config.timeout_config().expect("shared timeouts");
        assert_eq!(timeouts.connect_timeout(), Some(aws::CONNECT_TIMEOUT));
        assert_eq!(timeouts.read_timeout(), Some(aws::READ_TIMEOUT));
        if case == "selected" {
            assert_eq!(
                config
                    .retry_config()
                    .expect("selected profile retries")
                    .max_attempts(),
                4
            );
        }
        let process_marker = std::path::PathBuf::from(
            std::env::var_os("CLARIA_PROFILE_PROCESS_MARKER").expect("process marker path"),
        );
        assert!(
            !process_marker.exists(),
            "configuration loaded credentials eagerly"
        );
        if case.ends_with("-bearer") {
            let endpoint = std::env::var("CLARIA_PROFILE_BEDROCK_ENDPOINT")
                .expect("loopback Bedrock endpoint");
            let config = config.to_builder().endpoint_url(endpoint).build();
            let result = claria_bedrock::translate::translate_segments(
                &config,
                "synthetic-model",
                &[claria_bedrock::translate::TranslationRequest {
                    index: 0,
                    language_code: "es".to_string(),
                    source_text: "synthetic input".to_string(),
                }],
            )
            .await;
            assert!(
                result.is_err(),
                "loopback service always refuses the request"
            );
            return;
        }
        let credentials = config
            .credentials_provider()
            .expect("credential provider")
            .provide_credentials()
            .await;
        if matches!(
            case.as_str(),
            "missing-env" | "missing-container" | "token-only"
        ) {
            assert!(credentials.is_err(), "selected profile must fail closed");
            return;
        }
        let credentials = credentials.expect("synthetic credentials");
        let identity = match case.as_str() {
            "selected" => "profile",
            "default-env" => "environment",
            "default-container" => "container",
            "inline" => "inline",
            "process" => "process",
            _ => unreachable!(),
        };
        assert_eq!(
            credentials.access_key_id(),
            format!("synthetic-{identity}-key")
        );
        assert_eq!(
            credentials.secret_access_key(),
            format!("synthetic-{identity}-secret")
        );
        let token = format!("synthetic-{identity}-token");
        assert_eq!(credentials.session_token(), Some(token.as_str()));
        assert_eq!(process_marker.exists(), case == "process");
    })
    .await
    .expect("credential fixture must finish without external services");
}

/// The positive DefaultChain case proves this fallback is usable. Strict-profile
/// cases must neither return its identity nor contact its loopback endpoint.
struct LoopbackEndpoint {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    authorization: Arc<Mutex<Vec<String>>>,
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LoopbackEndpoint {
    fn start(status: &'static str, body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback credential endpoint");
        let address = listener.local_addr().expect("loopback address");
        let requests = Arc::new(AtomicUsize::new(0));
        let authorization = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let request_count = requests.clone();
        let authorization_headers = authorization.clone();
        let stop = stopping.clone();
        let worker = thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.expect("loopback connection");
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                request_count.fetch_add(1, Ordering::SeqCst);
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                let mut request = Vec::new();
                let header_end = loop {
                    if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        break end + 4;
                    }
                    let mut buffer = [0; 4096];
                    let read = stream.read(&mut buffer).expect("request headers");
                    assert!(
                        read > 0 && request.len() < 64 * 1024,
                        "bounded HTTP headers"
                    );
                    request.extend_from_slice(&buffer[..read]);
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let mut body_length = 0;
                for line in headers.lines() {
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("authorization") {
                            authorization_headers
                                .lock()
                                .expect("authorization capture")
                                .push(value.trim().to_string());
                        }
                        if name.eq_ignore_ascii_case("content-length") {
                            body_length = value.trim().parse::<usize>().expect("body length");
                        }
                    }
                }
                assert!(body_length < 64 * 1024, "bounded fixture body");
                while request.len() < header_end + body_length {
                    let mut buffer = [0; 4096];
                    let read = stream.read(&mut buffer).expect("request body");
                    assert!(read > 0, "complete HTTP body");
                    request.extend_from_slice(&buffer[..read]);
                }
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("synthetic credential response");
            }
        });
        Self {
            address,
            requests,
            authorization,
            stopping,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/credentials", self.address)
    }
}

impl Drop for LoopbackEndpoint {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("credential endpoint worker");
        }
    }
}
