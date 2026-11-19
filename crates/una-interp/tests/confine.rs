//! `Files` confinement (`spec/14 §3`, ADR 0038 D9, #1343): `confine` resolves each
//! intermediate directory from the one before it, one component at a time, with the outcomes
//! of the earlier algorithm, which resolved every lexical prefix again from the grant prefix.
//!
//! - A differential test keeps the earlier algorithm here and compares it with `confine` over
//!   every path of up to three components (and four, over the link components) built from
//!   names that are files, directories, links inside and outside the prefix, relative and
//!   absolute, dangling links, a link loop, `.` and `..`, under five prefixes: a directory, a
//!   link to it, a nested directory, a missing one and a dangling link. It runs on the machine's
//!   file system with the hosts' step ([`disk_step`]) and with the default step of `World`, and
//!   on an in-memory model of the same tree, whose outcomes must equal the disk's.
//! - The scaling test of ADR 0132 rule 16 counts the lookups of components in a counting
//!   world: paths of c and 10c components take at most 20 times as many lookups, with exact
//!   counts asserted, and the earlier algorithm is shown to exceed that bound.
//! - #1459: an intermediate directory confirmed absent is answered by confinement
//!   (`Confined::Absent`), and no outcome carries a path that confinement did not resolve: every
//!   path returned is a canonical directory inside the canonical prefix joined with the final
//!   component.

// Tests may unwrap and index freely (compiler/conventions.md §1). Links need Unix.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use una_interp::{Absence, Confined, FileFault, NotText, World, confine, disk_absent, disk_step};

/// The `World` methods confinement never calls.
macro_rules! no_io {
    () => {
        fn print(&mut self, _line: &str) {}
        fn now(&mut self) -> i64 {
            0
        }
        fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
            Ok(None)
        }
        fn read_file(&mut self, _path: &Path) -> Result<String, FileFault> {
            Err(FileFault::NotFound)
        }
        fn write_file(&mut self, _path: &Path, _content: &str) -> Result<(), FileFault> {
            Err(FileFault::Io)
        }
    };
}

/// The algorithm before #1343, kept as the oracle: every intermediate directory is resolved
/// by canonicalizing the whole lexical prefix up to it, O(c²) lookups for c components.
fn confine_quadratic(world: &mut dyn World, prefix: &Path, relative: &str) -> Confined {
    let mut parts: Vec<&OsStr> = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Confined::Denied;
                }
            }
            Component::RootDir | Component::Prefix(_) => return Confined::Denied,
        }
    }
    let Some((last, directories)) = parts.split_last() else {
        return Confined::Denied;
    };
    let mut lexical = prefix.to_path_buf();
    let Some(root) = world.canonical(prefix) else {
        return quadratic_missing(world, &lexical);
    };
    let mut resolved = root.clone();
    for part in directories {
        lexical.push(part);
        match world.canonical(&lexical) {
            Some(directory) if directory.starts_with(&root) => resolved = directory,
            Some(_) => return Confined::Denied,
            None => return quadratic_missing(world, &lexical),
        }
    }
    let target = resolved.join(last);
    if world.is_link(&target) {
        Confined::Denied
    } else {
        Confined::Path(target)
    }
}

fn quadratic_missing(world: &mut dyn World, lexical: &Path) -> Confined {
    // #1380: only a directory that does not exist is absence. #1459: absence is answered
    // here, and no path is left to the operation.
    if world.is_link(lexical) {
        return Confined::Denied;
    }
    world
        .absent(lexical)
        .map_or(Confined::Denied, Confined::Absent)
}

/// The path that the rule before #1459 left to the operation for an absent directory: the
/// grant prefix joined with the path as written, which the operation then opened.
fn lexical(prefix: &Path, relative: &str) -> PathBuf {
    prefix.join(relative)
}

/// The directory of a returned path is canonical on the disk and lies inside the canonical
/// form of `prefix` (#1459): the world is given no path that confinement did not resolve.
fn assert_resolved(outcome: &Confined, prefix: &Path, context: &str) {
    if let Confined::Path(path) = outcome {
        let root = std::fs::canonicalize(prefix).unwrap();
        let directory = path.parent().unwrap();
        assert_eq!(
            std::fs::canonicalize(directory).unwrap(),
            directory,
            "{context}"
        );
        assert!(directory.starts_with(&root), "{context}");
    }
}

/// The tree both file systems hold, under a root `{R}`: `(path, kind)`, where a kind is `dir`,
/// `file` or a link's target (`{R}` standing for the root in absolute targets).
const TREE: &[(&str, &str)] = &[
    ("box", "dir"),
    ("box/note.txt", "file"),
    ("box/sub", "dir"),
    ("box/sub/inner.txt", "file"),
    ("box/sub/deeper", "dir"),
    ("box/sub/deeper/x.txt", "file"),
    ("box/sub/up2", "->.."),
    ("box/sub/out2", "->../../outside"),
    ("box/in_dir", "->sub"),
    ("box/in_abs", "->{R}/box/sub"),
    ("box/out_dir", "->../outside"),
    ("box/out_abs", "->{R}/outside"),
    ("box/dangling", "->missing_target"),
    ("box/dangling_dir", "->../outside/nope"),
    ("box/in_link", "->note.txt"),
    ("box/up", "->.."),
    ("box/self", "->."),
    ("box/back", "->../box"),
    ("box/loop", "->loop"),
    ("box/chain", "->in_dir/deeper"),
    ("outside", "dir"),
    ("outside/secret.txt", "file"),
    ("outside/dir", "dir"),
    ("outside/back_in", "->{R}/box/sub"),
    ("outside/inner", "dir"),
    ("outside/inner/back", "->../../box"),
    ("boxlink", "->box"),
    ("dangling_prefix", "->{R}/nowhere"),
];

/// The prefixes of the grants tried, under the root.
const PREFIXES: &[&str] = &["box", "boxlink", "box/sub", "nothing", "dangling_prefix"];

/// The names paths are built from.
const NAMES: &[&str] = &[
    "note.txt",
    "sub",
    "inner.txt",
    "deeper",
    "x.txt",
    "up2",
    "out2",
    "in_dir",
    "in_abs",
    "out_dir",
    "out_abs",
    "dangling",
    "dangling_dir",
    "in_link",
    "up",
    "self",
    "back",
    "loop",
    "chain",
    "secret.txt",
    "back_in",
    "inner",
    "box",
    "missing",
    ".",
    "..",
];

/// The names of the longer paths: links and the components that move between directories.
const LINK_NAMES: &[&str] = &[
    "sub", "in_dir", "out_dir", "up", "self", "back", "up2", "chain", "..",
];

/// Every path of `names` joined by `/`, of 0 to `max` components, and a few written forms
/// that normalize: absolute paths, doubled and trailing separators, `.` steps.
fn paths(names: &[&str], max: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..max {
        let mut next = Vec::new();
        for base in &layer {
            for name in names {
                next.push(if base.is_empty() {
                    (*name).to_owned()
                } else {
                    format!("{base}/{name}")
                });
            }
        }
        out.extend(next.iter().cloned());
        layer = next;
    }
    out.extend(
        [
            "/etc/hosts",
            "/",
            "sub//inner.txt",
            "./note.txt",
            "sub/./inner.txt",
            "sub/",
            "in_dir/",
            "a/../../box/note.txt",
        ]
        .map(str::to_owned),
    );
    out
}

/// A fresh directory for one test under the target's temporary directory.
fn fresh(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("confine")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

/// The machine's file system, as the worlds of `una run` and of native executables see it.
struct Disk;

impl World for Disk {
    no_io!();
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }
    fn is_link(&mut self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    }
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        disk_step(directory, name)
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        disk_absent(path)
    }
}

/// The machine's file system with the default step of `World`.
struct DiskDefault;

impl World for DiskDefault {
    no_io!();
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }
    fn is_link(&mut self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        disk_absent(path)
    }
}

/// `world` with the rule before #1380: every directory that does not resolve and is not a
/// link counts as absent, whatever stopped the resolution. Before #1459 its lexical path was
/// then returned and opened; now such a world gets `NotFound` and no path.
struct Before<'a>(&'a mut dyn World);

impl World for Before<'_> {
    no_io!();
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        self.0.canonical(path)
    }
    fn is_link(&mut self, path: &Path) -> bool {
        self.0.is_link(path)
    }
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        self.0.step(directory, name)
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        self.0.absent(path).or(Some(Absence::Missing))
    }
}

enum Node {
    Dir(BTreeMap<String, usize>),
    File,
    Link(String),
}

/// An in-memory file system with the semantics of `realpath` and `lstat`, counting each
/// lookup of a name in a directory. Its step looks up only the new name in the directory
/// already resolved (as an `openat` from a held directory would), and resolves a link's
/// target from there. Its canonicalization fails, as `realpath` does, when a path it
/// resolves grows longer than `max_path` bytes (#1380); its step and `lstat` count no length.
struct Model {
    nodes: Vec<Node>,
    lookups: u64,
    max_path: usize,
}

/// Why a walk of the model stops.
#[derive(Debug, PartialEq, Eq)]
enum Fault {
    /// A name is missing (`ENOENT`).
    Missing,
    /// A name is looked up in a file (`ENOTDIR`).
    NotDir,
    /// More than 40 links (`ELOOP`).
    Loop,
    /// A resolved path longer than `max_path` (`ENAMETOOLONG`).
    TooLong,
}

/// A directory being walked: the names and nodes from the root, the root first.
type Walk = Vec<(String, usize)>;

impl Model {
    fn new() -> Self {
        Model {
            nodes: vec![Node::Dir(BTreeMap::new())],
            lookups: 0,
            max_path: usize::MAX,
        }
    }

    /// Adds `node` at the absolute `path`, whose parent directories are created as needed.
    fn add(&mut self, path: &str, node: Node) {
        let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let last = parts.pop().unwrap();
        let mut at = 0;
        for part in parts {
            let next = match &self.nodes[at] {
                Node::Dir(children) => children.get(part).copied(),
                _ => panic!("{path}: not a directory"),
            };
            at = match next {
                Some(next) => next,
                None => self.insert(at, part, Node::Dir(BTreeMap::new())),
            };
        }
        self.insert(at, last, node);
    }

    fn insert(&mut self, at: usize, name: &str, node: Node) -> usize {
        self.nodes.push(node);
        let index = self.nodes.len() - 1;
        let Node::Dir(children) = &mut self.nodes[at] else {
            panic!("not a directory");
        };
        children.insert(name.to_owned(), index);
        index
    }

    /// The entry `name` of the directory `at`, one lookup when `counted`; `Err` when `at` is
    /// not a directory.
    fn entry(&mut self, at: usize, name: &str, counted: bool) -> Result<Option<usize>, ()> {
        let Node::Dir(children) = &self.nodes[at] else {
            return Err(());
        };
        let child = children.get(name).copied();
        if counted {
            self.lookups += 1;
        }
        Ok(child)
    }

    /// Walks `pending` from `walk`, following links (the last one only when `follow_last`),
    /// with at most 40 links followed, as the hosts do, and resolved paths of at most `limit`
    /// bytes.
    fn walk(
        &mut self,
        mut walk: Walk,
        mut pending: VecDeque<String>,
        follow_last: bool,
        counted: bool,
        limit: usize,
    ) -> Result<Walk, Fault> {
        let mut hops = 0;
        while let Some(name) = pending.pop_front() {
            match name.as_str() {
                "" | "." => continue,
                ".." => {
                    if walk.len() > 1 {
                        walk.pop();
                    }
                    continue;
                }
                _ => {}
            }
            let at = walk.last().unwrap().1;
            let child = self
                .entry(at, &name, counted)
                .map_err(|()| Fault::NotDir)?
                .ok_or(Fault::Missing)?;
            match &self.nodes[child] {
                Node::Link(target) if follow_last || !pending.is_empty() => {
                    hops += 1;
                    if hops > 40 {
                        return Err(Fault::Loop);
                    }
                    if target.starts_with('/') {
                        walk.truncate(1);
                    }
                    for part in target.split('/').rev() {
                        pending.push_front(part.to_owned());
                    }
                }
                _ => {
                    walk.push((name, child));
                    if Model::path_of(&walk).as_os_str().len() > limit {
                        return Err(Fault::TooLong);
                    }
                }
            }
        }
        Ok(walk)
    }

    /// `realpath`: the walk of `path` with its links followed and its length limited.
    fn resolve(&mut self, path: &Path) -> Result<Walk, Fault> {
        let limit = self.max_path;
        self.walk(Model::root(), Model::parts(path), true, true, limit)
    }

    fn root() -> Walk {
        vec![(String::new(), 0)]
    }

    fn parts(path: &Path) -> VecDeque<String> {
        assert!(path.is_absolute(), "{}", path.display());
        path.iter()
            .skip(1)
            .map(|part| part.to_str().unwrap().to_owned())
            .collect()
    }

    fn path_of(walk: &Walk) -> PathBuf {
        let mut out = PathBuf::from("/");
        out.extend(walk.iter().skip(1).map(|(name, _)| name));
        out
    }
}

impl World for Model {
    no_io!();
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        let walk = self.resolve(path).ok()?;
        Some(Model::path_of(&walk))
    }
    fn is_link(&mut self, path: &Path) -> bool {
        self.walk(Model::root(), Model::parts(path), false, true, usize::MAX)
            .is_ok_and(|walk| matches!(self.nodes[walk.last().unwrap().1], Node::Link(_)))
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        match self.resolve(path) {
            Err(Fault::Missing) => Some(Absence::Missing),
            Err(Fault::NotDir) => Some(Absence::NotDirectory),
            _ => None,
        }
    }
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        // The directory is held already: walking to it again is not a lookup.
        let held = self
            .walk(
                Model::root(),
                Model::parts(directory),
                true,
                false,
                usize::MAX,
            )
            .unwrap();
        assert_eq!(
            Model::path_of(&held),
            directory,
            "a step from a canonical path"
        );
        let name = name.to_str().unwrap();
        let child = self.entry(held.last().unwrap().1, name, true).ok()??;
        match &self.nodes[child] {
            Node::Link(target) => {
                let mut walk = held;
                if target.starts_with('/') {
                    walk.truncate(1);
                }
                let pending = target.split('/').map(str::to_owned).collect();
                let walk = self.walk(walk, pending, true, true, usize::MAX).ok()?;
                Some(Model::path_of(&walk))
            }
            _ => Some(directory.join(name)),
        }
    }
}

/// The model with the default step of `World`.
struct ModelDefault<'a>(&'a mut Model);

impl World for ModelDefault<'_> {
    no_io!();
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        self.0.canonical(path)
    }
    fn is_link(&mut self, path: &Path) -> bool {
        self.0.is_link(path)
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        self.0.absent(path)
    }
}

/// [`TREE`] in the model, under `/r`.
fn model_tree() -> Model {
    let mut model = Model::new();
    for (path, kind) in TREE {
        let node = match *kind {
            "dir" => Node::Dir(BTreeMap::new()),
            "file" => Node::File,
            link => Node::Link(link.strip_prefix("->").unwrap().replace("{R}", "/r")),
        };
        model.add(&format!("/r/{path}"), node);
    }
    model
}

/// [`TREE`] on the disk, under `root`.
fn disk_tree(root: &Path) {
    let r = root.to_str().unwrap();
    for (path, kind) in TREE {
        let at = root.join(path);
        match *kind {
            "dir" => std::fs::create_dir_all(at).unwrap(),
            "file" => std::fs::write(at, "content\n").unwrap(),
            link => {
                let target = link.strip_prefix("->").unwrap().replace("{R}", r);
                std::os::unix::fs::symlink(target, at).unwrap();
            }
        }
    }
}

/// The outcome of the model under `/r` as the disk's under `root`.
fn on_disk(outcome: Confined, root: &Path) -> Confined {
    match outcome {
        Confined::Path(path) => Confined::Path(match path.strip_prefix("/r") {
            Ok(rest) => root.join(rest),
            Err(_) => path,
        }),
        other => other,
    }
}

#[test]
fn confinement_keeps_the_outcomes_of_resolving_every_prefix_1343() {
    let root = fresh("differential");
    disk_tree(&root);
    let mut model = model_tree();
    let mut all = paths(NAMES, 3);
    all.extend(
        paths(LINK_NAMES, 4)
            .into_iter()
            .filter(|path| path.matches('/').count() == 3),
    );
    let mut checked = 0;
    let mut opened = 0;
    let mut denied = 0;
    let mut absent = 0;
    for prefix in PREFIXES {
        let on = root.join(prefix);
        let model_prefix = Path::new("/r").join(prefix);
        for relative in &all {
            let expected = confine_quadratic(&mut Disk, &on, relative);
            assert_resolved(&expected, &on, relative);
            assert_eq!(
                confine(&mut Disk, &on, relative),
                expected,
                "disk step, prefix {prefix}, path {relative:?}"
            );
            assert_eq!(
                confine(&mut DiskDefault, &on, relative),
                expected,
                "default step, prefix {prefix}, path {relative:?}"
            );
            let modeled = confine(&mut model, &model_prefix, relative);
            assert_eq!(
                modeled,
                confine_quadratic(&mut model, &model_prefix, relative),
                "model, prefix {prefix}, path {relative:?}"
            );
            assert_eq!(
                modeled,
                confine(&mut ModelDefault(&mut model), &model_prefix, relative),
                "model with the default step, prefix {prefix}, path {relative:?}"
            );
            assert_eq!(
                on_disk(modeled.clone(), &root),
                expected,
                "model against the disk, prefix {prefix}, path {relative:?}"
            );
            // #1380: no host limit is reached here, so every directory that does not resolve
            // and is not a link is absent, and the outcomes are those of the rule before.
            assert_eq!(
                confine(&mut Before(&mut Disk), &on, relative),
                expected,
                "rule before #1380 on the disk, prefix {prefix}, path {relative:?}"
            );
            assert_eq!(
                confine(&mut Before(&mut model), &model_prefix, relative),
                modeled,
                "rule before #1380 in the model, prefix {prefix}, path {relative:?}"
            );
            checked += 1;
            match expected {
                Confined::Path(path) if path.exists() => opened += 1,
                Confined::Path(_) => {}
                Confined::Absent(_) => absent += 1,
                Confined::Denied => denied += 1,
            }
        }
    }
    // The cases reach every outcome: opened, a final component that does not exist, an absent
    // directory (`NotFound` or `Io`, answered without a path) and denied.
    assert!(checked > 50_000, "{checked}");
    assert!(opened > 500 && denied > 1_000, "{opened} {denied}");
    assert!(absent > 1_000, "{absent}");
    // Spot checks of `spec/14 §3`, under `box`.
    let on = root.join("box");
    let box_ = std::fs::canonicalize(&on).unwrap();
    for (relative, outcome) in [
        (
            "in_dir/inner.txt",
            Confined::Path(box_.join("sub/inner.txt")),
        ),
        ("chain/x.txt", Confined::Path(box_.join("sub/deeper/x.txt"))),
        ("back/note.txt", Confined::Path(box_.join("note.txt"))),
        ("sub/up2/note.txt", Confined::Path(box_.join("note.txt"))),
        // A final component that does not exist, in a directory that resolves: the path is
        // the canonical directory joined with it, so a write creates the file.
        ("new.txt", Confined::Path(box_.join("new.txt"))),
        ("in_dir/new.txt", Confined::Path(box_.join("sub/new.txt"))),
        (
            "chain/new.txt",
            Confined::Path(box_.join("sub/deeper/new.txt")),
        ),
        // #1459: an absent directory is answered, with no path.
        ("missing/new.txt", Confined::Absent(Absence::Missing)),
        ("missing/deeper/new.txt", Confined::Absent(Absence::Missing)),
        ("in_dir/missing/new.txt", Confined::Absent(Absence::Missing)),
        (
            "note.txt/x/new.txt",
            Confined::Absent(Absence::NotDirectory),
        ),
        ("in_link/x/new.txt", Confined::Absent(Absence::NotDirectory)),
        ("in_link", Confined::Denied),
        ("dangling", Confined::Denied),
        ("dangling_dir/new.txt", Confined::Denied),
        ("out_dir/secret.txt", Confined::Denied),
        ("out_abs/secret.txt", Confined::Denied),
        ("sub/out2/secret.txt", Confined::Denied),
        // D-134: out of the prefix and back into it is denied.
        ("out_dir/back_in/inner.txt", Confined::Denied),
        ("out_abs/inner/back/note.txt", Confined::Denied),
        ("up/outside/back_in/inner.txt", Confined::Denied),
        ("up/box/note.txt", Confined::Denied),
        ("loop/x", Confined::Denied),
        ("../outside/secret.txt", Confined::Denied),
        ("/etc/hosts", Confined::Denied),
        (".", Confined::Denied),
    ] {
        assert_eq!(confine(&mut Disk, &on, relative), outcome, "{relative}");
    }
    // The grant prefix itself: one that does not exist has no canonical directory, so every
    // path under it is `NotFound` without a path; a dangling link is denied; and under a
    // prefix that is a link the paths are those of the directory it leads to.
    for relative in ["new.txt", "sub/new.txt", "missing/deeper/new.txt"] {
        assert_eq!(
            confine(&mut Disk, &root.join("nothing"), relative),
            Confined::Absent(Absence::Missing),
            "{relative}"
        );
        assert_eq!(
            confine(&mut Disk, &root.join("nothing/deeper"), relative),
            Confined::Absent(Absence::Missing),
            "{relative}"
        );
        assert_eq!(
            confine(&mut Disk, &root.join("box/note.txt/under"), relative),
            Confined::Absent(Absence::NotDirectory),
            "{relative}"
        );
        assert_eq!(
            confine(&mut Disk, &root.join("dangling_prefix"), relative),
            Confined::Denied,
            "{relative}"
        );
    }
    let link = root.join("boxlink");
    assert_eq!(
        confine(&mut Disk, &link, "in_dir/new.txt"),
        Confined::Path(box_.join("sub/new.txt"))
    );
    assert_eq!(
        confine(&mut Disk, &link, "in_dir/missing/new.txt"),
        Confined::Absent(Absence::Missing)
    );
}

/// `world`, keeping every path a method of it is given, a step's as `directory/name`. The
/// file operations are never reached by confinement.
struct Recording<'a> {
    world: &'a mut dyn World,
    seen: Vec<PathBuf>,
}

impl World for Recording<'_> {
    fn print(&mut self, _line: &str) {}
    fn now(&mut self) -> i64 {
        0
    }
    fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
        Ok(None)
    }
    fn read_file(&mut self, path: &Path) -> Result<String, FileFault> {
        panic!("confinement read {}", path.display());
    }
    fn write_file(&mut self, path: &Path, _content: &str) -> Result<(), FileFault> {
        panic!("confinement wrote {}", path.display());
    }
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        self.seen.push(path.to_path_buf());
        self.world.canonical(path)
    }
    fn is_link(&mut self, path: &Path) -> bool {
        self.seen.push(path.to_path_buf());
        self.world.is_link(path)
    }
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        self.seen.push(directory.join(name));
        self.world.step(directory, name)
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        self.seen.push(path.to_path_buf());
        self.world.absent(path)
    }
}

#[test]
fn an_absent_directory_is_answered_without_a_path_1459() {
    // `spec/14 §3` rule 2, #1459: once confinement has confirmed that an intermediate
    // directory does not exist, the outcome is the failure itself. No path is returned, and
    // confinement gives the world no path below the absent directory: neither the path as
    // written nor any part of it past the first component that is not there.
    let root = fresh("absent");
    disk_tree(&root);
    let mut model = model_tree();
    // (prefix, path, the absent directory as written from the prefix, the absence)
    let cases = [
        ("box", "missing/new.txt", "missing", Absence::Missing),
        ("box", "missing/deeper/new.txt", "missing", Absence::Missing),
        (
            "box",
            "missing/a/b/c/d/new.txt",
            "missing",
            Absence::Missing,
        ),
        (
            "box",
            "sub/deeper/missing/a/new.txt",
            "sub/deeper/missing",
            Absence::Missing,
        ),
        (
            "box",
            "in_dir/missing/new.txt",
            "in_dir/missing",
            Absence::Missing,
        ),
        (
            "box",
            "in_abs/deeper/missing/a/new.txt",
            "in_abs/deeper/missing",
            Absence::Missing,
        ),
        (
            "box",
            "self/back/chain/missing/new.txt",
            "self/back/chain/missing",
            Absence::Missing,
        ),
        ("box", "sub/../missing/new.txt", "missing", Absence::Missing),
        (
            "box",
            "note.txt/x/new.txt",
            "note.txt/x",
            Absence::NotDirectory,
        ),
        (
            "box",
            "in_dir/inner.txt/x/y/new.txt",
            "in_dir/inner.txt/x",
            Absence::NotDirectory,
        ),
        ("boxlink", "missing/new.txt", "missing", Absence::Missing),
        (
            "boxlink",
            "in_dir/missing/a/new.txt",
            "in_dir/missing",
            Absence::Missing,
        ),
        ("box/sub", "missing/new.txt", "missing", Absence::Missing),
        ("nothing", "new.txt", "", Absence::Missing),
        ("nothing", "sub/new.txt", "", Absence::Missing),
        ("nothing/deeper", "sub/new.txt", "", Absence::Missing),
        (
            "box/note.txt/under",
            "sub/new.txt",
            "",
            Absence::NotDirectory,
        ),
    ];
    for (prefix, relative, stop, absence) in cases {
        for (name, world, base) in [
            ("disk", &mut Disk as &mut dyn World, root.as_path()),
            ("default step", &mut DiskDefault, root.as_path()),
            ("model", &mut model, Path::new("/r")),
        ] {
            let on = base.join(prefix);
            // `stop` is empty for the prefix itself: `join("")` would add a separator.
            let gone = if stop.is_empty() {
                on.clone()
            } else {
                on.join(stop)
            };
            let mut recording = Recording {
                world,
                seen: Vec::new(),
            };
            assert_eq!(
                confine(&mut recording, &on, relative),
                Confined::Absent(absence),
                "{name}: {prefix} {relative}"
            );
            assert!(
                recording.seen.contains(&gone),
                "{name}: {prefix} {relative}"
            );
            let written = lexical(&on, relative);
            for seen in &recording.seen {
                assert_ne!(*seen, written, "{name}: {prefix} {relative}");
                assert!(
                    *seen == gone || !seen.starts_with(&gone),
                    "{name}: {prefix} {relative}: the world was given {}",
                    seen.display()
                );
            }
        }
    }
    // A final component that does not exist is not absence: its directory resolves, so the
    // path is the canonical directory joined with it, under a prefix that is a link too.
    let box_ = std::fs::canonicalize(root.join("box")).unwrap();
    for (prefix, relative, target) in [
        ("box", "new.txt", "new.txt"),
        ("box", "in_dir/new.txt", "sub/new.txt"),
        ("boxlink", "new.txt", "new.txt"),
        ("boxlink", "in_abs/deeper/new.txt", "sub/deeper/new.txt"),
        ("box/sub", "deeper/new.txt", "sub/deeper/new.txt"),
    ] {
        let on = root.join(prefix);
        let outcome = confine(&mut Disk, &on, relative);
        assert_eq!(
            outcome,
            Confined::Path(box_.join(target)),
            "{prefix} {relative}"
        );
        assert_resolved(&outcome, &on, relative);
    }
}

/// A model with `/r/box` holding a chain of `depth` directories `d`, each holding the next
/// and a link `l` to it.
fn chain(depth: usize) -> Model {
    let mut model = Model::new();
    let mut at = String::from("/r/box");
    model.add(&at, Node::Dir(BTreeMap::new()));
    for _ in 0..depth {
        model.add(&format!("{at}/l"), Node::Link("d".to_owned()));
        at.push_str("/d");
        model.add(&at, Node::Dir(BTreeMap::new()));
    }
    model
}

/// The same chain on the disk, under `root/box`.
fn disk_chain(root: &Path, depth: usize) {
    let mut at = root.join("box");
    std::fs::create_dir_all(&at).unwrap();
    for _ in 0..depth {
        std::os::unix::fs::symlink("d", at.join("l")).unwrap();
        at.push("d");
        std::fs::create_dir(&at).unwrap();
    }
}

/// `links` components `l`, then `c - links` components `d`, then `f`.
fn chain_path(c: usize, links: usize) -> String {
    format!("{}{}f", "l/".repeat(links), "d/".repeat(c - links))
}

/// The lookups that `algorithm` makes in the model for [`chain_path`], which must open
/// `/r/box/d/../d/f`.
fn lookups(c: usize, links: usize, algorithm: fn(&mut dyn World, &Path, &str) -> Confined) -> u64 {
    let mut model = chain(c);
    let target = algorithm(&mut model, Path::new("/r/box"), &chain_path(c, links));
    let expected = format!("/r/box/{}f", "d/".repeat(c));
    assert_eq!(target, Confined::Path(PathBuf::from(expected)));
    model.lookups
}

#[test]
fn confinement_makes_linear_lookups_adr_0132_rule_16() {
    let c = 50;
    for links in [0, 3] {
        let small = lookups(c, links, confine);
        let large = lookups(10 * c, links, confine);
        // The prefix (2 lookups); one per directory, and one more for each link's target;
        // the confirming canonicalization of the directories (2 + c, and the targets); and
        // the final `lstat` of the whole path (2 + c + 1).
        let count = |c: u64| 7 + 3 * c + 2 * links as u64;
        assert_eq!(small, count(50), "{links} links");
        assert_eq!(large, count(500), "{links} links");
        assert_eq!(lookups(c, links, confine), small, "deterministic");
        assert!(large <= 20 * small, "{links} links: {small} then {large}");
        // The earlier algorithm resolved every prefix again: quadratic, past the bound.
        let small = lookups(c, links, confine_quadratic);
        let large = lookups(10 * c, links, confine_quadratic);
        assert!(large > 20 * small, "{links} links: {small} then {large}");
    }
}

/// A name of 100 bytes, repeated `levels` times as a relative path.
fn deep(levels: usize) -> String {
    vec!["X".repeat(100); levels].join("/")
}

/// Builds under the directory `on` a chain of hops: each `h` is a link to the four
/// directories [`deep`]`(4)` below it, so every lexical path stays short while the canonical
/// paths grow by about 400 bytes a hop. Returns the hops `h/../h/` up to the last one built,
/// and the number of directories below it whose canonical path is longer than the host's
/// canonicalization takes (`ENAMETOOLONG`: 1024 bytes on macOS; on Linux the limit of the
/// host's C library, 4096 bytes with glibc and lower with musl), or `None` when 64 hops reach
/// no limit.
fn disk_hops(on: &Path) -> (String, Option<usize>) {
    let mut hops = String::new();
    let mut at = on.to_path_buf();
    for _ in 0..64 {
        std::fs::create_dir_all(at.join(deep(4))).unwrap();
        std::os::unix::fs::symlink(deep(4), at.join("h")).unwrap();
        let past = (1..=4).find(|&levels| std::fs::canonicalize(at.join(deep(levels))).is_err());
        if past.is_some() {
            return (hops, past);
        }
        at.push("h");
        hops.push_str("h/");
    }
    (hops, None)
}

#[test]
fn host_limits_keep_the_outcomes_of_resolving_every_prefix_1343() {
    // Where resolving each component from the one before it and canonicalizing the lexical
    // path disagree, the earlier algorithm decides: more links in one path than the host
    // follows, and canonical paths longer than the host's canonicalization takes.
    //
    // #1456: the links alone show a new denial only where the kernel follows fewer links in
    // one lookup than the canonicalization does. On macOS `realpath` fails at the 34th link
    // of a path and `lstat` already at its 33rd, so the prefix ending at the link where
    // `realpath` stops cannot be examined: not a link, not absent, now Denied (before #1380
    // its lexical path was opened, and failed with `ELOOP`). On Linux glibc's `realpath`
    // and the kernel both follow at most 40: the prefix ending at the 41st link is seen as a
    // link and Denied under both rules, and glibc's `realpath` takes lexical paths of any
    // length. So the hops below, whose canonical paths pass the host's limit (1024 bytes on
    // macOS; on Linux the C library's, 4096 bytes with glibc, lower with musl) while their lexical paths stay short, give the new denials on
    // both hosts, and they are the paths of S-33: before #1380 the operation opened the
    // lexical path, through a link to a directory outside the prefix.
    let depth = 70;
    let root = fresh("limits");
    disk_chain(&root, depth);
    std::fs::write(root.join("box/note.txt"), "content\n").unwrap();
    std::os::unix::fs::symlink(".", root.join("box/self")).unwrap();
    std::fs::create_dir(root.join("outside")).unwrap();
    std::fs::write(root.join("outside/secret.txt"), "secret\n").unwrap();
    let on = root.join("box");
    let (hops, past) = disk_hops(&on);
    let levels = past.unwrap_or(1);
    let below = format!("{hops}{}", deep(levels));
    std::os::unix::fs::symlink(root.join("outside"), on.join(&below).join("esc")).unwrap();
    let mut model = chain(depth);
    model.add("/r/box/note.txt", Node::File);
    model.add("/r/box/self", Node::Link(".".to_owned()));
    let mut cases: Vec<String> = Vec::new();
    for links in 0..=depth {
        cases.push(chain_path(depth, links));
        cases.push(format!("{}missing/f", "l/".repeat(links)));
        cases.push(format!("{}l", "l/".repeat(links)));
    }
    for repeat in [10, 100, 199, 200, 201, 204, 205, 300, 819, 820, 821, 1000] {
        cases.push(format!("{}note.txt", "self/".repeat(repeat)));
        cases.push(format!("{}missing/note.txt", "self/".repeat(repeat)));
        cases.push(format!("{}l/d/f", "self/".repeat(repeat)));
    }
    for tail in ["f", "esc/secret.txt", "esc/new.txt", "missing/f", "esc"] {
        cases.push(format!("{below}/{tail}"));
    }
    cases.push("h/missing/new.txt".to_owned());
    let mut denied = 0;
    let mut newly_denied = 0;
    for relative in &cases {
        let expected = confine_quadratic(&mut Disk, &on, relative);
        assert_resolved(&expected, &on, relative);
        denied += usize::from(expected == Confined::Denied);
        // #1380: a host limit is not absence; nothing else changes.
        let before = confine_quadratic(&mut Before(&mut Disk), &on, relative);
        if before != expected {
            assert_eq!(
                expected,
                Confined::Denied,
                "only denials are new: {relative}"
            );
            // #1459: the rule before #1380 no longer has a path to open either.
            assert_eq!(before, Confined::Absent(Absence::Missing), "{relative}");
            newly_denied += 1;
        }
        assert_eq!(
            confine(&mut Disk, &on, relative),
            expected,
            "disk: {relative}"
        );
        assert_eq!(
            confine(&mut DiskDefault, &on, relative),
            expected,
            "default step: {relative}"
        );
        let prefix = Path::new("/r/box");
        assert_eq!(
            confine(&mut model, prefix, relative),
            confine_quadratic(&mut model, prefix, relative),
            "model: {relative}"
        );
    }
    // The host's link limit denies the longest chains, and the host's path limit now denies
    // the directories whose canonical path is longer than it takes.
    assert!(denied > 0, "{denied}");
    if past.is_some() {
        assert!(newly_denied > 0, "{newly_denied}");
        // S-33: the rule before #1380 left the operation the path as written, which opens
        // the file outside the prefix. #1459: that rule now yields `NotFound` and no path.
        let escape = format!("{below}/esc/secret.txt");
        let before = confine_quadratic(&mut Before(&mut Disk), &on, &escape);
        assert_eq!(before, Confined::Absent(Absence::Missing));
        assert_eq!(
            std::fs::read_to_string(lexical(&on, &escape)).unwrap(),
            "secret\n"
        );
        assert_eq!(confine(&mut Disk, &on, &escape), Confined::Denied);
    }
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        assert!(past.is_some(), "{hops}");
    }
    if cfg!(target_os = "linux") {
        // Past 40 links the prefix ending at the 41st is a link: Denied by both rules.
        for links in 41..=depth {
            let relative = chain_path(depth, links);
            assert_eq!(
                confine(&mut Disk, &on, &relative),
                Confined::Denied,
                "{relative}"
            );
            let before = confine_quadratic(&mut Before(&mut Disk), &on, &relative);
            assert_eq!(before, Confined::Denied, "{relative}");
        }
    }
}

/// The disk, counting the canonicalizations of one path.
struct Counting<'a> {
    path: &'a Path,
    count: usize,
}

impl World for Counting<'_> {
    no_io!();
    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        self.count += usize::from(path == self.path);
        Disk.canonical(path)
    }
    fn is_link(&mut self, path: &Path) -> bool {
        Disk.is_link(path)
    }
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        disk_step(directory, name)
    }
    fn absent(&mut self, path: &Path) -> Option<Absence> {
        disk_absent(path)
    }
}

#[test]
fn case_variant_spellings_keep_the_outcomes_of_resolving_every_prefix_1343() {
    // Names that differ only in case: on a case-insensitive file system the hosts' step keeps
    // the name as spelled while `canonical` returns it as stored, so the confirmation
    // disagrees and the prefix-by-prefix resolution decides; on a case-sensitive one they are
    // other names, mostly missing. Either way the outcomes are the earlier algorithm's.
    let root = fresh("case");
    let r = root.to_str().unwrap().to_owned();
    std::fs::create_dir_all(root.join("box/sub/deep")).unwrap();
    std::fs::create_dir_all(root.join("outside")).unwrap();
    std::fs::write(root.join("box/sub/inner.txt"), "content\n").unwrap();
    std::fs::write(root.join("box/F.TXT"), "content\n").unwrap();
    std::fs::write(root.join("outside/secret.txt"), "secret\n").unwrap();
    std::os::unix::fs::symlink("SUB", root.join("box/upper")).unwrap();
    std::os::unix::fs::symlink(format!("{r}/BOX/Sub"), root.join("box/abs")).unwrap();
    std::os::unix::fs::symlink("../OUTSIDE", root.join("box/out")).unwrap();
    std::os::unix::fs::symlink(format!("{r}/box/SUB"), root.join("outside/BACK")).unwrap();
    let insensitive = root.join("BOX/SUB/INNER.TXT").exists();
    let names = [
        "sub",
        "SUB",
        "Sub",
        "deep",
        "DEEP",
        "inner.txt",
        "INNER.TXT",
        "f.txt",
        "F.TXT",
        "upper",
        "abs",
        "out",
        "back",
        "BACK",
        "box",
        "BOX",
        "..",
    ];
    let mut fell_back = 0;
    for prefix in ["box", "BOX", "box/SUB", "Box/sub/../Sub"] {
        let on = root.join(prefix);
        for relative in paths(&names, 3) {
            let expected = confine_quadratic(&mut Disk, &on, &relative);
            let mut counting = Counting {
                path: &on,
                count: 0,
            };
            assert_eq!(
                confine(&mut counting, &on, &relative),
                expected,
                "prefix {prefix}, path {relative:?}"
            );
            assert_eq!(
                confine(&mut DiskDefault, &on, &relative),
                expected,
                "default step, prefix {prefix}, path {relative:?}"
            );
            // The fallback canonicalizes the prefix a second time.
            fell_back += usize::from(counting.count > 1);
        }
    }
    if insensitive {
        let on = root.join("box");
        let mut counting = Counting {
            path: &on,
            count: 0,
        };
        let opened = confine(&mut counting, &on, "SUB/inner.txt");
        assert_eq!(counting.count, 2, "the respelled directory falls back");
        let stored = std::fs::canonicalize(&on).unwrap().join("sub/inner.txt");
        assert_eq!(opened, Confined::Path(stored));
        assert!(fell_back > 100, "{fell_back}");
        // Out of the prefix and back into it, respelled: denied.
        assert_eq!(
            confine(&mut Disk, &on, "out/BACK/inner.txt"),
            Confined::Denied
        );
    }
}

#[test]
fn a_directory_past_the_host_path_limit_is_denied_1380() {
    // A link inside the prefix resolves to a deep directory, and the canonical path of the
    // directory below it is longer than the host takes (1024 bytes, as on macOS), so it does
    // not resolve though it exists. Below it a link leads outside the prefix. The rule before
    // #1380 returned the lexical path, which the operation then opened through that link;
    // since #1459 that rule has no path to return.
    let name = "X".repeat(200);
    let deep = |levels: usize| vec![name.as_str(); levels].join("/");
    let mut model = Model::new();
    model.max_path = 1024;
    model.add("/r/outside/secret.txt", Node::File);
    model.add(
        &format!("/r/box/{}/esc", deep(6)),
        Node::Link("/r/outside".to_owned()),
    );
    model.add("/r/box/q", Node::Link(deep(4)));
    let prefix = Path::new("/r/box");
    let relative = format!("q/{}/esc/secret.txt", deep(2));
    // `q/X` resolves (1011 bytes), `q/X/X` does not (1212 bytes) and is not a link.
    assert!(model.canonical(&prefix.join(format!("q/{name}"))).is_some());
    let past = prefix.join(format!("q/{}", deep(2)));
    assert_eq!(model.resolve(&past).err(), Some(Fault::TooLong));
    assert!(!model.is_link(&past));
    let before = confine(&mut Before(&mut model), prefix, &relative);
    assert_eq!(
        before,
        Confined::Absent(Absence::Missing),
        "the rule before #1380"
    );
    model.max_path = usize::MAX;
    assert_eq!(
        model.canonical(&lexical(prefix, &relative)),
        Some(PathBuf::from("/r/outside/secret.txt")),
        "the lexical path leads outside the prefix"
    );
    model.max_path = 1024;
    // Denied, in both routines and with either step.
    assert_eq!(confine(&mut model, prefix, &relative), Confined::Denied);
    assert_eq!(
        confine_quadratic(&mut model, prefix, &relative),
        Confined::Denied
    );
    assert_eq!(
        confine(&mut ModelDefault(&mut model), prefix, &relative),
        Confined::Denied
    );
    // A directory past the limit is denied even with nothing below it.
    let new = format!("q/{}/new.txt", deep(2));
    assert_eq!(confine(&mut model, prefix, &new), Confined::Denied);
    // Directories that do not exist, or are not directories, are absent, under the deep link
    // too: the failure is answered, with no path (#1459).
    model.add(&format!("/r/box/{}/file.txt", deep(5)), Node::File);
    for (relative, absence) in [
        ("missing/new.txt".to_owned(), Absence::Missing),
        ("q/missing/new.txt".to_owned(), Absence::Missing),
        (format!("q/{name}/missing/new.txt"), Absence::Missing),
        (
            format!("q/{name}/file.txt/x/new.txt"),
            Absence::NotDirectory,
        ),
    ] {
        assert_eq!(
            confine(&mut model, prefix, &relative),
            Confined::Absent(absence),
            "{relative}"
        );
        assert_eq!(
            confine(&mut ModelDefault(&mut model), prefix, &relative),
            Confined::Absent(absence),
            "default step: {relative}"
        );
    }
    // The table of #1459, second row: the canonical directory is long (1011 bytes of the
    // 1024 the host takes), the directory below it is missing and short, and the final
    // component is long, so the canonical directory joined with the rest is past the limit.
    // The answer is `NotFound`, not a path whose operation would fail on its length.
    let long = format!("q/{name}/missing/{}", "L".repeat(150));
    assert_eq!(
        confine(&mut model, prefix, &long),
        Confined::Absent(Absence::Missing)
    );
    // A world that does not tell absence denies every directory that does not resolve.
    struct Silent<'a>(&'a mut Model);
    impl World for Silent<'_> {
        no_io!();
        fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
            self.0.canonical(path)
        }
        fn is_link(&mut self, path: &Path) -> bool {
            self.0.is_link(path)
        }
    }
    assert_eq!(
        confine(&mut Silent(&mut model), prefix, "missing/new.txt"),
        Confined::Denied
    );
    assert_eq!(
        confine(&mut Silent(&mut model), prefix, "note.txt"),
        Confined::Path(prefix.join("note.txt"))
    );
}

#[test]
fn a_directory_past_the_host_path_limit_is_denied_on_the_disk_1380() {
    // The same layout on the disk. Each hop `h` is a link to the four directories below it,
    // so every lexical path stays short while the canonical paths grow; on a host whose
    // canonicalization has a length limit (1024 bytes on macOS) one of them does not resolve.
    let root = fresh("path_limit");
    let on = root.join("box");
    std::fs::create_dir_all(root.join("outside")).unwrap();
    std::fs::write(root.join("outside/secret.txt"), "secret\n").unwrap();
    std::fs::create_dir(&on).unwrap();
    let (hops, past) = disk_hops(&on);
    let limited = past.is_some();
    let levels = past.unwrap_or(1);
    let below = on.join(format!("{hops}{}", deep(levels)));
    std::os::unix::fs::symlink(root.join("outside"), below.join("esc")).unwrap();
    let relative = format!("{hops}{}/esc/secret.txt", deep(levels));
    if limited {
        // The rule before #1380 left the operation the path as written, which opens the file
        // outside the prefix; since #1459 that rule has no path to return.
        assert_eq!(
            confine(&mut Before(&mut Disk), &on, &relative),
            Confined::Absent(Absence::Missing)
        );
        assert_eq!(
            std::fs::read_to_string(lexical(&on, &relative)).unwrap(),
            "secret\n"
        );
    }
    assert_eq!(
        confine(&mut Disk, &on, &relative),
        Confined::Denied,
        "{limited}"
    );
    assert_eq!(
        confine(&mut DiskDefault, &on, &relative),
        Confined::Denied,
        "{limited}"
    );
    assert_eq!(
        confine_quadratic(&mut Disk, &on, &relative),
        Confined::Denied,
        "{limited}"
    );
    // A missing directory under a hop is absent: `NotFound`, with no path (#1459).
    let missing = "h/missing/new.txt";
    assert_eq!(
        confine(&mut Disk, &on, missing),
        Confined::Absent(Absence::Missing)
    );
    // On macOS (1024 bytes) and Linux (the C library's limit: 4096 bytes with glibc, lower
    // with musl, #1456) the limit is reached;
    // other hosts may canonicalize longer paths, and then the link below is resolved and
    // denied.
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        assert!(limited);
    }
}

/// Builds under the directory `on` a directory whose canonical path is `length` bytes long,
/// reached by the short path `a/b`: `a` is a link to its first component and `b`, there, a
/// link to the rest. Returns the canonical directory.
fn disk_long(on: &Path, length: usize) -> PathBuf {
    let mut remaining = length - on.as_os_str().len();
    let mut names = Vec::new();
    while remaining > 202 {
        names.push("N".repeat(200));
        remaining -= 201;
    }
    names.push("N".repeat(remaining - 1));
    let directory = on.join(names.join("/"));
    assert_eq!(directory.as_os_str().len(), length);
    std::fs::create_dir_all(&directory).unwrap();
    std::os::unix::fs::symlink(&names[0], on.join("a")).unwrap();
    std::os::unix::fs::symlink(names[1..].join("/"), on.join(&names[0]).join("b")).unwrap();
    directory
}

#[test]
fn the_host_limit_cases_of_a_missing_directory_are_not_found_1459() {
    // The table of #1459. Before it, a missing directory left the operation the path as
    // written, and the operation's own failure decided the value: `Io` where that path had
    // more links than the host follows in one lookup (`ELOOP`) or was longer than the host
    // takes (`ENAMETOOLONG`), although the directory was confirmed missing. Those are the
    // two classes whose value changes. Now confinement answers `NotFound` (`spec/14 §3` rule
    // 2), the same
    // value at a host limit as away from it, and the operation gets no path: neither the path
    // as written nor the canonical directory joined with the rest, which is longer than the
    // host takes in the last case and would fail with `Io` (`ENAMETOOLONG`).
    let depth = 40;
    let root = fresh("limit_cases");
    disk_chain(&root, depth);
    let on = root.join("box");
    std::os::unix::fs::symlink(".", on.join("self")).unwrap();
    let kind = |path: &Path| std::fs::read(path).unwrap_err().kind();
    let mut changed = Vec::new();
    for (link, tail) in [("l/", "missing/f"), ("self/", "missing/f")] {
        for links in 0..=depth {
            let relative = format!("{}{tail}", link.repeat(links));
            let outcome = confine(&mut Disk, &on, &relative);
            assert_eq!(
                outcome,
                confine_quadratic(&mut Disk, &on, &relative),
                "{relative}"
            );
            assert_eq!(
                outcome,
                confine(&mut DiskDefault, &on, &relative),
                "{relative}"
            );
            // Never a path: `NotFound`, or `Denied` where a host limit stops the resolution
            // before the missing directory is confirmed (#1380).
            assert!(
                matches!(
                    outcome,
                    Confined::Absent(Absence::Missing) | Confined::Denied
                ),
                "{relative}: {outcome:?}"
            );
            // The value changes only where the path as written failed otherwise.
            if outcome == Confined::Absent(Absence::Missing)
                && kind(&lexical(&on, &relative)) != std::io::ErrorKind::NotFound
            {
                changed.push((link, links));
            }
        }
    }
    // Away from the limits the answer is the operation's, as before.
    for links in 0..=30 {
        for link in ["l/", "self/"] {
            let relative = format!("{}missing/f", link.repeat(links));
            assert_eq!(
                confine(&mut Disk, &on, &relative),
                Confined::Absent(Absence::Missing),
                "{relative}"
            );
            assert!(!changed.contains(&(link, links)), "{relative}");
        }
    }
    if cfg!(target_os = "macos") {
        // 33 links: `realpath` resolves them and confirms the directory missing, while one
        // lookup of the path as written fails with `ELOOP`, `Io` before.
        assert_eq!(changed, [("l/", 33), ("self/", 33)]);
    }
    // The second class whose value changes: a path as written longer than the host takes,
    // below a missing directory. The operation failed on its length (`ENAMETOOLONG`, `Io`)
    // before it reached the missing directory; confinement stops at that directory. Six
    // components of 200 bytes pass the 1024 bytes of macOS, 25 the 4096 of Linux too. One
    // long final component alone changes nothing: the path as written was `NotFound` already.
    for (components, limited) in [
        (6, cfg!(target_os = "macos")),
        (25, cfg!(any(target_os = "macos", target_os = "linux"))),
    ] {
        let relative = format!("missing/{}/f", vec!["P".repeat(200); components].join("/"));
        for outcome in [
            confine(&mut Disk, &on, &relative),
            confine(&mut DiskDefault, &on, &relative),
            confine_quadratic(&mut Disk, &on, &relative),
        ] {
            assert_eq!(outcome, Confined::Absent(Absence::Missing), "{components}");
        }
        if limited {
            assert_ne!(
                kind(&lexical(&on, &relative)),
                std::io::ErrorKind::NotFound,
                "{components}"
            );
        }
    }
    let one = format!("missing/{}", "P".repeat(200));
    assert_eq!(
        confine(&mut Disk, &on, &one),
        Confined::Absent(Absence::Missing)
    );
    assert_eq!(kind(&lexical(&on, &one)), std::io::ErrorKind::NotFound);
    // A canonical directory of 897 bytes behind two links, a short missing directory and a
    // final component of 150 bytes.
    let long = fresh("limit_long").join("box");
    std::fs::create_dir(&long).unwrap();
    let directory = disk_long(&long, 897);
    assert_eq!(std::fs::canonicalize(long.join("a/b")).unwrap(), directory);
    let leaf = "L".repeat(150);
    let relative = format!("a/b/missing/{leaf}");
    for outcome in [
        confine(&mut Disk, &long, &relative),
        confine(&mut DiskDefault, &long, &relative),
        confine_quadratic(&mut Disk, &long, &relative),
    ] {
        assert_eq!(outcome, Confined::Absent(Absence::Missing));
    }
    assert_eq!(
        kind(&lexical(&long, &relative)),
        std::io::ErrorKind::NotFound
    );
    if cfg!(target_os = "macos") {
        // The canonical directory joined with the rest is 1056 bytes, past the 1024 the host
        // takes: an operation on it would fail with `Io`.
        let joined = directory.join("missing").join(&leaf);
        assert_eq!(joined.as_os_str().len(), 1056);
        assert_ne!(kind(&joined), std::io::ErrorKind::NotFound);
    }
    // A final component that does not exist in that directory is still left to the
    // operation, in the canonical directory.
    assert_eq!(
        confine(&mut Disk, &long, "a/b/new.txt"),
        Confined::Path(directory.join("new.txt"))
    );
}
