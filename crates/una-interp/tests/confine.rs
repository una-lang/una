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

// Tests may unwrap and index freely (compiler/conventions.md §1). Links need Unix.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use una_interp::{FileFault, NotText, World, confine, disk_step};

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
fn confine_quadratic(world: &mut dyn World, prefix: &Path, relative: &str) -> Option<PathBuf> {
    let mut parts: Vec<&OsStr> = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let (last, directories) = parts.split_last()?;
    let mut lexical = prefix.to_path_buf();
    let Some(root) = world.canonical(prefix) else {
        return quadratic_missing(world, &lexical, prefix, &parts);
    };
    let mut resolved = root.clone();
    for part in directories {
        lexical.push(part);
        match world.canonical(&lexical) {
            Some(directory) if directory.starts_with(&root) => resolved = directory,
            Some(_) => return None,
            None => return quadratic_missing(world, &lexical, prefix, &parts),
        }
    }
    let target = resolved.join(last);
    (!world.is_link(&target)).then_some(target)
}

fn quadratic_missing(
    world: &mut dyn World,
    lexical: &Path,
    prefix: &Path,
    parts: &[&OsStr],
) -> Option<PathBuf> {
    if world.is_link(lexical) {
        return None;
    }
    let mut out = prefix.to_path_buf();
    out.extend(parts);
    Some(out)
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
}

enum Node {
    Dir(BTreeMap<String, usize>),
    File,
    Link(String),
}

/// An in-memory file system with the semantics of `realpath` and `lstat`, counting each
/// lookup of a name in a directory. Its step looks up only the new name in the directory
/// already resolved (as an `openat` from a held directory would), and resolves a link's
/// target from there.
struct Model {
    nodes: Vec<Node>,
    lookups: u64,
}

/// A directory being walked: the names and nodes from the root, the root first.
type Walk = Vec<(String, usize)>;

impl Model {
    fn new() -> Self {
        Model {
            nodes: vec![Node::Dir(BTreeMap::new())],
            lookups: 0,
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
    /// with at most 40 links followed, as the hosts do.
    fn walk(
        &mut self,
        mut walk: Walk,
        mut pending: VecDeque<String>,
        follow_last: bool,
        counted: bool,
    ) -> Option<Walk> {
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
            let child = self.entry(at, &name, counted).ok()??;
            match &self.nodes[child] {
                Node::Link(target) if follow_last || !pending.is_empty() => {
                    hops += 1;
                    if hops > 40 {
                        return None;
                    }
                    if target.starts_with('/') {
                        walk.truncate(1);
                    }
                    for part in target.split('/').rev() {
                        pending.push_front(part.to_owned());
                    }
                }
                _ => walk.push((name, child)),
            }
        }
        Some(walk)
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
        let walk = self.walk(Model::root(), Model::parts(path), true, true)?;
        Some(Model::path_of(&walk))
    }
    fn is_link(&mut self, path: &Path) -> bool {
        self.walk(Model::root(), Model::parts(path), false, true)
            .is_some_and(|walk| matches!(self.nodes[walk.last().unwrap().1], Node::Link(_)))
    }
    fn step(&mut self, directory: &Path, name: &OsStr) -> Option<PathBuf> {
        // The directory is held already: walking to it again is not a lookup.
        let held = self
            .walk(Model::root(), Model::parts(directory), true, false)
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
                let walk = self.walk(walk, pending, true, true)?;
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
fn on_disk(outcome: Option<PathBuf>, root: &Path) -> Option<PathBuf> {
    outcome.map(|path| match path.strip_prefix("/r") {
        Ok(rest) => root.join(rest),
        Err(_) => path,
    })
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
    for prefix in PREFIXES {
        let on = root.join(prefix);
        let model_prefix = Path::new("/r").join(prefix);
        for relative in &all {
            let expected = confine_quadratic(&mut Disk, &on, relative);
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
                on_disk(modeled, &root),
                expected,
                "model against the disk, prefix {prefix}, path {relative:?}"
            );
            checked += 1;
            match expected {
                Some(path) if path.exists() => opened += 1,
                Some(_) => {}
                None => denied += 1,
            }
        }
    }
    // The cases reach every outcome: opened, missing (`NotFound` later) and denied.
    assert!(checked > 50_000, "{checked}");
    assert!(opened > 500 && denied > 1_000, "{opened} {denied}");
    // Spot checks of `spec/14 §3`, under `box`.
    let on = root.join("box");
    let box_ = std::fs::canonicalize(&on).unwrap();
    for (relative, outcome) in [
        ("in_dir/inner.txt", Some(box_.join("sub/inner.txt"))),
        ("chain/x.txt", Some(box_.join("sub/deeper/x.txt"))),
        ("back/note.txt", Some(box_.join("note.txt"))),
        ("sub/up2/note.txt", Some(box_.join("note.txt"))),
        ("missing/new.txt", Some(on.join("missing/new.txt"))),
        ("in_link", None),
        ("dangling", None),
        ("dangling_dir/new.txt", None),
        ("out_dir/secret.txt", None),
        ("out_abs/secret.txt", None),
        ("sub/out2/secret.txt", None),
        // D-134: out of the prefix and back into it is denied.
        ("out_dir/back_in/inner.txt", None),
        ("out_abs/inner/back/note.txt", None),
        ("up/outside/back_in/inner.txt", None),
        ("up/box/note.txt", None),
        ("loop/x", None),
        ("../outside/secret.txt", None),
        ("/etc/hosts", None),
        (".", None),
    ] {
        assert_eq!(confine(&mut Disk, &on, relative), outcome, "{relative}");
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
fn lookups(
    c: usize,
    links: usize,
    algorithm: fn(&mut dyn World, &Path, &str) -> Option<PathBuf>,
) -> u64 {
    let mut model = chain(c);
    let target = algorithm(&mut model, Path::new("/r/box"), &chain_path(c, links));
    let expected = format!("/r/box/{}f", "d/".repeat(c));
    assert_eq!(target, Some(PathBuf::from(expected)));
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

#[test]
fn host_limits_keep_the_outcomes_of_resolving_every_prefix_1343() {
    // Where resolving each component from the one before it and canonicalizing the lexical
    // path disagree, the earlier algorithm decides: more links in one path than the host
    // follows (32 on macOS, 40 on Linux, 40 in the model), and lexical paths longer than the
    // host takes (1024 bytes on macOS, 4096 on Linux), though each resolved path is short.
    let depth = 70;
    let root = fresh("limits");
    disk_chain(&root, depth);
    std::fs::write(root.join("box/note.txt"), "content\n").unwrap();
    std::os::unix::fs::symlink(".", root.join("box/self")).unwrap();
    let on = root.join("box");
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
    let mut denied = 0;
    for relative in &cases {
        let expected = confine_quadratic(&mut Disk, &on, relative);
        denied += usize::from(expected.is_none());
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
    // The host's link limit denies the longest chains.
    assert!(denied > 0, "{denied}");
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
        assert_eq!(opened, Some(stored));
        assert!(fell_back > 100, "{fell_back}");
        // Out of the prefix and back into it, respelled: denied.
        assert_eq!(confine(&mut Disk, &on, "out/BACK/inner.txt"), None);
    }
}
