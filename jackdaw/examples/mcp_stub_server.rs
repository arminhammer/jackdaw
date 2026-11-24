//! A small Model Context Protocol server for the `call: mcp` scenarios (`tests/lib/tasks/mcp_calls.feature`). It
//! is no reference implementation: it answers the methods the scenarios call and refuses anything else.
//!
//! ```text
//! mcp_stub_server stdio      # newline-delimited JSON-RPC on stdin and stdout
//! mcp_stub_server http       # Streamable HTTP on 127.0.0.1; prints `listening <port>` first
//! mcp_stub_server http-sse   # the same, answering each request as a `text/event-stream`
//! ```
//!
//! A request before `initialize` and `notifications/initialized` is refused, so a client that skips the handshake
//! fails. The tool `session` reports what the handshake told the server; `echo` returns its `text`; `fail` answers
//! with a JSON-RPC error; `slow` takes five seconds.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Session {
    protocol_version: String,
    client_name: String,
    client_version: String,
    ready: bool,
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn handle(session: &Mutex<Session>, message: &Value) -> Option<Value> {
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let Some(id) = message.get("id") else {
        if method == "notifications/initialized" {
            session.lock().unwrap().ready = true;
        }
        return None;
    };
    if method == "initialize" {
        let mut s = session.lock().unwrap();
        s.protocol_version = params["protocolVersion"].as_str().unwrap_or("").to_string();
        s.client_name = params["clientInfo"]["name"].as_str().unwrap_or("").to_string();
        s.client_version = params["clientInfo"]["version"].as_str().unwrap_or("").to_string();
        return Some(json!({ "jsonrpc": "2.0", "id": id, "result": {
            "protocolVersion": s.protocol_version,
            "capabilities": { "tools": {}, "prompts": {}, "resources": {} },
            "serverInfo": { "name": "stub", "version": "1.0.0" },
        }}));
    }
    if !session.lock().unwrap().ready {
        return Some(error(id, -32002, "the client has not initialized the session"));
    }
    let result = match method {
        "tools/list" => json!({ "tools": [
            { "name": "echo", "description": "Returns its text", "inputSchema": { "type": "object" } },
            { "name": "session", "description": "Reports the handshake", "inputSchema": { "type": "object" } },
            { "name": "fail", "description": "Always fails", "inputSchema": { "type": "object" } },
            { "name": "slow", "description": "Takes five seconds", "inputSchema": { "type": "object" } },
        ]}),
        "tools/call" => match params["name"].as_str() {
            Some("echo") => json!({ "content": [{ "type": "text", "text": params["arguments"]["text"] }] }),
            Some("session") => {
                let s = session.lock().unwrap();
                json!({
                    "content": [{ "type": "text", "text": "session" }],
                    "structuredContent": {
                        "protocolVersion": s.protocol_version,
                        "clientName": s.client_name,
                        "clientVersion": s.client_version,
                        "greeting": std::env::var("STUB_GREETING").unwrap_or_default(),
                    },
                })
            }
            Some("fail") => return Some(error(id, -32000, "the tool exploded")),
            Some("slow") => {
                std::thread::sleep(Duration::from_secs(5));
                json!({ "content": [{ "type": "text", "text": "done" }] })
            }
            _ => return Some(error(id, -32602, "unknown tool")),
        },
        "prompts/list" => json!({ "prompts": [
            { "name": "greet", "description": "Greets someone",
              "arguments": [{ "name": "who", "required": true }] },
        ]}),
        "prompts/get" => {
            let who = params["arguments"]["who"].as_str().unwrap_or("nobody");
            json!({ "description": "Greets someone", "messages": [
                { "role": "user", "content": { "type": "text", "text": format!("Hello, {who}!") } },
            ]})
        }
        "resources/list" => json!({ "resources": [
            { "uri": "stub://readme", "name": "readme", "mimeType": "text/plain" },
        ]}),
        "resources/read" => json!({ "contents": [
            { "uri": params["uri"], "mimeType": "text/plain", "text": "The stub's readme" },
        ]}),
        "resources/templates/list" => json!({ "resourceTemplates": [
            { "uriTemplate": "stub://notes/{id}", "name": "notes" },
        ]}),
        _ => return Some(error(id, -32601, "method not found")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn stdio() {
    let session = Arc::new(Mutex::new(Session::default()));
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let session = Arc::clone(&session);
        if let Some(reply) = handle(&session, &message) {
            writeln!(out, "{reply}").unwrap();
            out.flush().unwrap();
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str, headers: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn serve(mut stream: TcpStream, session: &Mutex<Session>, sse: bool) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    if request_line.starts_with("POST") {
        let message: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        match handle(session, &message) {
            None => respond(&mut stream, "202 Accepted", "", ""),
            Some(reply) if sse => respond(
                &mut stream,
                "200 OK",
                "Content-Type: text/event-stream\r\n",
                &format!("id: 1\ndata: \n\ndata: {reply}\n\n"),
            ),
            Some(reply) => respond(
                &mut stream,
                "200 OK",
                "Content-Type: application/json\r\nMcp-Session-Id: stub-session\r\n",
                &reply.to_string(),
            ),
        }
    } else if request_line.starts_with("DELETE") {
        respond(&mut stream, "200 OK", "", "");
    } else {
        respond(&mut stream, "405 Method Not Allowed", "", "");
    }
}

fn http(sse: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    println!("listening {}", listener.local_addr().unwrap().port());
    std::io::stdout().flush().unwrap();
    let session = Arc::new(Mutex::new(Session::default()));
    for stream in listener.incoming().flatten() {
        let session = Arc::clone(&session);
        std::thread::spawn(move || serve(stream, &session, sse));
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("stdio") => stdio(),
        Some("http") => http(false),
        Some("http-sse") => http(true),
        _ => eprintln!("usage: mcp_stub_server stdio | http | http-sse"),
    }
}
