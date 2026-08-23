//! `shaipe mcp` over real standard input and output.
//!
//! The unit tests in `src/mcp/server.rs` drive the handler over an in-process
//! duplex, which is where protocol bugs live. This file covers the thing they
//! cannot: that the *binary* speaks JSON-RPC on stdout, and speaks nothing
//! else there. A stray byte on that stream is invisible in-process and fatal
//! in the field.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

/// The fixture, copied so nothing writes to a tracked file.
fn project(directory: &std::path::Path) -> std::path::PathBuf {
    let path = directory.join("logo.svg");
    std::fs::copy("tests/fixtures/logo.svg", &path).expect("the fixture is readable");
    path
}

/// Speak MCP to `shaipe mcp` and collect what it says back.
///
/// Frames are newline-delimited JSON, which is what the MCP stdio transport
/// specifies; nothing here parses anything cleverer than a line.
fn exchange(arguments: &[&str], requests: &[Value]) -> (Vec<Value>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_shaipe"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");

    let mut stdin = child.stdin.take().expect("stdin is piped");
    for request in requests {
        writeln!(stdin, "{request}").expect("the server accepts a frame");
    }
    stdin.flush().expect("the frames are sent");

    // Read exactly as many responses as we asked for, then close stdin so the
    // server exits. Reading to EOF first would deadlock: the server has no
    // reason to stop while its input is open.
    let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut responses = Vec::new();
    for line in stdout.lines() {
        let line = line.expect("stdout is valid UTF-8");
        if line.trim().is_empty() {
            continue;
        }
        responses.push(
            serde_json::from_str(&line)
                .unwrap_or_else(|error| panic!("`{line}` is not JSON-RPC: {error}")),
        );
        if responses.len() == requests.len() {
            break;
        }
    }

    drop(stdin);
    let _ = child.wait_timeout(Duration::from_secs(5));

    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        use std::io::Read as _;
        let _ = handle.read_to_string(&mut stderr);
    }

    (responses, stderr)
}

/// `wait` with a bound, so a hung server fails the test rather than the suite.
trait WaitTimeout {
    fn wait_timeout(&mut self, limit: Duration) -> Option<std::process::ExitStatus>;
}

impl WaitTimeout for std::process::Child {
    fn wait_timeout(&mut self, limit: Duration) -> Option<std::process::ExitStatus> {
        let deadline = std::time::Instant::now() + limit;
        loop {
            match self.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = self.kill();
                    return None;
                }
            }
        }
    }
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "shaipe-tests", "version": "0" }
        }
    })
}

#[test]
fn the_standalone_server_advertises_every_tool() {
    let directory = tempfile::tempdir().unwrap();
    let path = project(directory.path());

    let (responses, _) = exchange(
        &["mcp", path.to_str().unwrap()],
        &[
            initialize(),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        ],
    );

    let names: Vec<&str> = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools/list returns an array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a tool has a name"))
        .collect();

    assert_eq!(
        names,
        [
            "get_palette",
            "get_project",
            "get_svg",
            "get_variants",
            "render_grid",
            "render_svg",
            "set_generation",
            "set_palette_colour",
            "write_svg",
            "write_variant",
        ]
    );
}

#[test]
fn the_standalone_server_introduces_itself_as_shaipe() {
    let directory = tempfile::tempdir().unwrap();
    let path = project(directory.path());

    let (responses, _) = exchange(&["mcp", path.to_str().unwrap()], &[initialize()]);

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "shaipe");
    assert!(
        responses[0]["result"]["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("render_svg")),
        "the server did not tell the model how to see the artwork"
    );
}

#[test]
fn a_render_arrives_over_stdio_as_an_image_a_model_can_see() {
    // The end-to-end shape of the product claim, through a real process
    // boundary: an agent asks for a picture and gets image content back.
    let directory = tempfile::tempdir().unwrap();
    let path = project(directory.path());

    let (responses, _) = exchange(
        &["mcp", path.to_str().unwrap()],
        &[
            initialize(),
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "render_svg", "arguments": { "variant": "icon", "width": 32 } }
            }),
        ],
    );

    let content = responses[1]["result"]["content"]
        .as_array()
        .expect("a tool result carries content");

    let image = content
        .iter()
        .find(|block| block["type"] == "image")
        .expect("the render came back with no image");

    assert_eq!(image["mimeType"], "image/png");

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().expect("image data is a string"))
        .expect("image data is base64");
    let pixmap = tiny_skia::Pixmap::decode_png(&bytes).expect("it is a PNG");
    assert_eq!((pixmap.width(), pixmap.height()), (32, 32));
}

#[test]
fn nothing_but_json_rpc_is_written_to_standard_output() {
    // The regression test for the failure mode that is invisible in-process:
    // a warning printed to stdout is a frame the client cannot parse, and it
    // looks like Shaipe speaking a broken protocol rather than like a warning.
    //
    // `-vv` is the interesting case, because it is the flag that turns on the
    // logging that would do it, and `render_svg` on a project whose declared
    // font is missing is a call that actually logs.
    let directory = tempfile::tempdir().unwrap();
    let path = project(directory.path());

    let (responses, _stderr) = exchange(
        &["-vv", "mcp", path.to_str().unwrap()],
        &[
            initialize(),
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "render_svg", "arguments": { "variant": "wordmark" } }
            }),
        ],
    );

    // `exchange` parses every line as JSON and panics otherwise, so arriving
    // here at all is most of the assertion.
    for response in &responses {
        assert_eq!(response["jsonrpc"], "2.0", "{response}");
    }
}

#[test]
fn the_standalone_server_does_not_write_to_the_project() {
    // Invariant 2, asserted where it matters most: the agent is the only user
    // of a standalone session, and it still may not touch the working tree
    // unless asked.
    let directory = tempfile::tempdir().unwrap();
    let path = project(directory.path());
    let before = std::fs::read_to_string(&path).unwrap();

    let edited = before.replace("#f05032", "#00ff00");
    let (responses, _) = exchange(
        &["mcp", path.to_str().unwrap()],
        &[
            initialize(),
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "write_svg", "arguments": { "source": edited } }
            }),
        ],
    );

    assert_ne!(responses[1]["result"]["isError"], json!(true));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "the project was written to without being asked"
    );
}

#[test]
fn the_standalone_server_saves_when_it_is_asked_to() {
    let directory = tempfile::tempdir().unwrap();
    let path = project(directory.path());
    let before = std::fs::read_to_string(&path).unwrap();
    let edited = before.replace("#f05032", "#00ff00");

    exchange(
        &["mcp", "--write", path.to_str().unwrap()],
        &[
            initialize(),
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "write_svg", "arguments": { "source": edited } }
            }),
            // A third round trip, so the save — which happens after the reply
            // is sent — has certainly run by the time the process exits.
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "get_svg", "arguments": {} }
            }),
        ],
    );

    assert!(
        std::fs::read_to_string(&path).unwrap().contains("#00ff00"),
        "`--write` did not save the edit"
    );
}

#[test]
fn opening_a_project_that_is_not_there_fails_before_any_protocol_starts() {
    // A misconfigured MCP server should say what is wrong on stderr and exit,
    // not sit there speaking a protocol about a project it does not have.
    let output = Command::new(env!("CARGO_BIN_EXE_shaipe"))
        .args(["mcp", "does-not-exist.svg"])
        .output()
        .expect("the binary runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does-not-exist.svg"), "{stderr}");
    assert!(output.stdout.is_empty(), "it wrote to the JSON-RPC stream");
}
