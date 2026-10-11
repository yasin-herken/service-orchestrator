//! End-to-end test for the MCP stdio server (`tasks/014.md` §37).
//!
//! Unlike the adapter unit tests in the crate, this drives the **real binary**
//! (`service-orchestrator mcp`) as a child process over the stdio transport, so
//! it exercises the whole path: transport framing, the `initialize` handshake,
//! capability discovery, tool/resource/prompt listing, tool invocation, the
//! policy engine, the task engine, and graceful shutdown.
//!
//! The binary is launched with an isolated, generated configuration that points
//! at a throwaway temporary workspace, so no real repository, credential, or
//! host is ever touched. Every line the server writes to **stdout** is parsed as
//! JSON, which is exactly the guarantee the transport must uphold: stdout
//! carries only MCP protocol messages.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// The fixture configuration, shared with the adapter unit tests.
const FIXTURE: &str = include_str!("../crates/mcp/tests/fixtures/config.toml");

/// The protocol revision negotiated for the handshake.
const PROTOCOL_VERSION: &str = "2025-11-25";

/// How long any single interaction may take before the test fails.
const DEADLINE: Duration = Duration::from_secs(30);

/// A running `service-orchestrator mcp` child spoken to over stdio.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    next_id: u64,
}

impl Server {
    /// Launches the binary against an isolated, generated configuration.
    fn launch() -> Self {
        let root = std::env::temp_dir().join(format!(
            "so-mcp-e2e-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the clock is after the epoch")
                .as_nanos()
        ));
        // Materialize empty service directories so the scheduled Git task has a
        // real working directory and fails fast (the directory is not a
        // repository), without ever touching a developer's real checkout.
        std::fs::create_dir_all(root.join("backend/orders-api")).expect("create workspace dir");
        std::fs::create_dir_all(root.join("frontend/admin-ui")).expect("create workspace dir");

        let root = root.to_str().expect("the temporary path is valid UTF-8");
        let config = FIXTURE.replace("~/Projects/demo", root);
        let config_path = std::path::Path::new(root).join("config.toml");
        std::fs::write(&config_path, config).expect("write the fixture configuration");

        let mut child = Command::new(env!("CARGO_BIN_EXE_service-orchestrator"))
            .arg("mcp")
            .arg("--config")
            .arg(&config_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the service-orchestrator binary launches");

        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let stderr_buffer = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&stderr_buffer);
        std::thread::spawn(move || {
            let mut buffer = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut buffer);
            if let Ok(mut guard) = sink.lock() {
                guard.push_str(&buffer);
            }
        });

        Self {
            child,
            stdin: Some(stdin),
            stdout: lines,
            stderr: stderr_buffer,
            next_id: 1,
        }
    }

    /// Writes one JSON-RPC message as a newline-delimited frame.
    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is still open");
        let mut line = serde_json::to_string(message).expect("the request serializes");
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .expect("the request is written");
        stdin.flush().expect("the request is flushed");
    }

    /// Sends a request and returns its JSON-RPC response.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        self.read_response(id)
    }

    /// Sends a notification (no response is expected).
    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }));
    }

    /// Reads stdout until the response with `id` arrives.
    ///
    /// Every line must be valid JSON — a stray banner or debug print on stdout
    /// fails the test here.
    fn read_response(&self, id: u64) -> Value {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("the server responded before the deadline");
            let line = self
                .stdout
                .recv_timeout(remaining)
                .expect("the server produced a line before the deadline");
            let trimmed = line.trim();
            assert!(
                !trimmed.is_empty(),
                "stdout must not contain blank lines (the transport reserves it)"
            );
            let value: Value = serde_json::from_str(trimmed)
                .unwrap_or_else(|error| panic!("non-JSON on stdout: {line:?} ({error})"));
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return value;
            }
            // Any other message (a notification or an unrelated response) is
            // ignored; the transport is free to interleave them.
        }
    }

    /// Closes stdin, waits for a clean exit, and returns stderr and the code.
    fn shutdown(mut self) -> (String, Option<i32>) {
        drop(self.stdin.take());
        let deadline = Instant::now() + DEADLINE;
        loop {
            match self.child.try_wait().expect("the child is waitable") {
                Some(status) => {
                    let stderr = self
                        .stderr
                        .lock()
                        .map(|guard| guard.clone())
                        .unwrap_or_default();
                    return (stderr, status.code());
                }
                None if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                None => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    panic!("the server did not exit after stdin was closed");
                }
            }
        }
    }
}

/// Extracts the structured envelope from a `tools/call` result.
///
/// Prefers `structuredContent` and falls back to the JSON text content block so
/// the assertion is robust across protocol revisions.
fn envelope(result: &Value) -> Value {
    if let Some(structured) = result.get("structuredContent") {
        if !structured.is_null() {
            return structured.clone();
        }
    }
    let text = result["content"][0]["text"]
        .as_str()
        .expect("a tool result carries a content text block");
    serde_json::from_str(text).expect("the content text block is JSON")
}

/// Drives the full acceptance flow in one server lifecycle.
#[test]
fn mcp_stdio_end_to_end() {
    let mut server = Server::launch();

    // -- 1. handshake --------------------------------------------------------
    let init = server.request(
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "service-orchestrator-e2e", "version": "0.0.0" },
        }),
    );
    assert!(
        init.get("error").is_none(),
        "initialize must succeed: {init}"
    );
    let capabilities = init["result"]["capabilities"]
        .as_object()
        .expect("the server advertises capabilities");
    for capability in ["tools", "resources", "prompts"] {
        assert!(
            capabilities.contains_key(capability),
            "the server advertises the '{capability}' capability"
        );
    }

    server.notify("notifications/initialized", json!({}));

    // -- 2. discovery --------------------------------------------------------
    let tools = server.request("tools/list", json!({}))["result"]["tools"].clone();
    let tools = tools.as_array().expect("tools/list returns an array");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in [
        "list_workspaces",
        "list_services",
        "get_service_status",
        "git_status",
        "build_service",
        "apply_migration",
        "start_service",
        "get_task",
        "get_logs",
        "prepare_environment",
    ] {
        assert!(names.contains(&expected), "tool '{expected}' is advertised");
    }
    assert!(
        !names.contains(&"run_shell"),
        "no arbitrary shell tool is exposed"
    );
    assert!(
        tools.iter().all(|tool| tool["annotations"].is_object()),
        "every tool carries risk annotations"
    );

    let resource_templates = server.request("resources/templates/list", json!({}))["result"]
        ["resourceTemplates"]
        .clone();
    let resource_templates = resource_templates
        .as_array()
        .expect("resource templates are listed");
    let template_uris: Vec<&str> = resource_templates
        .iter()
        .filter_map(|template| template["uriTemplate"].as_str())
        .collect();
    assert!(template_uris.contains(&"workspace://{workspaceId}"));
    assert!(template_uris.contains(&"service://{serviceId}"));
    assert!(template_uris.contains(&"task://{taskId}"));

    let prompts = server.request("prompts/list", json!({}))["result"]["prompts"].clone();
    let prompts = prompts.as_array().expect("prompts are listed");
    assert!(!prompts.is_empty(), "at least one prompt is advertised");

    // -- 3. read-only operation returns real fixture state -------------------
    let workspaces = envelope(
        &server.request(
            "tools/call",
            json!({ "name": "list_workspaces", "arguments": {} }),
        )["result"],
    );
    assert_eq!(workspaces["success"], json!(true));
    assert_eq!(workspaces["data"][0]["id"], json!("demo"));

    // -- 4. a mutating operation submits a real task -------------------------
    let fetch = envelope(
        &server.request(
            "tools/call",
            json!({ "name": "git_fetch", "arguments": { "serviceId": "orders-api" } }),
        )["result"],
    );
    assert_eq!(fetch["success"], json!(true));
    assert_eq!(fetch["data"]["outcome"], json!("task_scheduled"));
    let task_id = fetch["data"]["taskId"]
        .as_str()
        .expect("a mutating operation returns a real task id")
        .to_owned();

    // -- 5. the task can be queried ------------------------------------------
    let task = envelope(
        &server.request(
            "tools/call",
            json!({ "name": "get_task", "arguments": { "taskId": task_id } }),
        )["result"],
    );
    assert_eq!(task["success"], json!(true));
    assert_eq!(task["data"]["id"], json!(task_id));

    // -- 6. invalid input returns a structured error -------------------------
    let missing = server.request(
        "tools/call",
        json!({ "name": "get_service", "arguments": { "serviceId": "does-not-exist" } }),
    )["result"]
        .clone();
    assert_eq!(missing["isError"], json!(true));
    let missing = envelope(&missing);
    assert_eq!(missing["success"], json!(false));
    assert_eq!(missing["error"]["code"], json!("SERVICE_NOT_FOUND"));

    // A malformed request never routes: it is a JSON-RPC error, not a tool
    // result.
    let malformed = server.request(
        "tools/call",
        json!({ "name": "get_service", "arguments": { "serviceId": 5 } }),
    );
    assert_eq!(malformed["error"]["code"], json!(-32602));

    let unknown = server.request(
        "tools/call",
        json!({ "name": "no_such_tool", "arguments": {} }),
    );
    assert_eq!(unknown["error"]["code"], json!(-32601));

    // -- 7. protection cannot be bypassed ------------------------------------
    let denied = envelope(
        &server.request(
            "tools/call",
            json!({ "name": "git_reset_hard", "arguments": { "serviceId": "orders-api" } }),
        )["result"],
    );
    assert_eq!(denied["success"], json!(false));
    assert_eq!(denied["error"]["code"], json!("CONFIRMATION_REQUIRED"));

    // -- 8/9. clean shutdown, diagnostics on stderr --------------------------
    let (stderr, code) = server.shutdown();
    assert!(
        stderr.contains("mcp_server_started"),
        "the structured startup log is written to stderr: {stderr:?}"
    );
    assert_eq!(code, Some(0), "the server exits cleanly on stdin EOF");
}
