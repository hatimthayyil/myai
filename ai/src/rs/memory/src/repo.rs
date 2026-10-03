use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use anyhow::Result;

use crate::store::AtPath;

pub const STORE: &str = ".ai/memory";
const IGNORE: &str = "/.ai/memory/";

/// The nearest ancestor of `from` (inclusive) holding a `.git` entry.
pub fn root(from: &Path) -> Option<&Path> {
    from.ancestors().find(|a| a.join(".git").exists())
}

/// Ensures `<root>/.gitignore` ignores the store; returns whether it added the line.
pub fn ignore_store(root: &Path) -> Result<bool> {
    let p = root.join(".gitignore");
    let have = match fs::read_to_string(&p) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).at(&p),
    };
    let forms = [IGNORE, "/.ai/memory", ".ai/memory", ".ai/memory/"];
    if have.lines().any(|l| forms.contains(&l.trim())) {
        return Ok(false);
    }
    let sep = if have.is_empty() || have.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut f = fs::File::options()
        .append(true)
        .create(true)
        .open(&p)
        .at(&p)?;
    writeln!(f, "{sep}{IGNORE}").at(&p)?;
    Ok(true)
}
