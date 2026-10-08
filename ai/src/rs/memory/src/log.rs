use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

use anyhow::Result;
use tracing::{level_filters::LevelFilter, subscriber::DefaultGuard};
use tracing_subscriber::EnvFilter;

use crate::store::AtPath;

pub const NAP_LOG: &str = "nap.log";

/// `nap.log` in the memory `dir`, opened for appending.
pub fn open(dir: &Path) -> Result<File> {
    let p = dir.join(NAP_LOG);
    OpenOptions::new().create(true).append(true).open(&p).at(&p)
}

/// Sends this thread's events to `nap.log` in `dir`, filtered by `$AI_MEMORY_LOG`
/// (default `info`), until the guard drops.
pub fn init(dir: &Path) -> Result<DefaultGuard> {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .with_env_var("AI_MEMORY_LOG")
        .from_env_lossy();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(open(dir)?)
        .with_ansi(false)
        .with_env_filter(filter)
        .finish();
    Ok(tracing::subscriber::set_default(subscriber))
}

/// `d` in seconds, to the millisecond.
pub fn secs(d: Duration) -> f64 {
    (d.as_secs_f64() * 1e3).round() / 1e3
}
