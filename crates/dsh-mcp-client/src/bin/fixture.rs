//! Crate-local Content-Length MCP fixture for stdio spawn tests. Not a product binary.

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    while let Ok(body) = read_frame(&mut reader) {
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = message.get("id").cloned();
        let params = match message.get("params") {
            Some(value) => value.clone(),
            None => json!({}),
        };
        match method {
            "initialize" => {
                let Some(id) = id else {
                    continue;
                };
                write_result(&mut writer, id, initialize_result());
            }
            "notifications/initialized" => {}
            "tools/list" => {
                let Some(id) = id else {
                    continue;
                };
                write_result(&mut writer, id, list_tools_result());
            }
            "tools/call" => {
                let Some(id) = id else {
                    continue;
                };
                write_result(&mut writer, id, call_tool_result(&params));
            }
            _ => {}
        }
    }
}

fn write_result(writer: &mut impl Write, id: Value, result: Value) {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let bytes = serde_json::to_vec(&body).expect("json");
    writer
        .write_all(&encode_frame(&bytes))
        .expect("fixture write");
    writer.flush().expect("fixture flush");
}

fn encode_frame(body: &[u8]) -> Vec<u8> {
    let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    framed.extend_from_slice(body);
    framed
}

fn read_frame(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut headers = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete headers",
            ));
        }
        headers.extend_from_slice(&line);
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let len = parse_content_length(&headers)?;
    let mut body = vec![0_u8; len];
    io::Read::read_exact(reader, &mut body)?;
    Ok(body)
}

fn parse_content_length(header_block: &[u8]) -> io::Result<usize> {
    let Some(headers) = header_block.strip_suffix(b"\r\n\r\n") else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing header terminator",
        ));
    };
    let headers = std::str::from_utf8(headers)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    for line in headers.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("content-length") {
            continue;
        }
        return value
            .trim()
            .parse::<usize>()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "missing Content-Length",
    ))
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2025-03-26",
        "capabilities": { "tools": { "listChanged": true } },
        "serverInfo": { "name": "fixture-server", "version": "1.0.0" },
    })
}

fn list_tools_result() -> Value {
    json!({
        "tools": [
            {
                "name": "add",
                "description": "Adds two numbers.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "a": { "type": "number", "description": "First number" },
                        "b": { "type": "number", "description": "Second number" }
                    },
                    "required": ["a", "b"]
                }
            },
            {
                "name": "fail",
                "description": "Always returns an error.",
                "inputSchema": { "type": "object", "properties": {} }
            }
        ]
    })
}

fn call_tool_result(params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = match params.get("arguments") {
        Some(value) => value.clone(),
        None => json!({}),
    };
    match name {
        "add" => {
            let a = args.get("a").and_then(Value::as_f64).unwrap_or(0.0);
            let b = args.get("b").and_then(Value::as_f64).unwrap_or(0.0);
            json!({
                "content": [{ "type": "text", "text": decimal_text(a + b) }]
            })
        }
        "fail" => json!({
            "content": [{ "type": "text", "text": "Something went wrong" }],
            "isError": true
        }),
        _ => json!({
            "content": [{ "type": "text", "text": "unknown tool" }],
            "isError": true
        }),
    }
}

fn decimal_text(sum: f64) -> String {
    if sum.is_finite() && sum.fract() == 0.0 && sum.abs() <= i64::MAX as f64 {
        return (sum as i64).to_string();
    }
    sum.to_string()
}
