use std::{
    fmt, fs,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{config::Config, cover::Block};

pub const ME: &str = "ai memory";
pub const LOG_REC: u64 = 320;
pub const TREE_REC: u64 = 288;

/// A path as the user would type it: fold `$HOME` to `~`.
pub fn pretty(p: &Path) -> String {
    let p = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    match std::env::home_dir().and_then(|h| p.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

pub trait AtPath<T> {
    fn at(self, p: &Path) -> Result<T>;
}

impl<T> AtPath<T> for io::Result<T> {
    fn at(self, p: &Path) -> Result<T> {
        self.map_err(|e| {
            let msg = e.to_string();
            let reason = msg.split(" (os error").next().unwrap_or(&msg);
            anyhow!("{}: {reason}.", pretty(p))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub date: String,
    pub text: String,
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "#{} {} {}", self.id, self.date, self.text)
    }
}

impl Entry {
    fn decode(rec: &[u8]) -> Result<Entry> {
        let line = std::str::from_utf8(rec).context("A log record is not UTF-8.")?;
        let line = line.trim_end();
        let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
        let (date, text) = rest.split_once(' ').unwrap_or((rest, ""));
        let id = head
            .strip_prefix('#')
            .and_then(|h| h.parse().ok())
            .with_context(|| format!("A log record is corrupt: {line}"))?;
        Ok(Entry {
            id,
            date: date.into(),
            text: text.into(),
        })
    }
}

fn records(buf: &[u8]) -> Result<Vec<Entry>> {
    let (recs, _) = buf.as_chunks::<{ LOG_REC as usize }>();
    recs.iter().map(|r| Entry::decode(r)).collect()
}

fn pad(text: &str, rec: u64) -> Result<Vec<u8>> {
    let room = rec as usize - 1;
    let mut b = text.as_bytes().to_vec();
    if b.len() > room {
        bail!("Too long: {} bytes. The record holds {room}.", b.len());
    }
    b.resize(room, b' ');
    b.push(b'\n');
    Ok(b)
}

fn count(p: &Path, rec: u64) -> Result<u64> {
    match fs::metadata(p) {
        Ok(m) => Ok(m.len() / rec),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e).at(p),
    }
}

fn repair(p: &Path, rec: u64) -> Result<()> {
    let n = match fs::metadata(p) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).at(p),
    };
    if n % rec != 0 {
        File::options()
            .write(true)
            .open(p)
            .and_then(|f| f.set_len(n - n % rec))
            .at(p)?;
    }
    Ok(())
}

fn read_at(f: &mut File, offset: u64, len: u64, p: &Path) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    f.seek(SeekFrom::Start(offset)).at(p)?;
    f.take(len).read_to_end(&mut buf).at(p)?;
    Ok(buf)
}

fn append_synced(p: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = File::options().append(true).create(true).open(p).at(p)?;
    f.write_all(bytes).and_then(|()| f.sync_all()).at(p)
}

pub struct Store {
    dir: PathBuf,
    pub cfg: Config,
}

impl Store {
    /// Opens an existing memory; only [`Store::create`] makes the directory.
    pub fn open(dir: &Path) -> Result<Store> {
        if !dir.is_dir() {
            bail!(
                "No memory at {}.\nTo create one, run: {ME} init\n\
                 To use an existing one, point AI_MEMORY_DIR at it.",
                dir.display()
            );
        }
        Store::ensure(dir)?;
        Store::load(dir)
    }

    /// Creates whatever of the memory is missing; returns whether it was new.
    pub fn create(dir: &Path) -> Result<(Store, bool)> {
        let fresh = !dir.is_dir();
        Store::ensure(dir)?;
        if !dir.join("config").exists() {
            Config::default().write(dir)?;
        }
        Ok((Store::load(dir)?, fresh))
    }

    fn ensure(dir: &Path) -> Result<()> {
        let tree = dir.join("TREE");
        fs::create_dir_all(&tree).at(&tree)?;
        let log = dir.join("LOG.txt");
        File::options()
            .append(true)
            .create(true)
            .open(&log)
            .at(&log)?;
        Ok(())
    }

    fn load(dir: &Path) -> Result<Store> {
        Ok(Store {
            dir: dir.to_path_buf(),
            cfg: Config::load(dir)?,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn log_path(&self) -> PathBuf {
        self.dir.join("LOG.txt")
    }

    fn tree_path(&self, size: u64) -> PathBuf {
        self.dir.join("TREE").join(size.to_string())
    }

    fn lock(&self) -> Result<File> {
        let p = self.dir.join(".lock");
        let f = File::options().append(true).create(true).open(&p).at(&p)?;
        f.lock().at(&p)?;
        Ok(f)
    }

    pub fn log_len(&self) -> Result<u64> {
        count(&self.log_path(), LOG_REC)
    }

    /// How many blocks of `size` are built.
    pub fn level_len(&self, size: u64) -> Result<u64> {
        count(&self.tree_path(size), TREE_REC)
    }

    pub fn log_slice(&self, lo: u64, hi: u64) -> Result<Vec<Entry>> {
        let p = self.log_path();
        let mut f = File::open(&p).at(&p)?;
        records(&read_at(&mut f, lo * LOG_REC, (hi - lo) * LOG_REC, &p)?)
    }

    pub fn log_get(&self, i: u64) -> Result<Entry> {
        self.log_slice(i, i + 1)?
            .pop()
            .with_context(|| format!("Memory #{i} is missing from the log."))
    }

    /// Streams every memory, in order, without holding the log.
    pub fn log_scan(&self, mut each: impl FnMut(Entry) -> Result<()>) -> Result<()> {
        let p = self.log_path();
        let mut f = File::open(&p).at(&p)?;
        let mut buf = Vec::new();
        loop {
            buf.clear();
            (&mut f).take(LOG_REC * 4096).read_to_end(&mut buf).at(&p)?;
            if buf.is_empty() {
                return Ok(());
            }
            records(&buf)?.into_iter().try_for_each(&mut each)?;
        }
    }

    /// The summary of block `[lo, hi)`, or `None` if it is not built yet.
    pub fn tree_get(&self, lo: u64, hi: u64) -> Result<Option<String>> {
        let size = hi - lo;
        let p = self.tree_path(size);
        let mut f = match File::open(&p) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).at(&p),
        };
        let rec = read_at(&mut f, lo / size * TREE_REC, TREE_REC, &p)?;
        let Ok(text) = String::from_utf8(rec) else {
            bail!(
                "The summary of #{lo}-{} is corrupt. Run: {ME} forget {lo}-{}",
                hi - 1,
                hi - 1
            );
        };
        let text = text.trim_end();
        Ok((!text.is_empty()).then(|| text.to_string()))
    }

    /// Appends `(date, text)` memories under the lock; returns the first id.
    pub fn log_append(&self, items: &[(String, String)]) -> Result<u64> {
        let _lock = self.lock()?;
        let p = self.log_path();
        repair(&p, LOG_REC)?;
        let base = self.log_len()?;
        let mut bytes = Vec::new();
        for (k, (date, text)) in (base..).zip(items) {
            bytes.extend(pad(&format!("#{k} {date} {text}"), LOG_REC)?);
        }
        append_synced(&p, &bytes)?;
        Ok(base)
    }

    /// Writes block `[lo, hi)` if it is the next one at its level.
    pub fn tree_put(&self, lo: u64, hi: u64, text: &str) -> Result<bool> {
        let size = hi - lo;
        let _lock = self.lock()?;
        let p = self.tree_path(size);
        repair(&p, TREE_REC)?;
        if count(&p, TREE_REC)? != lo / size {
            return Ok(false);
        }
        append_synced(&p, &pad(text, TREE_REC)?)?;
        Ok(true)
    }

    /// Drops block `[lo, hi)` and every block built from it; returns them.
    pub fn tree_drop(&self, lo: u64, hi: u64) -> Result<Vec<Block>> {
        let (mut gone, mut size) = (Vec::new(), hi - lo);
        let _lock = self.lock()?;
        while size <= self.log_len()? {
            let (p, k) = (self.tree_path(size), lo / size);
            let n = count(&p, TREE_REC)?;
            if n > k {
                gone.extend((k..n).map(|i| (i * size, (i + 1) * size)));
                File::options()
                    .write(true)
                    .open(&p)
                    .and_then(|f| f.set_len(k * TREE_REC))
                    .at(&p)?;
            }
            size *= 2;
        }
        Ok(gone)
    }
}
