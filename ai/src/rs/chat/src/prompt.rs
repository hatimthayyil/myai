use std::{fs, io, path::Path};

use ai_memory::prov;
use anyhow::{Context, Result};

pub const MASTER: &str = include_str!("../prompts/master.txt");
pub const VIEW_DOC: &str = include_str!("../prompts/view_doc.txt");

/// MASTER, VIEW_DOC, the user's global instructions (`~/.claude/CLAUDE.md`) and the
/// `AGENTS.md` of the git repository at `cwd`, those present, in that order.
pub fn system_prompt(home: Option<&Path>, cwd: &Path) -> Result<String> {
    let mut parts = vec![
        MASTER.trim_end().to_string(),
        VIEW_DOC.trim_end().to_string(),
    ];
    let files = [
        home.map(|h| h.join(".claude/CLAUDE.md")),
        prov::repo_root(cwd).map(|r| r.join("AGENTS.md")),
    ];
    for f in files.into_iter().flatten() {
        match fs::read_to_string(&f) {
            Ok(t) if !t.trim().is_empty() => parts.push(t.trim_end().to_string()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("Cannot read {}.", f.display())),
        }
    }
    Ok(parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_the_spec_s() {
        for p in [MASTER, VIEW_DOC] {
            assert!(!p.contains("OptChat") && p.contains("MyAI"));
        }
        assert!(MASTER.starts_with("You are MyAI, an AI agent that works for one user"));
        assert!(VIEW_DOC.contains("note\n(notes recorded by other agent sessions)"));
        assert!(
            VIEW_DOC
                .trim_end()
                .ends_with("gives the date and time of message id.")
        );
    }

    #[test]
    fn instructions_come_last_and_only_when_present() {
        let home = tempfile::TempDir::new().unwrap();
        let elsewhere = tempfile::TempDir::new().unwrap();
        let bare = format!("{}\n\n{}", MASTER.trim_end(), VIEW_DOC.trim_end());
        assert_eq!(
            system_prompt(Some(home.path()), elsewhere.path()).unwrap(),
            bare
        );
        fs::create_dir(home.path().join(".claude")).unwrap();
        fs::write(home.path().join(".claude/CLAUDE.md"), "Mine.\n").unwrap();
        let repo = elsewhere.path().join("repo");
        let git = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&repo)
            .status()
            .unwrap();
        assert!(git.success());
        fs::create_dir(repo.join("sub")).unwrap();
        fs::write(repo.join("AGENTS.md"), "Theirs.\n").unwrap();
        assert_eq!(
            system_prompt(Some(home.path()), &repo.join("sub")).unwrap(),
            format!("{bare}\n\nMine.\n\nTheirs.")
        );
    }
}
