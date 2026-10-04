use std::{
    collections::hash_map::DefaultHasher,
    ffi::OsString,
    fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, RecvTimeoutError, channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::{
    backend::{Backend, Block, Conversation},
    store::AtPath,
};

const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Claude Code in print mode (`claude -p`), isolated from the user's settings, without tools.
pub struct ClaudeCode {
    program: OsString,
    model: String,
    effort: String,
    prompts: TempDir,
}

impl ClaudeCode {
    /// The compactor's model: Sonnet at medium effort, through the `claude` on `PATH`.
    pub fn compactor() -> Result<ClaudeCode> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let found = std::env::split_paths(&path)
            .map(|d| d.join("claude"))
            .find(|p| p.is_file())
            .context("No claude on PATH: the compactor runs on Claude Code.")?;
        ClaudeCode::compactor_at(found.into())
    }

    pub fn compactor_at(program: OsString) -> Result<ClaudeCode> {
        Ok(ClaudeCode {
            program,
            model: "sonnet".into(),
            effort: "medium".into(),
            prompts: TempDir::new().context("Cannot create a temporary directory.")?,
        })
    }

    fn system_file(&self, system: &str) -> Result<PathBuf> {
        let mut h = DefaultHasher::new();
        system.hash(&mut h);
        let p = self.prompts.path().join(format!("{:016x}.txt", h.finish()));
        if !p.exists() {
            fs::write(&p, system).at(&p)?;
        }
        Ok(p)
    }

    pub fn command(&self, system_file: &Path) -> Command {
        let mut c = Command::new(&self.program);
        c.args(["-p", "--model", &self.model, "--effort", &self.effort])
            .args([
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
            ])
            .args(["--verbose", "--include-partial-messages"])
            .args([
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--strict-mcp-config",
            ])
            .arg("--system-prompt-file")
            .arg(system_file)
            .args(["--tools", "", "--safe-mode"])
            .env("CLAUDE_CODE_PROMPT_CACHE_TTL", "5m")
            .env("DISABLE_PROMPT_CACHING", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }
}

impl Backend for ClaudeCode {
    fn model(&self) -> &str {
        &self.model
    }

    fn open(&self, system: &str) -> Result<Box<dyn Conversation>> {
        let mut child = self
            .command(&self.system_file(system)?)
            .spawn()
            .context("Cannot run claude. Is Claude Code installed and on PATH?")?;
        let stdin = child.stdin.take().context("No stdin.")?;
        let stdout = child.stdout.take().context("No stdout.")?;
        let mut stderr = child.stderr.take().context("No stderr.")?;
        let (tx, lines) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let err = Arc::new(Mutex::new(String::new()));
        let sink = err.clone();
        thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            *sink.lock().expect("stderr lock") = s;
        });
        Ok(Box::new(Session {
            child,
            stdin,
            lines,
            stderr: err,
        }))
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<String>>,
}

/// A stream-json user message.
pub fn user_message(message: &[Block]) -> String {
    let content: Vec<Value> = message
        .iter()
        .map(|b| match b.cache {
            true => json!({"type": "text", "text": b.text, "cache_control": {"type": "ephemeral"}}),
            false => json!({"type": "text", "text": b.text}),
        })
        .collect();
    json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
}

/// The reply a stream-json `result` event carries; `None` for any other line.
pub fn result(line: &str) -> Option<Result<String>> {
    let ev: Value = serde_json::from_str(line).ok()?;
    if ev["type"] != "result" {
        return None;
    }
    let text = ev["result"].as_str().unwrap_or_default().to_string();
    Some(if ev["is_error"].as_bool().unwrap_or(false) {
        let why = if text.is_empty() {
            ev["subtype"].as_str().unwrap_or("error").to_string()
        } else {
            text
        };
        Err(anyhow::anyhow!("claude: {}", tail(&why, 300)))
    } else if ev["stop_reason"] == "refusal" {
        Err(anyhow::anyhow!("claude refused"))
    } else {
        Ok(text)
    })
}

fn tail(s: &str, n: usize) -> &str {
    let s = s.trim();
    &s[s.ceil_char_boundary(s.len().saturating_sub(n))..]
}

impl Conversation for Session {
    fn say(&mut self, message: &[Block]) -> Result<String> {
        let sent =
            writeln!(self.stdin, "{}", user_message(message)).and_then(|_| self.stdin.flush());
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(wait) {
                Ok(line) => {
                    if let Some(r) = result(&line) {
                        return r;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    bail!("claude: no result after {} s.", CALL_TIMEOUT.as_secs())
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let code = self.child.wait().ok().and_then(|s| s.code());
                    thread::sleep(Duration::from_millis(50));
                    let err = self.stderr.lock().expect("stderr lock").clone();
                    let err = if err.trim().is_empty() {
                        sent.err()
                            .map_or("no error output".into(), |e| e.to_string())
                    } else {
                        tail(&err, 300).to_string()
                    };
                    bail!("claude exited (code {code:?}) before its result: {err}");
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compactor_runs_isolated_without_tools() {
        let c = ClaudeCode::compactor_at("claude".into()).unwrap();
        let file = c.system_file("You write the memory.").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "You write the memory.");
        assert_eq!(c.system_file("You write the memory.").unwrap(), file);
        let cmd = c.command(&file);
        assert_eq!(cmd.get_program(), "claude");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        let f = file.to_str().unwrap();
        assert_eq!(
            args,
            [
                "-p",
                "--model",
                "sonnet",
                "--effort",
                "medium",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--strict-mcp-config",
                "--system-prompt-file",
                f,
                "--tools",
                "",
                "--safe-mode",
            ]
        );
        let mut env: Vec<_> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap(), v.and_then(|v| v.to_str())))
            .collect();
        env.sort();
        assert_eq!(
            env,
            [
                ("CLAUDE_CODE_PROMPT_CACHE_TTL", Some("5m")),
                ("DISABLE_PROMPT_CACHING", Some("1"))
            ]
        );
        assert_eq!(c.model(), "sonnet");
    }

    #[test]
    fn stream_json_in_and_out() {
        let m = user_message(&[Block::new("a", true), Block::new("b", false)]);
        let v: Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(
            v["message"]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert!(v["message"]["content"][1].get("cache_control").is_none());
        assert!(result(r#"{"type":"assistant"}"#).is_none());
        assert!(result("not json").is_none());
        let ok = result(
            r#"{"type":"result","is_error":false,"result":" a line ","stop_reason":"end_turn"}"#,
        );
        assert_eq!(ok.unwrap().unwrap(), " a line ");
        let err = result(r#"{"type":"result","is_error":true,"subtype":"error_max_turns"}"#);
        assert!(
            err.unwrap()
                .unwrap_err()
                .to_string()
                .contains("error_max_turns")
        );
        let refused =
            result(r#"{"type":"result","is_error":false,"result":"","stop_reason":"refusal"}"#);
        assert!(refused.unwrap().is_err());
    }
}
