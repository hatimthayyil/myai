use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use ai_memory::claude::{self, Prompts};
use anyhow::Result;
use serde_json::json;

use crate::mcp::{SERVER, TOOLS};

pub const BUILTIN: &str = "Bash,Read,Edit,Write,Glob,Grep,WebFetch,WebSearch";

/// The inline `--mcp-config` that has `claude` start `exe chat mcp` on the memory at `dir`.
pub fn mcp_config(exe: &Path, dir: &Path) -> String {
    json!({"mcpServers": {SERVER: {
        "command": exe,
        "args": ["chat", "mcp"],
        "env": {"AI_MEMORY_DIR": dir}
    }}})
    .to_string()
}

/// The agent's `claude -p`: one argv for every turn and every priming call of a chat.
pub struct Master {
    program: OsString,
    model: String,
    effort: String,
    mcp: String,
    system: PathBuf,
    _prompts: Prompts,
}

impl Master {
    pub fn new(
        program: OsString,
        model: &str,
        effort: &str,
        system: &str,
        mcp: String,
    ) -> Result<Master> {
        let prompts = Prompts::new()?;
        Ok(Master {
            program,
            model: model.into(),
            effort: effort.into(),
            mcp,
            system: prompts.file(system)?,
            _prompts: prompts,
        })
    }

    /// The turn's command; `prime` only adds `DISABLE_PROMPT_CACHING=1`, which no request carries.
    pub fn command(&self, prime: bool) -> Command {
        let allowed = format!("{BUILTIN},{}", TOOLS.join(","));
        let mut c = claude::command(
            &self.program,
            &self.model,
            &self.effort,
            &self.system,
            BUILTIN,
        );
        c.args(["--mcp-config", &self.mcp])
            .args(["--permission-mode", "dontAsk", "--allowedTools", &allowed])
            .arg("--replay-user-messages");
        if prime {
            c.env("DISABLE_PROMPT_CACHING", "1");
        } else {
            c.env_remove("DISABLE_PROMPT_CACHING");
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_and_primes_share_one_argv() {
        let mcp = mcp_config(Path::new("/bin/ai"), Path::new("/m"));
        assert_eq!(
            mcp,
            r#"{"mcpServers":{"memory":{"args":["chat","mcp"],"command":"/bin/ai","env":{"AI_MEMORY_DIR":"/m"}}}}"#
        );
        let m = Master::new("claude".into(), "opus", "high", "Be MyAI.", mcp.clone()).unwrap();
        assert_eq!(std::fs::read_to_string(&m.system).unwrap(), "Be MyAI.");
        let (turn, prime) = (m.command(false), m.command(true));
        let args = |c: &Command| -> Vec<String> {
            c.get_args()
                .map(|a| a.to_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(args(&turn), args(&prime));
        let a = args(&turn);
        let sys = m.system.to_str().unwrap();
        assert_eq!(
            a,
            [
                "-p",
                "--model",
                "opus",
                "--effort",
                "high",
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
                sys,
                "--tools",
                BUILTIN,
                "--mcp-config",
                &mcp,
                "--permission-mode",
                "dontAsk",
                "--allowedTools",
                "Bash,Read,Edit,Write,Glob,Grep,WebFetch,WebSearch,mcp__memory__zoom,mcp__memory__date",
                "--replay-user-messages",
            ]
        );
        assert!(!a.iter().any(|x| x == "--safe-mode" || x == "--bare"));
        let env = |c: &Command| -> Vec<(String, Option<String>)> {
            let mut e: Vec<_> = c
                .get_envs()
                .map(|(k, v)| {
                    (
                        k.to_str().unwrap().into(),
                        v.map(|v| v.to_str().unwrap().into()),
                    )
                })
                .collect();
            e.sort();
            e
        };
        let ttl = (
            "CLAUDE_CODE_PROMPT_CACHE_TTL".to_string(),
            Some("5m".to_string()),
        );
        assert_eq!(
            env(&turn),
            [ttl.clone(), ("DISABLE_PROMPT_CACHING".into(), None)]
        );
        assert_eq!(
            env(&prime),
            [ttl, ("DISABLE_PROMPT_CACHING".into(), Some("1".into()))]
        );
    }
}
