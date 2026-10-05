use std::path::Path;

use gix::bstr::ByteSlice;

use crate::record::{Place, Who};

const CODEX: [&str; 3] = [
    "CODEX_THREAD_ID",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
];

/// The git repository at `cwd`: its identity, `HEAD` and branch.
pub fn place(cwd: &Path) -> Place {
    let Ok(repo) = gix::discover(cwd) else {
        return Place::default();
    };
    Place {
        repo: identity(&repo).unwrap_or_default(),
        head: repo
            .head_id()
            .map(|id| id.to_hex_with_len(12).to_string())
            .unwrap_or_default(),
        branch: repo
            .head_name()
            .ok()
            .flatten()
            .map(|n| n.shorten().to_string())
            .unwrap_or_default(),
    }
}

/// The work tree of the git repository at `cwd`.
pub fn repo_root(cwd: &Path) -> Option<std::path::PathBuf> {
    Some(gix::discover(cwd).ok()?.workdir()?.to_path_buf())
}

fn identity(repo: &gix::Repository) -> Option<String> {
    let url = repo.config_snapshot().string("remote.origin.url");
    url.and_then(|u| owner_name(u.as_bstr())).or_else(|| {
        let root = repo.workdir().unwrap_or(repo.git_dir());
        Some(root.file_name()?.to_string_lossy().into_owned())
    })
}

fn owner_name(url: &gix::bstr::BStr) -> Option<String> {
    let url = gix::url::parse(url).ok()?;
    let path = url.path.to_str_lossy();
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.rsplit(['/', ':']).filter(|p| !p.is_empty());
    let name = parts.next()?;
    Some(format!("{}/{name}", parts.next()?))
}

/// The agent, model and session, read through `var`.
pub fn who(var: impl Fn(&str) -> Option<String>) -> Who {
    let var = |k: &str| var(k).filter(|v| !v.is_empty());
    let agent = var("AI_AGENT").or_else(|| {
        if var("CLAUDECODE").is_some() {
            Some("claude-code".into())
        } else if CODEX.iter().any(|k| var(k).is_some()) {
            Some("codex".into())
        } else {
            None
        }
    });
    Who {
        agent: agent.unwrap_or_default(),
        model: var("AI_MODEL").unwrap_or_default(),
        session: var("CLAUDE_CODE_SESSION_ID")
            .or_else(|| var("AI_SESSION"))
            .unwrap_or_default(),
    }
}

pub fn env_who() -> Who {
    who(|k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(pairs: &[(&str, &str)]) -> Who {
        who(|k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn agents() {
        assert_eq!(with(&[]), Who::default());
        assert_eq!(with(&[("CLAUDECODE", "1")]).agent, "claude-code");
        assert_eq!(with(&[("CODEX_SANDBOX", "seatbelt")]).agent, "codex");
        let w = with(&[
            ("CLAUDECODE", "1"),
            ("AI_AGENT", "pi"),
            ("AI_MODEL", "opus"),
            ("AI_SESSION", "s2"),
            ("CLAUDE_CODE_SESSION_ID", "s1"),
        ]);
        assert_eq!(
            (w.agent.as_str(), w.model.as_str(), w.session.as_str()),
            ("pi", "opus", "s1")
        );
        assert_eq!(
            with(&[("AI_SESSION", "s2"), ("AI_AGENT", "")]).session,
            "s2"
        );
    }

    #[test]
    fn repo_names() {
        for (url, want) in [
            ("git@github.com:acme/widget.git", Some("acme/widget")),
            ("https://github.com/acme/widget", Some("acme/widget")),
            (
                "https://gitlab.com/group/sub/widget.git/",
                Some("sub/widget"),
            ),
            ("/srv/git/widget.git", Some("git/widget")),
            ("widget", None),
        ] {
            assert_eq!(owner_name(url.into()).as_deref(), want, "{url}");
        }
    }
}
