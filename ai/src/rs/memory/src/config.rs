use std::{fs, path::Path};

use anyhow::{Result, bail};

use crate::store::{AtPath, LOG_REC, ME, TREE_REC, pretty};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    WakeLines,
    EntryChars,
    PartChars,
    PartLines,
}

impl Knob {
    pub const ALL: [Knob; 4] = [
        Knob::WakeLines,
        Knob::EntryChars,
        Knob::PartChars,
        Knob::PartLines,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Knob::WakeLines => "WAKE_LINES",
            Knob::EntryChars => "ENTRY_CHARS",
            Knob::PartChars => "PART_CHARS",
            Knob::PartLines => "PART_LINES",
        }
    }

    pub fn default(self) -> u64 {
        match self {
            Knob::WakeLines => 96,
            Knob::EntryChars => 280,
            Knob::PartChars => 20000,
            Knob::PartLines => 500,
        }
    }

    pub fn what(self) -> &'static str {
        match self {
            Knob::WakeLines => "the memory context: how many lines wake prints",
            Knob::EntryChars => "the longest one memory may be, in bytes",
            Knob::PartChars => "output paging: largest part, in bytes",
            Knob::PartLines => "output paging: largest part, in lines",
        }
    }

    pub fn parse(name: &str) -> Option<Knob> {
        Knob::ALL.into_iter().find(|k| k.name() == name)
    }

    pub fn names() -> String {
        Knob::ALL.map(Knob::name).join(", ")
    }

    pub fn validate(self, v: &str, place: &str) -> Result<u64> {
        let n = match v.parse::<u64>() {
            Ok(n) if n >= 1 && v.bytes().all(|b| b.is_ascii_digit()) => n,
            _ => bail!(
                "{place}{} must be a positive whole number, not '{v}'.",
                self.name()
            ),
        };
        let top = (TREE_REC - 8).min(LOG_REC - 40);
        if self == Knob::EntryChars && n > top {
            bail!(
                "{place}ENTRY_CHARS is at most {top}: a memory has to fit the fixed-width records."
            );
        }
        Ok(n)
    }
}

/// The sizes one memory overrides; every other knob follows the tool's default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config([Option<u64>; 4]);

impl Config {
    pub fn get(&self, k: Knob) -> u64 {
        self.overridden(k).unwrap_or(k.default())
    }

    pub fn overridden(&self, k: Knob) -> Option<u64> {
        self.0[k as usize]
    }

    pub fn set(&mut self, k: Knob, v: Option<u64>) {
        self.0[k as usize] = v;
    }

    pub fn load(dir: &Path) -> Result<Config> {
        let p = dir.join("config");
        let mut cfg = Config::default();
        if !p.exists() {
            return Ok(cfg);
        }
        for (n, line) in fs::read_to_string(&p).at(&p)?.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim().to_uppercase(), v.trim());
            let place = format!("{} line {}: ", pretty(&p), n + 1);
            let Some(knob) = Knob::parse(&k) else {
                bail!(
                    "{place}{k} is not a size. Delete the line, or name one of: {}.",
                    Knob::names()
                );
            };
            cfg.set(knob, Some(knob.validate(v, &place)?));
        }
        Ok(cfg)
    }

    pub fn write(&self, dir: &Path) -> Result<()> {
        let mut out = format!(
            "# Sizes for this memory. A commented line means: follow the\n\
             # tool's default. Edit with `{ME} config NAME=VALUE`.\n\n"
        );
        for k in Knob::ALL {
            let mark = if self.overridden(k).is_some() {
                ""
            } else {
                "# "
            };
            out += &format!(
                "{mark:<2}{:<12} = {:<6} # {}\n",
                k.name(),
                self.get(k),
                k.what()
            );
        }
        let p = dir.join("config");
        fs::write(&p, out).at(&p)
    }
}
