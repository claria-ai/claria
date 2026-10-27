//! A bounded, local-only gate in front of the existing mock S3 service.
//! Holds one selected PUT before forwarding it; storage semantics stay in the mock.

use std::sync::{Arc, Mutex};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};

pub struct S3Gate {
    pub endpoint: String,
    reached: oneshot::Receiver<String>,
    release: Option<oneshot::Sender<()>>,
    server: JoinHandle<()>,
}

impl S3Gate {
    pub async fn spawn(
        endpoint: &str,
        matches: impl Fn(&str, &[u8]) -> bool + Send + Sync + 'static,
    ) -> Self {
        let backend = endpoint
            .strip_prefix("http://")
            .expect("local HTTP mock")
            .to_owned();
        assert!(backend.starts_with("127.0.0.1:"), "only loopback fixtures");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind gate");
        let endpoint = format!("http://{}", listener.local_addr().expect("gate address"));
        let (reached_tx, reached) = oneshot::channel();
        let (release, release_rx) = oneshot::channel();
        let gate = Arc::new(Mutex::new(Some((reached_tx, release_rx))));
        let matches = Arc::new(matches);
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept(), if connections.len() < 8 => {
                        let (stream, _) = result.expect("accept local S3 connection");
                        let backend = backend.clone();
                        let gate = gate.clone();
                        let matches = matches.clone();
                        connections.spawn(async move {
                            let (mut stream, headers, body, method, path) = request(stream).await;
                            let selected = if method == "PUT" && matches(&path, &body) {
                                gate.lock().expect("gate mutex").take()
                            } else {
                                None
                            };
                            if let Some((reached, release)) = selected {
                                reached.send(path).expect("test waits for held PUT");
                                if release.await.is_err() {
                                    return;
                                }
                            }
                            let mut upstream = TcpStream::connect(backend).await.expect("mock S3");
                            upstream.write_all(headers.as_bytes()).await.expect("forward headers");
                            upstream.write_all(&body).await.expect("forward body");
                            // Closing each upstream connection ensures every SDK request
                            // crosses the gate instead of reusing an uninspected socket.
                            let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
                        });
                    }
                    result = connections.join_next(), if !connections.is_empty() => {
                        result.expect("connection result").expect("gate connection task");
                    }
                }
            }
        });
        Self {
            endpoint,
            reached,
            release: Some(release),
            server,
        }
    }

    pub async fn held(&mut self) -> String {
        (&mut self.reached)
            .await
            .expect("selected PUT reaches gate")
    }

    pub fn release(&mut self) {
        self.release
            .take()
            .expect("release once")
            .send(())
            .expect("held PUT still alive");
    }
}

impl Drop for S3Gate {
    fn drop(&mut self) {
        self.server.abort(); // Dropping its JoinSet also aborts all connection tasks.
    }
}

async fn request(mut stream: TcpStream) -> (TcpStream, String, Vec<u8>, String, String) {
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.expect("read S3 request");
        assert!(read > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() <= MAX_BYTES, "fixture request bound");
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).expect("HTTP headers");
    let first = headers.lines().next().expect("request line");
    let mut fields = first.split_whitespace();
    let method = fields.next().expect("method").to_string();
    let path = fields.next().expect("target").to_string();
    let mut forwarded = format!("{first}\r\n");
    let mut length = 0;
    for line in headers.lines().skip(1).filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').expect("header");
        assert!(
            !name.eq_ignore_ascii_case("transfer-encoding"),
            "fixture uses known body lengths"
        );
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse::<usize>().expect("body length");
        }
        if !name.eq_ignore_ascii_case("connection") {
            forwarded.push_str(line);
            forwarded.push_str("\r\n");
        }
    }
    forwarded.push_str("Connection: close\r\n\r\n");
    assert!(header_end + length <= MAX_BYTES, "fixture request bound");
    while bytes.len() < header_end + length {
        let read = stream.read(&mut chunk).await.expect("read S3 body");
        assert!(read > 0, "request ended before body");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() <= MAX_BYTES, "fixture request bound");
    }
    assert_eq!(
        bytes.len(),
        header_end + length,
        "one request per connection"
    );
    (
        stream,
        forwarded,
        bytes[header_end..].to_vec(),
        method,
        path,
    )
}
