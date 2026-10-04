use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use gix::{
    ObjectId,
    lock::acquire::Fail,
    objs::tree::EntryKind,
    refs::{
        Target,
        transaction::{PreviousValue, RefEdit},
    },
};

use crate::{
    config::{Config, SECTION, SUBSECTION},
    cover::Block,
    record::{Memory, REC, Summary, Who, crockford, fingerprint, next_ts, now},
};

pub const ME: &str = "ai memory";
pub const REF: &str = "refs/ai/memory";
pub const LOG: &str = "log";
const SEG: u64 = 256;
const ORIGIN: &str = "origin";

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

/// Segment `n` of the level stored under `dir`.
pub fn seg_path(dir: &str, n: u64) -> String {
    format!("{dir}/{:04x}/{:02x}", n >> 8, n & 255)
}

pub fn level(size: u64) -> String {
    format!("tree/{size}")
}

/// New segment blobs by path; an empty one removes the path.
#[derive(Debug, Default)]
pub struct Changes(BTreeMap<String, Vec<u8>>);

impl Changes {
    pub fn put(&mut self, path: String, bytes: Vec<u8>) {
        self.0.insert(path, bytes);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Put {
    Saved,
    Moved,
    Changed,
}

pub struct Store {
    repo: gix::Repository,
    dir: PathBuf,
    pub cfg: Config,
}

impl Store {
    /// Opens an existing memory; only [`Store::create`] makes one.
    pub fn open(dir: &Path) -> Result<Store> {
        match Store::find(dir)? {
            Some(s) => Ok(s),
            None => bail!(
                "No memory at {}.\nTo create one, run: {ME} init\n\
                 To use an existing one, point AI_MEMORY_DIR at it.",
                dir.display()
            ),
        }
    }

    /// Opens the memory at `dir`, creating it if absent; returns whether it was new.
    pub fn create(dir: &Path) -> Result<(Store, bool)> {
        if let Some(s) = Store::find(dir)? {
            return Ok((s, false));
        }
        if fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
            bail!(
                "{} is not a memory, and not empty. Point AI_MEMORY_DIR elsewhere.",
                pretty(dir)
            );
        }
        fs::create_dir_all(dir).at(dir)?;
        gix::init_bare(dir)?;
        Ok((Store::open(dir)?, true))
    }

    fn find(dir: &Path) -> Result<Option<Store>> {
        if !dir.is_dir() {
            return Ok(None);
        }
        let Ok(mut repo) = gix::open(dir) else {
            return Ok(None);
        };
        if !repo.is_bare() {
            return Ok(None);
        }
        repo.committer_or_set_fallback(ME, "ai@localhost")?;
        let cfg = Config::load(&repo.config_snapshot(), &pretty(&dir.join("config")))?;
        Ok(Some(Store {
            repo,
            dir: dir.to_path_buf(),
            cfg,
        }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A value from this memory's resolved git config.
    pub fn setting(&self, key: &str) -> Option<String> {
        self.repo
            .config_snapshot()
            .string(key)
            .map(|v| v.to_string())
            .filter(|v| !v.is_empty())
    }

    /// The commit `name` points at, if the ref exists.
    pub fn resolve(&self, name: &str) -> Result<Option<ObjectId>> {
        Ok(self
            .repo
            .try_find_reference(name)?
            .and_then(|r| r.try_id().map(|id| id.detach())))
    }

    pub fn head(&self) -> Result<Option<ObjectId>> {
        self.resolve(REF)
    }

    /// Whether `a` is `b` or one of its ancestors.
    pub fn is_ancestor(&self, a: ObjectId, b: ObjectId) -> Result<bool> {
        Ok(self
            .repo
            .rev_walk([a])
            .with_hidden([b])
            .all()?
            .next()
            .is_none())
    }

    /// The memory as of the latest commit.
    pub fn snapshot(&self) -> Result<Snapshot<'_>> {
        self.at(self.head()?)
    }

    /// The memory as of `commit`; `None` is the empty memory.
    pub fn at(&self, commit: Option<ObjectId>) -> Result<Snapshot<'_>> {
        let tree = match commit {
            Some(c) => self.repo.find_commit(c)?.tree_id()?.detach(),
            None => ObjectId::empty_tree(self.repo.object_hash()),
        };
        Ok(Snapshot {
            store: self,
            commit,
            tree,
            cache: RefCell::default(),
        })
    }

    /// Applies `change` to the latest snapshot as one commit, redoing it until the ref update wins.
    pub fn mutate<R>(
        &self,
        msg: &str,
        mut change: impl FnMut(&Snapshot) -> Result<(R, Changes)>,
    ) -> Result<(R, Snapshot<'_>)> {
        loop {
            let base = self.snapshot()?;
            let (r, ch) = change(&base)?;
            if ch.0.is_empty() {
                return Ok((r, base));
            }
            let commit = self.commit(&base, ch, msg, base.commit)?;
            if self.cas(base.commit, commit, msg)? {
                return Ok((r, self.at(Some(commit))?));
            }
        }
    }

    /// Writes `base` with `ch` applied as a new commit; moves no ref.
    pub fn commit(
        &self,
        base: &Snapshot,
        Changes(ch): Changes,
        msg: &str,
        parents: impl IntoIterator<Item = ObjectId>,
    ) -> Result<ObjectId> {
        let mut ed = self.repo.edit_tree(base.tree)?;
        for (p, bytes) in ch {
            if bytes.is_empty() {
                ed.remove(&p)?;
            } else {
                let id = self.repo.write_blob(bytes)?;
                ed.upsert(&p, EntryKind::Blob, id)?;
            }
        }
        let tree = ed.write()?.detach();
        let me = self.repo.committer().context("No committer.")??;
        Ok(self.repo.new_commit_as(me, me, msg, tree, parents)?.id)
    }

    /// Moves the memory from `from` to `to`; false if another writer moved it first.
    pub fn cas(&self, from: Option<ObjectId>, to: ObjectId, msg: &str) -> Result<bool> {
        let expected = match from {
            Some(c) => PreviousValue::MustExistAndMatch(Target::Object(c)),
            None => PreviousValue::MustNotExist,
        };
        let edit = RefEdit::update(REF.try_into()?, to, expected, msg);
        match self.repo.edit_reference(edit) {
            Ok(_) => Ok(true),
            Err(_) if self.head()? != from => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn edit_config<R>(&self, edit: impl FnOnce(&mut gix::config::File) -> Result<R>) -> Result<R> {
        let p = self.dir.join("config");
        let wait = Fail::AfterDurationWithBackoff(Duration::from_secs(3));
        let mut lock = gix::lock::File::acquire_to_update_resource(&p, wait, None)?;
        let mut file =
            gix::config::File::from_path_no_includes(p.clone(), gix::config::Source::Local)?;
        let r = edit(&mut file)?;
        file.write_to(&mut lock).at(&p)?;
        lock.commit().map_err(|e| e.error).at(&p)?;
        Ok(r)
    }

    pub fn save_config(&self, cfg: &Config) -> Result<()> {
        self.edit_config(|f| cfg.save(f))
    }

    /// This clone's id, created on first use.
    pub fn origin(&self) -> Result<String> {
        let key = format!("{SECTION}.{SUBSECTION}.{ORIGIN}");
        if let Some(o) = self.setting(&key) {
            return Ok(o);
        }
        self.edit_config(|f| {
            if let Some(o) = f.string_by(SECTION, Some(SUBSECTION.into()), ORIGIN) {
                return Ok(o.to_string());
            }
            let o = crockford(getrandom::u32()?, 6);
            f.set_raw_value_by(SECTION, Some(SUBSECTION.into()), ORIGIN, o.as_str())?;
            Ok(o)
        })
    }

    /// Appends `items` in order; each `ts` is the wanted time, kept only if it sorts after the tail.
    pub fn log_append(&self, msg: &str, items: &[Memory]) -> Result<(u64, Snapshot<'_>)> {
        let origin = self.origin()?;
        self.mutate(msg, |s| {
            let base = s.log_len()?;
            let mut tail = match base {
                0 => None,
                n => Some(s.log_get(n - 1)?),
            };
            let mut recs = Vec::new();
            for m in items {
                let key = tail.as_ref().map(|t| (t.ts.as_str(), t.origin.as_str()));
                let m = Memory {
                    ts: next_ts(&m.ts, key, &origin)?,
                    origin: origin.clone(),
                    ..m.clone()
                };
                recs.push(m.encode()?);
                tail = Some(m);
            }
            let mut ch = Changes::default();
            s.append(LOG, base, recs, &mut ch)?;
            Ok((base, ch))
        })
    }

    /// Writes block `[lo, hi)` if it is the next one at its level and its fingerprint starts with `fp`.
    pub fn tree_put(
        &self,
        (lo, hi): Block,
        fp: &str,
        text: &str,
        who: &Who,
    ) -> Result<(Put, Snapshot<'_>)> {
        let (origin, size) = (self.origin()?, hi - lo);
        self.mutate(&format!("nap {lo}-{}", hi - 1), |s| {
            let mut ch = Changes::default();
            if s.level_len(size)? != lo / size {
                return Ok((Put::Moved, ch));
            }
            let have = s.fingerprint(lo, hi)?;
            if !have.starts_with(fp) {
                return Ok((Put::Changed, ch));
            }
            let rec = Summary {
                ts: now(),
                origin: origin.clone(),
                fp: have,
                who: who.clone(),
                text: text.into(),
            };
            s.append(&level(size), lo / size, [rec.encode()?], &mut ch)?;
            Ok((Put::Saved, ch))
        })
    }

    /// Drops block `[lo, hi)` and every block built from it; returns them.
    pub fn tree_drop(&self, lo: u64, hi: u64) -> Result<(Vec<Block>, Snapshot<'_>)> {
        self.mutate(&format!("forget {lo}-{}", hi - 1), |s| {
            let (mut gone, mut size, mut ch) = (Vec::new(), hi - lo, Changes::default());
            let t = s.log_len()?;
            while size <= t {
                let (k, n) = (lo / size, s.level_len(size)?);
                if n > k {
                    gone.extend((k..n).map(|i| (i * size, (i + 1) * size)));
                    s.truncate(&level(size), k, n, &mut ch)?;
                }
                size *= 2;
            }
            Ok((gone, ch))
        })
    }
}

fn summary((lo, hi): Block, sum: Option<Summary>) -> Result<Option<Summary>> {
    let Some(sum) = sum else {
        bail!(
            "The summary of #{lo}-{} is corrupt. Run: {ME} forget {lo}-{}",
            hi - 1,
            hi - 1
        );
    };
    Ok((!sum.text.is_empty()).then_some(sum))
}

/// The memory as of one commit: every read through it sees the same state.
pub struct Snapshot<'s> {
    store: &'s Store,
    commit: Option<ObjectId>,
    tree: ObjectId,
    cache: RefCell<HashMap<String, Rc<[u8]>>>,
}

impl Snapshot<'_> {
    pub fn commit(&self) -> Option<ObjectId> {
        self.commit
    }

    pub fn cfg(&self) -> &Config {
        &self.store.cfg
    }

    pub fn tree(&self) -> ObjectId {
        self.tree
    }

    fn entry(&self, path: &str) -> Result<Option<ObjectId>> {
        let root = self.store.repo.find_tree(self.tree)?;
        Ok(root.lookup_entry_by_path(path)?.map(|e| e.object_id()))
    }

    fn fetch(&self, path: &str) -> Result<Vec<u8>> {
        Ok(match self.entry(path)? {
            Some(id) => self.store.repo.find_blob(id)?.take_data(),
            None => Vec::new(),
        })
    }

    /// The blob at `path`, empty if absent.
    pub fn read(&self, path: &str) -> Result<Rc<[u8]>> {
        if let Some(b) = self.cache.borrow().get(path) {
            return Ok(b.clone());
        }
        let b: Rc<[u8]> = self.fetch(path)?.into();
        self.cache.borrow_mut().insert(path.into(), b.clone());
        Ok(b)
    }

    fn last(&self, tree: ObjectId) -> Result<Option<(u64, ObjectId)>> {
        let t = self.store.repo.find_tree(tree)?;
        let Some(e) = t
            .decode()?
            .entries
            .last()
            .map(|e| (e.filename.to_string(), e.oid.to_owned()))
        else {
            return Ok(None);
        };
        let n = u64::from_str_radix(&e.0, 16)
            .with_context(|| format!("A segment is misnamed: {}.", e.0))?;
        Ok(Some((n, e.1)))
    }

    fn count(&self, dir: &str) -> Result<u64> {
        let Some(top) = self.entry(dir)? else {
            return Ok(0);
        };
        let Some((hi, sub)) = self.last(top)? else {
            return Ok(0);
        };
        let Some((lo, seg)) = self.last(sub)? else {
            return Ok(0);
        };
        let n = hi << 8 | lo;
        let size = self.store.repo.find_header(seg)?.size();
        Ok(n * SEG + size / REC as u64)
    }

    pub fn log_len(&self) -> Result<u64> {
        self.count(LOG)
    }

    /// How many blocks of `size` are built.
    pub fn level_len(&self, size: u64) -> Result<u64> {
        self.count(&level(size))
    }

    fn record<R>(&self, dir: &str, i: u64, decode: impl FnOnce(&[u8]) -> R) -> Result<Option<R>> {
        let seg = self.read(&seg_path(dir, i / SEG))?;
        let at = (i % SEG) as usize * REC;
        Ok(seg.get(at..at + REC).map(decode))
    }

    pub fn log_get(&self, i: u64) -> Result<Memory> {
        self.record(LOG, i, Memory::decode)?
            .with_context(|| format!("Memory #{i} is missing from the log."))?
    }

    pub fn log_slice(&self, lo: u64, hi: u64) -> Result<Vec<Memory>> {
        (lo..hi).map(|i| self.log_get(i)).collect()
    }

    /// The fingerprint of block `[lo, hi)`'s members.
    pub fn fingerprint(&self, lo: u64, hi: u64) -> Result<String> {
        let keys: Vec<_> = self.log_slice(lo, hi)?.iter().map(Memory::key).collect();
        Ok(fingerprint(keys.iter().map(String::as_str)))
    }

    /// Every record under `dir`, concatenated.
    pub fn dump(&self, dir: &str) -> Result<Vec<u8>> {
        let mut all = Vec::new();
        for seg in 0..self.count(dir)?.div_ceil(SEG) {
            all.extend(self.fetch(&seg_path(dir, seg))?);
        }
        Ok(all)
    }

    /// Streams records `[0, end)` under `dir`, one segment at a time, without caching.
    fn scan(
        &self,
        dir: &str,
        end: u64,
        mut each: impl FnMut(u64, &[u8]) -> Result<()>,
    ) -> Result<()> {
        let end = end.min(self.count(dir)?);
        for seg in 0..end.div_ceil(SEG) {
            let buf = self.fetch(&seg_path(dir, seg))?;
            let (recs, _) = buf.as_chunks::<REC>();
            for (i, rec) in (seg * SEG..end).zip(recs) {
                each(i, rec)?;
            }
        }
        Ok(())
    }

    /// Streams memories `[0, end)` with their positions.
    pub fn log_scan(
        &self,
        end: u64,
        mut each: impl FnMut(u64, Memory) -> Result<()>,
    ) -> Result<()> {
        self.scan(LOG, end, |i, rec| each(i, Memory::decode(rec)?))
    }

    /// Streams the built summaries of `size` that end by `end`, skipping blank ones.
    pub fn tree_scan(
        &self,
        size: u64,
        end: u64,
        mut each: impl FnMut(Block, Summary) -> Result<()>,
    ) -> Result<()> {
        self.scan(&level(size), end / size, |k, rec| {
            let b = (k * size, (k + 1) * size);
            match summary(b, Summary::decode(rec))? {
                Some(sum) => each(b, sum),
                None => Ok(()),
            }
        })
    }

    /// The summary of block `[lo, hi)`, or `None` if it is not built yet.
    pub fn tree_get(&self, lo: u64, hi: u64) -> Result<Option<Summary>> {
        let size = hi - lo;
        match self.record(&level(size), lo / size, Summary::decode)? {
            Some(sum) => summary((lo, hi), sum),
            None => Ok(None),
        }
    }

    fn append(
        &self,
        dir: &str,
        start: u64,
        recs: impl IntoIterator<Item = Vec<u8>>,
        ch: &mut Changes,
    ) -> Result<()> {
        for (i, rec) in (start..).zip(recs) {
            let p = seg_path(dir, i / SEG);
            if !ch.0.contains_key(&p) {
                let cur = self.read(&p)?;
                ch.put(p.clone(), cur[..(i % SEG) as usize * REC].to_vec());
            }
            ch.0.entry(p).or_default().extend(rec);
        }
        Ok(())
    }

    /// Replaces the records under `dir` from `from` on (of `old` in all) with `recs`.
    pub fn replace(
        &self,
        dir: &str,
        from: u64,
        old: u64,
        recs: impl IntoIterator<Item = Vec<u8>>,
        ch: &mut Changes,
    ) -> Result<()> {
        if from < old {
            self.truncate(dir, from, old, ch)?;
        }
        self.append(dir, from, recs, ch)
    }

    fn truncate(&self, dir: &str, k: u64, n: u64, ch: &mut Changes) -> Result<()> {
        for seg in k / SEG..=(n - 1) / SEG {
            let keep = if seg == k / SEG {
                (k % SEG) as usize * REC
            } else {
                0
            };
            let p = seg_path(dir, seg);
            let cur = self.read(&p)?;
            ch.put(p, cur[..keep].to_vec());
        }
        Ok(())
    }
}
