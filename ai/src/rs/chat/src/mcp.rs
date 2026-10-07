use std::io::{BufRead, Write};

use ai_memory::{Knob, Snapshot, Store, page, zoom};
use anyhow::{Result, bail};
use jiff::tz::TimeZone;
use serde_json::{Value, json};

/// The MCP server's name: its tools reach the model as `mcp__memory__<tool>`.
pub const SERVER: &str = "memory";
pub const TOOLS: [&str; 2] = ["mcp__memory__zoom", "mcp__memory__date"];

fn tools() -> Value {
    json!([
        {
            "name": "zoom",
            "description": "Open the line id+n of the view into the two lines of n/2 under it; n = 1 gives the message whole.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer"},
                    "n": {"type": "integer"},
                    "part": {"type": "integer", "description": "the part of a long message to show, from 1"}
                },
                "required": ["id", "n"]
            }
        },
        {
            "name": "date",
            "description": "The date and time of message id.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "integer"}},
                "required": ["id"]
            }
        }
    ])
}

fn int(args: &Value, k: &str) -> Option<u64> {
    args.get(k)?.as_u64()
}

fn show(v: &Value, k: &str) -> String {
    v.get(k).map_or("?".into(), Value::to_string)
}

/// `zoom` per spec §7.1, a long message paged by `PART_CHARS`.
pub fn zoom_tool(s: &Snapshot, args: &Value) -> Result<String> {
    let none = format!("No line {}+{}.", show(args, "id"), show(args, "n"));
    let (Some(id), Some(n)) = (int(args, "id"), int(args, "n")) else {
        return Ok(none);
    };
    let Some(text) = zoom(s, id, n)? else {
        return Ok(none);
    };
    let part = match args.get("part") {
        None | Some(Value::Null) => 1,
        Some(p) => p.as_u64().unwrap_or(0),
    };
    let (body, parts) = page(&text, s.cfg().get(Knob::PartChars).min(29_000), part)?;
    Ok(match part < parts {
        true => format!(
            "{body}\n[Part {part} of {parts}. Next: zoom(id={id}, n={n}, part={})]",
            part + 1
        ),
        false => body.to_string(),
    })
}

/// `date` per spec §7.1, in the local time zone.
pub fn date_tool(s: &Snapshot, args: &Value, tz: &TimeZone) -> Result<String> {
    match int(args, "id").filter(|&i| s.log_len().is_ok_and(|t| i < t)) {
        Some(i) => Ok(s.message(i)?.time_in(tz)),
        None => Ok(format!("No message {}.", show(args, "id"))),
    }
}

fn call(store: &Store, params: &Value) -> Result<String> {
    let s = store.snapshot()?;
    let args = &params["arguments"];
    match params["name"].as_str() {
        Some("zoom") => zoom_tool(&s, args),
        Some("date") => date_tool(&s, args, &TimeZone::system()),
        _ => bail!("unknown tool {}", show(params, "name")),
    }
}

fn answer(store: &Store, req: &Value) -> Option<Value> {
    let id = req.get("id")?.clone();
    let params = &req["params"];
    let reply = match req["method"].as_str().unwrap_or_default() {
        "initialize" => json!({"result": {
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")}
        }}),
        "ping" => json!({"result": {}}),
        "tools/list" => json!({"result": {"tools": tools()}}),
        "tools/call" => match call(store, params) {
            Ok(text) => json!({"result": {"content": [{"type": "text", "text": text}]}}),
            Err(e) => json!({"result": {
                "content": [{"type": "text", "text": format!("error: {e:#}")}],
                "isError": true
            }}),
        },
        m => json!({"error": {"code": -32601, "message": format!("method not found: {m}")}}),
    };
    let mut reply = reply;
    reply["jsonrpc"] = "2.0".into();
    reply["id"] = id;
    Some(reply)
}

/// A read-only stdio MCP server with `zoom` and `date`; each call reads the latest commit.
pub fn serve(store: &Store, input: impl BufRead, out: &mut impl Write) -> Result<()> {
    for line in input.lines() {
        let Ok(req) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        if let Some(reply) = answer(store, &req) {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_memory::{Kind, Message};
    #[test]
    fn rpc_serves_tools_and_pages_raw_zoom_without_mutating_memory() {
        let temp = tempfile::TempDir::new().unwrap();
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        store
            .append(
                "test",
                &[
                    Message::new(Kind::User, &"é".repeat(35_000)),
                    Message::new(Kind::Ai, "short"),
                ],
            )
            .unwrap();
        let head = store.head().unwrap();
        let input = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"zoom","arguments":{"id":0,"n":1}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"zoom","arguments":{"id":0,"n":1,"part":2}}}),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"zoom","arguments":{"id":1,"n":2}}}),
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"date","arguments":{"id":1}}}),
            json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"zoom","arguments":{"id":0,"n":1,"part":0}}}),
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        let mut out = vec![];
        serve(&store, std::io::Cursor::new(input), &mut out).unwrap();
        let replies: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(replies.len(), 7);
        assert_eq!(replies[0]["result"]["capabilities"], json!({"tools":{}}));
        assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 2);
        let first = replies[2]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            first.starts_with("0+0|user: é") && first.contains("Next: zoom(id=0, n=1, part=2)")
        );
        assert!(first.chars().count() <= 30_000);
        assert!(
            replies[3]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with('é')
        );
        assert_eq!(replies[4]["result"]["content"][0]["text"], "No line 1+2.");
        assert!(
            replies[5]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(':')
        );
        assert_eq!(replies[6]["result"]["isError"], true);
        assert_eq!(store.head().unwrap(), head);
    }
}
