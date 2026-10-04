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
    config::Config,
    record::{Message, Node, now},
    tree::{Coord, free, joined},
};

pub const ME: &str = "ai memory";
pub const REF: &str = "refs/ai/memory";
pub const LOG: &str = "log";
const SEG: u64 = 256;
const FREE: &str = "-";

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

/// Entry `n` under `dir`, fanned out 256 per tree.
pub fn fan_path(dir: &str, n: u64) -> String {
    format!("{dir}/{:04x}/{:02x}", n >> 8, n & 255)
}

pub fn level_dir(l: u32) -> String {
    format!("tree/{l}")
}

fn seg_of(c: Coord) -> (String, usize) {
    (fan_path(&level_dir(c.l), c.i / SEG), (c.i % SEG) as usize)
}

/// New blobs by path.
#[derive(Debug, Default)]
pub struct Changes(BTreeMap<String, Vec<u8>>);

impl Changes {
    pub fn put(&mut self, path: String, bytes: Vec<u8>) {
        self.0.insert(path, bytes);
    }
}

/// Nodes a mutation wrote, with their stored text.
pub type Built = Vec<(Coord, String)>;

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

    pub fn head(&self) -> Result<Option<ObjectId>> {
        Ok(self
            .repo
            .try_find_reference(REF)?
            .and_then(|r| r.try_id().map(|id| id.detach())))
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

    /// The commit a printed hex id names.
    pub fn commit_id(&self, hex: &str) -> Result<ObjectId> {
        let id = ObjectId::from_hex(hex.as_bytes())
            .ok()
            .filter(|id| self.repo.find_commit(*id).is_ok());
        id.with_context(|| format!("'{hex}' is not a commit of this memory. Run: {ME} wake"))
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
            let commit = self.commit(&base, ch, msg)?;
            if self.cas(base.commit, commit, msg)? {
                return Ok((r, self.at(Some(commit))?));
            }
        }
    }

    fn commit(&self, base: &Snapshot, Changes(ch): Changes, msg: &str) -> Result<ObjectId> {
        let mut ed = self.repo.edit_tree(base.tree)?;
        for (p, bytes) in ch {
            let id = self.repo.write_blob(bytes)?;
            ed.upsert(&p, EntryKind::Blob, id)?;
        }
        let tree = ed.write()?.detach();
        let me = self.repo.committer().context("No committer.")??;
        Ok(self.repo.new_commit_as(me, me, msg, tree, base.commit)?.id)
    }

    /// Moves the memory from `from` to `to`; false if another writer moved it first.
    fn cas(&self, from: Option<ObjectId>, to: ObjectId, msg: &str) -> Result<bool> {
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

    pub fn save_config(&self, cfg: &Config) -> Result<()> {
        let p = self.dir.join("config");
        let wait = Fail::AfterDurationWithBackoff(Duration::from_secs(3));
        let mut lock = gix::lock::File::acquire_to_update_resource(&p, wait, None)?;
        let mut file =
            gix::config::File::from_path_no_includes(p.clone(), gix::config::Source::Local)?;
        cfg.save(&mut file)?;
        file.write_to(&mut lock).at(&p)?;
        lock.commit().map_err(|e| e.error).at(&p)?;
        Ok(())
    }

    /// Appends `items` in order, with every free node they complete; returns the first new id.
    pub fn append(&self, msg: &str, items: &[Message]) -> Result<(u64, Built, Snapshot<'_>)> {
        let ((first, built), snap) = self.mutate(msg, |s| {
            let mut ed = Edit::new(s)?;
            let first = ed.t;
            for m in items {
                ed.push(m)?;
            }
            let (built, ch) = ed.finish();
            Ok(((first, built), ch))
        })?;
        Ok((first, built, snap))
    }

    /// Writes node `c` unless it is built, with every free node above it it completes.
    pub fn put_node(&self, c: Coord, model: &str, text: &str) -> Result<(Built, Snapshot<'_>)> {
        self.mutate(&format!("node {c}"), |s| {
            let mut ed = Edit::new(s)?;
            if c.end() > ed.t {
                bail!("Node {c} is beyond the log: it holds {} messages.", ed.t);
            }
            if ed.text(c)?.is_none() {
                ed.put(c, model, text)?;
            }
            Ok(ed.finish())
        })
    }
}

/// One mutation: new messages and nodes over a snapshot, free nodes completed as they become ready.
struct Edit<'a, 's> {
    snap: &'a Snapshot<'s>,
    t: u64,
    ch: Changes,
    segs: BTreeMap<String, Vec<String>>,
    built: Built,
}

impl<'a, 's> Edit<'a, 's> {
    fn new(snap: &'a Snapshot<'s>) -> Result<Self> {
        Ok(Edit {
            snap,
            t: snap.log_len()?,
            ch: Changes::default(),
            segs: BTreeMap::new(),
            built: Vec::new(),
        })
    }

    fn seg(&mut self, path: &str) -> Result<&mut Vec<String>> {
        if !self.segs.contains_key(path) {
            let lines = split(&self.snap.read(path)?)?;
            self.segs.insert(path.into(), lines);
        }
        Ok(self.segs.get_mut(path).expect("inserted above"))
    }

    fn text(&mut self, c: Coord) -> Result<Option<String>> {
        let (path, k) = seg_of(c);
        let line = self.seg(&path)?.get(k).cloned().unwrap_or_default();
        Ok(Node::decode(&line).map(|n| n.text))
    }

    fn put(&mut self, c: Coord, model: &str, text: &str) -> Result<()> {
        let node = Node {
            ts: now(),
            model: model.into(),
            text: text.into(),
        };
        let line = node.encode();
        let text = Node::decode(&line).context("An empty node.")?.text;
        let (path, k) = seg_of(c);
        let seg = self.seg(&path)?;
        if seg.len() <= k {
            seg.resize(k + 1, String::new());
        }
        seg[k] = line;
        self.built.push((c, text));
        self.cascade(c)
    }

    fn cascade(&mut self, c: Coord) -> Result<()> {
        let p = c.parent();
        if p.end() > self.t || self.text(p)?.is_some() {
            return Ok(());
        }
        let [a, b] = p.children();
        let (Some(a), Some(b)) = (self.text(a)?, self.text(b)?) else {
            return Ok(());
        };
        match free(joined(&a, &b)) {
            Some(text) => self.put(p, FREE, &text),
            None => Ok(()),
        }
    }

    fn push(&mut self, m: &Message) -> Result<()> {
        if m.text.is_empty() {
            bail!("An empty message.");
        }
        let i = self.t;
        self.ch.put(fan_path(LOG, i), m.encode());
        self.t += 1;
        match free(crate::record::flat(&m.label())) {
            Some(text) => self.put(Coord::leaf(i), FREE, &text),
            None => Ok(()),
        }
    }

    fn finish(mut self) -> (Built, Changes) {
        for (path, lines) in self.segs {
            if lines.is_empty() {
                continue;
            }
            let mut b = lines.join("\n").into_bytes();
            b.push(b'\n');
            self.ch.put(path, b);
        }
        (self.built, self.ch)
    }
}

fn split(seg: &[u8]) -> Result<Vec<String>> {
    let s = std::str::from_utf8(seg).map_err(|_| anyhow!("A tree segment is corrupt."))?;
    let mut lines: Vec<String> = s.split('\n').map(String::from).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    Ok(lines)
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

    fn entry(&self, path: &str) -> Result<Option<ObjectId>> {
        let root = self.store.repo.find_tree(self.tree)?;
        Ok(root.lookup_entry_by_path(path)?.map(|e| e.object_id()))
    }

    fn fetch(&self, id: ObjectId) -> Result<Vec<u8>> {
        Ok(self.store.repo.find_blob(id)?.take_data())
    }

    /// The blob at `path`, empty if absent; cached.
    pub fn read(&self, path: &str) -> Result<Rc<[u8]>> {
        if let Some(b) = self.cache.borrow().get(path) {
            return Ok(b.clone());
        }
        let b: Rc<[u8]> = match self.entry(path)? {
            Some(id) => self.fetch(id)?.into(),
            None => Rc::from([]),
        };
        self.cache.borrow_mut().insert(path.into(), b.clone());
        Ok(b)
    }

    /// A tree's entries by their hex names, in order.
    fn entries(&self, tree: ObjectId) -> Result<Vec<(u64, ObjectId)>> {
        let t = self.store.repo.find_tree(tree)?;
        t.decode()?
            .entries
            .iter()
            .map(|e| {
                let name = e.filename.to_string();
                let n = u64::from_str_radix(&name, 16)
                    .with_context(|| format!("An entry is misnamed: {name}."))?;
                Ok((n, e.oid.to_owned()))
            })
            .collect()
    }

    /// Every blob under the fanned-out `dir`, with its number.
    fn walk(&self, dir: &str, mut each: impl FnMut(u64, ObjectId) -> Result<()>) -> Result<()> {
        let Some(top) = self.entry(dir)? else {
            return Ok(());
        };
        for (hi, sub) in self.entries(top)? {
            for (lo, id) in self.entries(sub)? {
                each(hi << 8 | lo, id)?;
            }
        }
        Ok(())
    }

    /// How many messages the log holds.
    pub fn log_len(&self) -> Result<u64> {
        let Some(top) = self.entry(LOG)? else {
            return Ok(0);
        };
        let Some(&(hi, sub)) = self.entries(top)?.last() else {
            return Ok(0);
        };
        let Some(&(lo, _)) = self.entries(sub)?.last() else {
            return Ok(0);
        };
        Ok((hi << 8 | lo) + 1)
    }

    pub fn message(&self, i: u64) -> Result<Message> {
        let id = self
            .entry(&fan_path(LOG, i))?
            .with_context(|| format!("Message {i} is missing from the log."))?;
        Message::decode(&self.fetch(id)?).with_context(|| format!("Message {i}"))
    }

    /// Streams messages `[0, end)` in order.
    pub fn scan(&self, end: u64, mut each: impl FnMut(u64, Message) -> Result<()>) -> Result<()> {
        let mut want = 0;
        self.walk(LOG, |i, id| {
            if i >= end {
                return Ok(());
            }
            if i != want {
                bail!("Message {want} is missing from the log.");
            }
            want += 1;
            each(i, Message::decode(&self.fetch(id)?)?)
        })
    }

    /// Node `c`, if built.
    pub fn node(&self, c: Coord) -> Result<Option<Node>> {
        let (path, k) = seg_of(c);
        let seg = self.read(&path)?;
        Ok(split(&seg)?.get(k).and_then(|l| Node::decode(l)))
    }

    /// Every node of level `l`, by index; `None` where not built.
    pub fn level(&self, l: u32) -> Result<Vec<Option<Node>>> {
        let mut out = Vec::new();
        self.walk(&level_dir(l), |s, id| {
            for (k, line) in split(&self.fetch(id)?)?.iter().enumerate() {
                let i = (s * SEG) as usize + k;
                if out.len() <= i {
                    out.resize(i + 1, None);
                }
                out[i] = Node::decode(line);
            }
            Ok(())
        })?;
        Ok(out)
    }
}
