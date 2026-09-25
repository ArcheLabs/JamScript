use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

struct MockNode {
    endpoint: String,
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockNode {
    fn start(genesis: [u8; 32]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = Arc::clone(&running);
        let thread = thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => handle_mock_node_request(stream, genesis),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            endpoint,
            running,
            thread: Some(thread),
        }
    }
}

impl Drop for MockNode {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct BackendProcess {
    child: Child,
    bind: String,
}

impl Drop for BackendProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn handle_mock_node_request(mut stream: TcpStream, genesis: [u8; 32]) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else {
        return;
    };
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return;
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        return;
    };
    let result = match request.get("method").and_then(Value::as_str) {
        Some("chain_getBlockHash") => json!(format_hash(&genesis)),
        Some("minijam_getFinalizedContext") => json!({
            "blockHash": format_hash(&[0x11; 32]),
            "blockNumber": 7,
            "stateRoot": format_hash(&[0x22; 32]),
            "slot": 9
        }),
        _ => Value::Null,
    };
    let response = json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(Value::Null),
        "result": result,
    })
    .to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        response.len(),
        response
    );
}

fn format_hash(bytes: &[u8; 32]) -> String {
    let mut output = String::from("0x");
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_backend(node_rpc: &str, data_dir: &std::path::Path) -> BackendProcess {
    let bind = format!("127.0.0.1:{}", free_port());
    let child = Command::new(env!("CARGO_BIN_EXE_jamscript-service-backend"))
        .args([
            "--bind",
            &bind,
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--node-rpc",
            node_rpc,
            "--formal-rpc",
            "http://127.0.0.1:1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut process = BackendProcess { child, bind };
    wait_for_ready(&process.bind, &mut process.child);
    process
}

fn wait_for_ready(bind: &str, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("backend exited before readiness: {status}");
        }
        if let Ok(mut stream) = TcpStream::connect(bind) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
            let _ = stream.write_all(
                b"GET /readinessz HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n",
            );
            let mut response = String::new();
            let _ = stream.read_to_string(&mut response);
            if response.starts_with("HTTP/1.1 200 OK") {
                return;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("backend did not become ready on {bind}");
}

fn backend_genesis(bind: &str) -> Value {
    let mut stream = TcpStream::connect(bind).unwrap();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "chain_getBlockHash",
        "params": [0]
    })
    .to_string();
    write!(
        stream,
        "POST / HTTP/1.1\r\nhost: localhost\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let body = response.split_once("\r\n\r\n").unwrap().1;
    serde_json::from_str(body).unwrap()
}

#[test]
fn one_backend_binary_connects_to_nodes_with_distinct_genesis_identities() {
    for genesis in [[0xaa; 32], [0xbb; 32]] {
        let node = MockNode::start(genesis);
        let directory = tempfile::tempdir().unwrap();
        let backend = start_backend(&node.endpoint, directory.path());
        let response = backend_genesis(&backend.bind);
        assert_eq!(response["result"], format_hash(&genesis));
        drop(backend);
    }
}
