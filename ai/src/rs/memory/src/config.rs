use anyhow::{Result, bail};
use gix::bstr::ByteSlice;

pub const SECTION: &str = "ai";
pub const SUBSECTION: &str = "memory";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    PartChars,
    PartLines,
}

impl Knob {
    pub const ALL: [Knob; 2] = [Knob::PartChars, Knob::PartLines];

    pub fn name(self) -> &'static str {
        match self {
            Knob::PartChars => "PART_CHARS",
            Knob::PartLines => "PART_LINES",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Knob::PartChars => "partChars",
            Knob::PartLines => "partLines",
        }
    }

    pub fn default(self) -> u64 {
        match self {
            Knob::PartChars => 20000,
            Knob::PartLines => 500,
        }
    }

    pub fn what(self) -> &'static str {
        match self {
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

    pub fn validate(self, v: &str, label: &str) -> Result<u64> {
        match v.parse::<u64>() {
            Ok(n) if n >= 1 && v.bytes().all(|b| b.is_ascii_digit()) => Ok(n),
            _ => bail!("{label} must be a positive whole number, not '{v}'."),
        }
    }
}

/// The sizes one memory overrides; every other knob follows the tool's default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config([Option<u64>; 2]);

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

    /// Reads `ai.memory.<knob>` from `file`; `place` names where it is written.
    pub fn load(file: &gix::config::File, place: &str) -> Result<Config> {
        let mut cfg = Config::default();
        for k in Knob::ALL {
            if let Some(v) = file.string_by(SECTION, Some(SUBSECTION.into()), k.key()) {
                let label = format!("{place}: {SECTION}.{SUBSECTION}.{}", k.key());
                cfg.set(k, Some(k.validate(v.to_str_lossy().trim(), &label)?));
            }
        }
        Ok(cfg)
    }

    pub fn save(&self, file: &mut gix::config::File) -> Result<()> {
        for k in Knob::ALL {
            match self.overridden(k) {
                Some(v) => {
                    file.set_raw_value_by(
                        SECTION,
                        Some(SUBSECTION.into()),
                        k.key(),
                        v.to_string().as_str(),
                    )?;
                }
                None => {
                    if let Ok(mut s) = file.section_mut(SECTION, Some(SUBSECTION.into())) {
                        while s.remove(k.key()).is_some() {}
                    }
                }
            }
        }
        Ok(())
    }
}
