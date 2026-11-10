//! The timing label of an expression (ADR 0116 IF5, IF6): what the running time of a piece of
//! code can depend on, for the deadlines that turn that time into values (IF7, IF8).
//!
//! Two steps, so that every construct with a deadline uses the same rule:
//! 1. While a site is checked, [`Checker::timing_start`] and [`Checker::timing_end`] record
//!    its frame ([`TimingFrames`]): the program counter, the bindings it reads from outside it,
//!    the functions it calls, the operations it performs, the handlers it installs and whether
//!    it calls a function value, each recorded once, in the innermost open frame. Every value
//!    checked joins `ℓ*` of its type into the frames and into the body label of its function or
//!    handler ([`Checker::timing_value`]).
//! 2. Once every body is checked, the summaries of the graph give every frame's label bottom up
//!    ([`Summaries::frames`]), with `τ_conc` from the parts of task constructs that can run
//!    beside it ([`Concurrency`]); the search over the graph ([`timing`]) runs on a frame's
//!    rebuilt seed only to name the source of a label that is reported.
//!
//! The function of a `[drive]` step (IF7) is a site whose seed is the step's parameters and the
//! step itself as its one callee.

use std::collections::{BTreeMap, BTreeSet};

use super::Findings;
use super::check::Checker;
use super::env::Env;
use super::ty::{CapId, Ty};
use una_core::Label;

/// What a site's timing can depend on, as its checking recorded it (ADR 0116 IF5 items 1 to
/// 5): the rest comes from the call graph ([`timing`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct TimingSeed {
    /// Labels of the site itself, each with the source a message names: the program counter,
    /// the bindings it reads from outside it (`ℓ*` of their types), a deadline's own label.
    pub(crate) sources: Vec<(Label, String)>,
    /// The functions it calls or names as values, lambdas written in it included.
    pub(crate) callees: BTreeSet<usize>,
    /// The operations it performs itself.
    pub(crate) ops: BTreeSet<(CapId, String)>,
    /// The handlers it installs with `with` or `given`.
    pub(crate) handlers: BTreeSet<usize>,
    /// Whether it calls a function value whose code is not known at the site (IF5 item 4).
    pub(crate) value_call: bool,
    /// The prelude functions that apply a function argument ([`applies_function`]) which it
    /// calls with an argument whose code is not known at the call, or names as values (IF5
    /// item 4, #1023). A call whose every function argument is a lambda written at the call
    /// is not one: what the lambda runs and captures is in the seed already.
    pub(crate) opaque_applies: BTreeSet<usize>,
    /// Whether it lies inside a lambda, a `together` task or a handler's operation, whose
    /// concurrent work the structure of its function does not show: `τ_conc` is then `⊤`.
    pub(crate) unplaced: bool,
    /// The node of the graph of ADR 0116 P1 whose code holds it, `None` outside every node.
    pub(crate) node: Option<Caller>,
    /// The function or handler whose code holds it, a lambda written there included, `None`
    /// outside both: where a lambda's code is written, for the reach of a `via` (#1673).
    pub(crate) home: Option<Caller>,
    /// The innermost construct whose part holds it (P3), an index of `Findings::constructs`.
    pub(crate) construct: Option<usize>,
    /// The join of `ℓ*` of every value its own code computes, its symbolic labels dropped
    /// (#1685): a value labeled in it from literals, as `let k: secret Int = 3000000`, counts as
    /// the same value read from outside would by IF5 item 2; `None` while every value is
    /// `public`.
    pub(crate) values: Option<Label>,
    /// The join of the deep labels of what it binds and of the answers of the operations it
    /// performs, for the reach of a `start` (#1674); `None` when it is `public`.
    pub(crate) bound: Option<Label>,
}

/// The timing label of a site (ADR 0116 IF5): its join, and the sources it joins, in the order
/// a message names them.
#[derive(Clone, Debug)]
pub(crate) struct Timing {
    /// `τ(e)`, `τ_conc` included.
    pub(crate) label: Label,
    /// Each source with its label.
    pub(crate) sources: Vec<(Label, String)>,
    /// The operations the site can perform, through the functions and handlers it reaches.
    pub(crate) ops: BTreeSet<(CapId, String)>,
}

impl Timing {
    /// The first source whose label does not flow to `bound`, for a message.
    pub(crate) fn culprit(&self, bound: &Label) -> String {
        self.sources
            .iter()
            .find(|(label, _)| !label.flows_to(bound))
            .map_or_else(String::new, |(_, source)| source.clone())
    }
}

/// Which handlers can serve an operation of a host capability (`Console`, `Http`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostHandlers {
    /// Only those installed on the way, as for a `[drive]` step, which the host calls with the
    /// host's own capabilities (`spec/26 §2` rule 5).
    Installed,
    /// Every handler of the capability in the package: a site inside a function may run under
    /// a handler its callers installed (IF5 item 3).
    All,
}

/// An operation of the host's `Http`, for `L_lim` (ADR 0117 IF4): where it is, the function
/// whose body holds it, and whether it is unplaced, as a timing seed is ([`TimingSeed`]).
#[derive(Clone, Debug)]
pub(crate) struct HttpSite {
    /// The operation.
    pub(crate) span: una_syntax::Span,
    /// The function whose body holds it, or `None` outside a function (a handler's operation).
    pub(crate) function: Option<usize>,
    /// Whether it lies inside a lambda, a task, a `tasks` block or outside a function.
    pub(crate) unplaced: bool,
    /// The node of the graph of ADR 0116 P1 whose code holds it, `None` outside every node.
    pub(crate) node: Option<Caller>,
    /// The innermost construct whose part holds it (P3).
    pub(crate) construct: Option<usize>,
}

/// Where a call is written, as a node of the call graph of ADR 0116 P1: the body of a function,
/// the operations of a handler, or a lambda, all lambdas being one node (`Λ`), which every
/// function or handler that calls a function value reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Caller {
    /// The body of a function.
    Function(usize),
    /// The operations of a handler.
    Handler(usize),
    /// A lambda, wherever it is written.
    Lambda,
}

/// What an edge of the graph of ADR 0116 P1 goes to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Target {
    /// A function called, or named as a value.
    Function(usize),
    /// `Λ`: a call of a function value, a `uses of` parameter's included.
    Lambda,
    /// An operation of a capability, and through it the capability's node `K(c)`.
    Op(CapId, String),
    /// A handler installed by `with` or `given`.
    Install(usize),
}

/// An edge of the graph of ADR 0116 P1, with the call site it comes from and the innermost
/// task construct whose part holds that site (P3), so that `ctx` of the site is that of the
/// construct.
#[derive(Clone, Debug)]
pub(crate) struct GraphEdge {
    /// The node whose code holds the call: a function's body, a handler's operations, or `Λ`
    /// for a lambda's body.
    /// `None` in code outside every node, as an example's `do` line, whose edges still seed
    /// `conc` of what they reach from a part (P3).
    pub(crate) from: Option<Caller>,
    /// The function or handler whose code holds the call, a lambda written there included:
    /// where a lambda's call is written, for the reach of a `via` (#1673).
    pub(crate) home: Option<Caller>,
    /// What it goes to.
    pub(crate) to: Target,
    /// The call site.
    pub(crate) span: una_syntax::Span,
    /// The innermost construct whose part holds the call, an index of `Findings::constructs`.
    pub(crate) construct: Option<usize>,
}

/// The task constructs of ADR 0116 P2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstructKind {
    /// `together`: each branch is a part.
    Together,
    /// `together for`: the body is a part, the list is not.
    TogetherFor,
    /// `first of`: each branch is a part.
    FirstOf,
    /// `tasks`: the block, with each `start` in it, is a part, the bound is not.
    Tasks,
}

impl ConstructKind {
    /// Its words.
    pub(crate) fn words(self) -> &'static str {
        match self {
            ConstructKind::Together => "together",
            ConstructKind::TogetherFor => "together for",
            ConstructKind::FirstOf => "first of",
            ConstructKind::Tasks => "tasks",
        }
    }
}

/// A task construct checked, a node of the construct tree of its body (ADR 0116 P3): the
/// constructs of one node's code form a tree by nesting, a lambda's body starting a tree of its
/// own, since its code leaves from `Λ`.
#[derive(Clone, Debug)]
pub(crate) struct ConstructNode {
    /// Which construct it is.
    pub(crate) kind: ConstructKind,
    /// The whole construct.
    pub(crate) span: una_syntax::Span,
    /// The node of the graph whose code holds it.
    pub(crate) node: Option<Caller>,
    /// The innermost construct whose part holds it.
    pub(crate) parent: Option<usize>,
    /// The frame of `κ`, `τ⁰` of the whole construct (P2): its parts, its list and its bound;
    /// `None` until it is recorded, which counts as `⊤` (fail closed).
    pub(crate) frame: Option<TimingFrame>,
}

/// What checking records for the open sites, in the innermost one (ADR 0116 amendment of
/// 2026-10-25, E: frames as a tree, each item recorded once; slice 8a of the plan on #1087).
#[derive(Clone, Debug)]
enum Event {
    /// A source of the frame itself: its program counter, a deadline's label.
    Source(Label, String),
    /// A read of the local `name` at `position`, with `ℓ*` of its type, and the depth of the
    /// outermost open frame whose mark is above the position: the read is free in that frame
    /// and in every open frame inside it, since the marks of the stack never decrease.
    Read(Label, String, usize, usize),
    /// A call of, or a reference to, a function.
    Callee(usize),
    /// An operation performed.
    Op(CapId, String),
    /// A handler installed.
    Install(usize),
    /// A call of a function value.
    ValueCall,
    /// A prelude function applied to an argument whose code is not known (#1023).
    Opaque(usize),
    /// A value computed (IF5 item 7).
    Value(Label),
    /// A value bound or answered, for the reach of a `start` (#1674).
    Bound(Label),
}

/// One frame of [`Checker::timing_start`], a node of its body's tree of frames: where its
/// locals start, its parent and children, its own fields (`seed`, with no items), and the items
/// recorded while it was the innermost open frame, each with its sequence number.
#[derive(Clone, Debug, Default)]
struct FrameNode {
    mark: usize,
    parent: Option<usize>,
    depth: usize,
    children: Vec<usize>,
    seed: TimingSeed,
    events: Vec<(u64, Event)>,
}

/// A frame of [`TimingFrames`]: the site whose checking recorded it, by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimingFrame(usize);

/// The frames of the sites a pass checked (ADR 0116 amendment of 2026-10-25, E): a tree by
/// nesting for each body. Each item checking records goes once into the innermost open frame,
/// not into every open frame ([`Checker::timing_start`]), and a site keeps its frame, whose seed
/// is rebuilt from its subtree when a consumer asks for it ([`TimingFrames::seed`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct TimingFrames {
    tree: Vec<FrameNode>,
    next: u64,
}

impl TimingFrames {
    /// Records `event` in the frame `at`, the innermost open one.
    fn record(&mut self, at: usize, event: Event) {
        if let Some(frame) = self.tree.get_mut(at) {
            frame.events.push((self.next, event));
            self.next += 1;
        }
    }

    /// The seed of `frame`, rebuilt from its subtree ([`TimingFrames::seed_of`]): only for a
    /// consumer that needs its items one by one, as the search that names a reported site's
    /// source, or the reach of a `start`.
    pub(crate) fn seed(&self, frame: TimingFrame) -> TimingSeed {
        self.seed_of(frame.0, true)
    }

    /// The fields of `frame` itself, with none of its items: its node, its home, its innermost
    /// construct and whether it is unplaced, which `τ_conc`, the preview's `τ_conc` and the
    /// `via` rule read. O(1).
    pub(crate) fn own(&self, frame: TimingFrame) -> TimingSeed {
        self.tree
            .get(frame.0)
            .map_or_else(TimingSeed::default, |node| TimingSeed {
                unplaced: node.seed.unplaced,
                node: node.seed.node,
                home: node.seed.home,
                construct: node.seed.construct,
                ..TimingSeed::default()
            })
    }

    /// For every frame, the join of `item` over the items of its subtree other than sources and
    /// reads, bottom up: a frame's parent has a smaller id, so one pass over the ids in
    /// descending order adds each frame's join into its parent's. O(F + N) joins for the `F`
    /// frames and `N` items (slice 8b of #1087, amendment E: "items bottom-up").
    fn fold_up<T: Clone>(
        &self,
        zero: &T,
        item: impl Fn(&Event) -> Option<T>,
        join: impl Fn(&T, &T) -> T,
    ) -> Vec<T> {
        let mut out: Vec<T> = vec![zero.clone(); self.tree.len()];
        for at in (0..self.tree.len()).rev() {
            let Some(frame) = self.tree.get(at) else {
                continue;
            };
            let mut own = out.get(at).cloned().unwrap_or_else(|| zero.clone());
            for (_, event) in &frame.events {
                una_core::label_counters::frame_step();
                if let Some(value) = item(event) {
                    own = join(&own, &value);
                }
            }
            if let Some(parent) = frame.parent
                && let Some(slot) = out.get_mut(parent)
            {
                *slot = join(slot, &own);
            }
            if let Some(slot) = out.get_mut(at) {
                *slot = own;
            }
        }
        out
    }

    /// For every frame, the join of its own sources (its program counter, a deadline's label)
    /// and of the reads free in it, which may lie in its subtree (amendment E, "reads"):
    /// - its frames are numbered in the order the walk of its tree visits them, so that each
    ///   frame's subtree is the interval from its id to the end of its subtree;
    /// - the depths are swept upward; at depth `j`, each read whose outermost frame is at depth
    ///   `j` joins a segment tree over the ids at its frame's id;
    /// - each frame at depth `j` then asks for the join over its interval.
    ///
    /// So a frame counts a read in its subtree exactly when the read is free in it. O((R + F) log
    /// F) joins for the `R` reads and the `F` frames of the whole pass, and a counting sort of
    /// the depths. One segment tree serves every body of the pass, so the logarithm is that of
    /// the pass's frames, not of one body's as amendment E states it per body: within a
    /// logarithm of the best possible either way.
    fn sources_up(&self) -> Vec<Label> {
        let n = self.tree.len();
        // The end of each subtree: its last id, plus one. Children have larger ids.
        let mut end: Vec<usize> = (1..=n).collect();
        for at in (0..n).rev() {
            if let Some(parent) = self.tree.get(at).and_then(|frame| frame.parent) {
                let last = end.get(at).copied().unwrap_or(at + 1);
                if let Some(slot) = end.get_mut(parent) {
                    *slot = (*slot).max(last);
                }
            }
        }
        let depth_of = |at: usize| self.tree.get(at).map_or(0, |frame| frame.depth);
        let deepest = (0..n).map(depth_of).max().unwrap_or(0);
        let mut frames_at: Vec<Vec<usize>> = vec![Vec::new(); deepest + 1];
        let mut reads_at: Vec<Vec<(usize, &Label)>> = vec![Vec::new(); deepest + 1];
        let mut out: Vec<Label> = vec![Label::public(); n];
        for (at, frame) in self.tree.iter().enumerate() {
            if let Some(slot) = frames_at.get_mut(frame.depth) {
                slot.push(at);
            }
            for (_, event) in &frame.events {
                una_core::label_counters::frame_step();
                match event {
                    Event::Source(label, _) => {
                        if let Some(slot) = out.get_mut(at) {
                            *slot = slot.join(label);
                        }
                    }
                    Event::Read(label, _, _, outer) => {
                        if let Some(slot) = reads_at.get_mut(*outer) {
                            slot.push((at, label));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut tree = JoinTree::new(n);
        for (depth, frames) in frames_at.iter().enumerate() {
            for &(at, label) in reads_at.get(depth).map_or(&[][..], Vec::as_slice) {
                tree.join_at(at, label);
            }
            for &at in frames {
                let reads = tree.join_over(at, end.get(at).copied().unwrap_or(at + 1));
                if let Some(slot) = out.get_mut(at) {
                    *slot = slot.join(&reads);
                }
            }
        }
        out
    }
}

/// A segment tree of labels under join, over `n` positions: a join at a position and the join
/// over an interval, each O(log n) joins, counted.
struct JoinTree {
    size: usize,
    nodes: Vec<Label>,
}

impl JoinTree {
    fn new(n: usize) -> JoinTree {
        let size = n.next_power_of_two().max(1);
        JoinTree {
            size,
            nodes: vec![Label::public(); 2 * size],
        }
    }

    /// Joins `label` into position `at`.
    fn join_at(&mut self, at: usize, label: &Label) {
        let mut node = self.size + at;
        while node >= 1 {
            una_core::label_counters::frame_step();
            if let Some(slot) = self.nodes.get_mut(node) {
                *slot = slot.join(label);
            }
            node /= 2;
        }
    }

    /// The join over positions `from..to`.
    fn join_over(&self, from: usize, to: usize) -> Label {
        let mut out = Label::public();
        let (mut low, mut high) = (from + self.size, to + self.size);
        while low < high {
            una_core::label_counters::frame_step();
            if low % 2 == 1 {
                if let Some(label) = self.nodes.get(low) {
                    out = out.join(label);
                }
                low += 1;
            }
            if high % 2 == 1 {
                high -= 1;
                if let Some(label) = self.nodes.get(high) {
                    out = out.join(label);
                }
            }
            low /= 2;
            high /= 2;
        }
        out
    }
}

impl TimingFrames {
    /// The seed of the frame `at`, rebuilt from the items of its subtree in the order checking
    /// recorded them, exactly as the copying into every open frame built it: its own sources and
    /// fields; each read of a local bound before its mark, once per name, in order; and the
    /// callees, operations, installations, values, bindings, value calls and opaque
    /// applications of the whole subtree. O(k log k) for the `k` items of the subtree.
    fn seed_of(&self, at: usize, counted: bool) -> TimingSeed {
        let Some(frame) = self.tree.get(at) else {
            return TimingSeed::default();
        };
        let mut seed = frame.seed.clone();
        let mut events: Vec<&(u64, Event)> = Vec::new();
        let mut pending = vec![at];
        while let Some(node) = pending.pop() {
            let Some(child) = self.tree.get(node) else {
                continue;
            };
            pending.extend(child.children.iter().copied());
            events.extend(
                child
                    .events
                    .iter()
                    .filter(|(_, event)| node == at || !matches!(event, Event::Source(..))),
            );
        }
        events.sort_unstable_by_key(|(sequence, _)| *sequence);
        // Counted as frame steps, so that a test sees each rebuild (slice 8b of #1087), except the rebuilds of a
        // cross-check in a build with debug assertions, which a release build never makes.
        if counted {
            for _ in &events {
                una_core::label_counters::frame_step();
            }
        }
        let mut read: BTreeSet<&str> = BTreeSet::new();
        let join = |slot: &mut Option<Label>, label: &Label| {
            *slot = Some(match slot {
                Some(before) => before.join(label),
                None => label.clone(),
            });
        };
        for (_, event) in events {
            match event {
                Event::Source(label, source) => seed.sources.push((label.clone(), source.clone())),
                Event::Read(label, name, position, _) => {
                    if *position < frame.mark && read.insert(name.as_str()) {
                        seed.sources
                            .push((label.clone(), format!("the binding `{name}`")));
                    }
                }
                Event::Callee(f) => {
                    seed.callees.insert(*f);
                }
                Event::Op(cap, op) => {
                    seed.ops.insert((*cap, op.clone()));
                }
                Event::Install(h) => {
                    seed.handlers.insert(*h);
                }
                Event::ValueCall => seed.value_call = true,
                Event::Opaque(f) => {
                    seed.opaque_applies.insert(*f);
                }
                Event::Value(label) => join(&mut seed.values, label),
                Event::Bound(label) => join(&mut seed.bound, label),
            }
        }
        seed
    }
}

impl Checker<'_, '_, '_> {
    /// Starts recording the seed of a site (ADR 0116 IF5): the locals bound from here on are
    /// its own, and the ones before it are its free variables. Sites nest.
    pub(crate) fn timing_start(&mut self) {
        let mut seed = TimingSeed {
            unplaced: self.unplaced(),
            node: self.caller(),
            home: self.home(),
            construct: self.parts.last().copied(),
            ..TimingSeed::default()
        };
        let frames = &mut self.findings.frames;
        let at = frames.tree.len();
        if let Some(&parent) = self.timing.last()
            && let Some(frame) = frames.tree.get_mut(parent)
        {
            frame.children.push(at);
        }
        frames.tree.push(FrameNode {
            mark: self.locals.len(),
            parent: self.timing.last().copied(),
            depth: self.timing.len(),
            children: Vec::new(),
            seed: std::mem::take(&mut seed),
            events: Vec::new(),
        });
        self.timing.push(at);
        self.timing_record(Event::Source(
            self.ctx.pc.clone(),
            "the program counter".to_owned(),
        ));
    }

    /// Records `event` in the innermost open frame, if any: once, not in every open frame.
    fn timing_record(&mut self, event: Event) {
        if let Some(&at) = self.timing.last() {
            self.findings.frames.record(at, event);
        }
    }

    /// Whether code checked here runs where the structure of its function does not show what
    /// runs beside it: inside a lambda, a task, the block of a `tasks` statement, or outside a
    /// function, as a handler's operation (ADR 0116 IF6). `τ_conc` is then `⊤`.
    pub(crate) fn unplaced(&self) -> bool {
        self.lambda_depth > 0
            || self.task_depth > 0
            || !self.tasks_frames.is_empty()
            || self.current.is_none()
    }

    /// The node of the call graph whose code is being checked (ADR 0116 P1): a lambda, then a
    /// handler's operation, then a function; `None` elsewhere, as in a constant.
    pub(crate) fn caller(&self) -> Option<Caller> {
        if self.lambda_depth > 0 {
            Some(Caller::Lambda)
        } else if let Some(handler) = self.handler {
            Some(Caller::Handler(handler))
        } else {
            self.current.map(Caller::Function)
        }
    }

    /// The function or handler whose code is being checked, a lambda written there included:
    /// a handler's operation, then a function; `None` elsewhere.
    pub(crate) fn home(&self) -> Option<Caller> {
        match self.handler {
            Some(handler) => Some(Caller::Handler(handler)),
            None => self.current.map(Caller::Function),
        }
    }

    /// Records an edge of the call graph of ADR 0116 P1 from the node whose code is being
    /// checked to `to`, at the call site `span`, with the innermost construct whose part holds
    /// it (P3).
    fn graph_edge(&mut self, to: Target, span: una_syntax::Span) {
        let construct = self.parts.last().copied();
        let home = self.home();
        self.findings.graph_edges.push(GraphEdge {
            from: self.caller(),
            home,
            to,
            span,
            construct,
        });
    }

    /// Records a call of, or a reference to, the function `callee` at `span`.
    pub(crate) fn graph_call(&mut self, callee: usize, span: una_syntax::Span) {
        self.graph_edge(Target::Function(callee), span);
    }

    /// Records that the code being checked performs the operation `op` of `capability` at
    /// `span`, an edge to the operation from its node, `Λ` for a lambda's body: without it, a
    /// request written directly in a lambda would reach nothing.
    pub(crate) fn graph_op(&mut self, capability: CapId, op: &str, span: una_syntax::Span) {
        self.graph_edge(Target::Op(capability, op.to_owned()), span);
    }

    /// Records that the code being checked calls a function value at `span`, so it reaches
    /// `Λ`.
    pub(crate) fn graph_value_call(&mut self, span: una_syntax::Span) {
        self.graph_edge(Target::Lambda, span);
    }

    /// Records that the code being checked binds, or receives from an operation, a value whose
    /// deep label is `label`: in its node of the graph of ADR 0116 P1, and once in the innermost
    /// open site ([`TimingFrames`]), for the reach of a `start` (ADR 0123 IF6, #1674). O(log n).
    pub(crate) fn reach_label(&mut self, label: &Label) {
        if label.is_public() {
            return;
        }
        if let Some(from) = self.caller() {
            let slot = self
                .findings
                .reach_labels
                .entry(from)
                .or_insert_with(Label::public);
            *slot = slot.join(label);
        }
        self.timing_record(Event::Bound(label.clone()));
    }

    /// Records that the code being checked installs the handler `handler` at `span`.
    pub(crate) fn graph_install(&mut self, handler: usize, span: una_syntax::Span) {
        self.graph_edge(Target::Install(handler), span);
    }

    /// Opens the task construct `kind` at `span` (ADR 0116 P2), a node of the construct tree
    /// of the code being checked (P3), and returns its id; its parts are checked between
    /// [`Self::part_open`] and [`Self::part_close`].
    pub(crate) fn construct_open(&mut self, kind: ConstructKind, span: una_syntax::Span) -> usize {
        let id = self.findings.constructs.len();
        self.findings.constructs.push(ConstructNode {
            kind,
            span,
            node: self.caller(),
            parent: self.parts.last().copied(),
            // `⊤` until `construct_seed` records the frame, so a construct whose frame is never
            // recorded fails closed (P4, F item 6).
            frame: None,
        });
        id
    }

    /// Records `frame`, that of `κ` (P2), for the construct `id`, once its parts are checked.
    pub(crate) fn construct_seed(&mut self, id: usize, frame: TimingFrame) {
        if let Some(construct) = self.findings.constructs.get_mut(id) {
            construct.frame = Some(frame);
        }
    }

    /// Starts checking a part of the construct `id`: the edges and sites recorded until
    /// [`Self::part_close`] lie in it, unless a construct nested in it holds them.
    pub(crate) fn part_open(&mut self, id: usize) {
        self.parts.push(id);
    }

    /// Ends the part that [`Self::part_open`] started.
    pub(crate) fn part_close(&mut self) {
        self.parts.pop();
    }

    /// Records an operation of the host's `Http` at `span`, for `L_lim` (ADR 0117 IF4).
    pub(crate) fn http_site(&mut self, span: una_syntax::Span) {
        if self.pass != super::authority::Pass::Check {
            return;
        }
        let site = HttpSite {
            span,
            function: self.current,
            unplaced: self.unplaced(),
            node: self.caller(),
            construct: self.parts.last().copied(),
        };
        self.findings.http_sites.push(site);
    }

    /// Records a reading of the host's `Clock` at `span`, for `L_clk` (ADR 0116 amendment of
    /// 2026-11-09, #898, C1). Recorded in every pass, so that pass 1 tells whether the probe of
    /// pass 2 must compute the readings' labels; only pass 2's sites are labeled.
    pub(crate) fn clock_site(&mut self, span: una_syntax::Span) {
        let site = HttpSite {
            span,
            function: self.current,
            unplaced: self.unplaced(),
            node: self.caller(),
            construct: self.parts.last().copied(),
        };
        self.findings.clock_sites.push(site);
    }

    /// Marks the innermost site as placed: an example's `do` line is the root of its run, which
    /// a test runs alone, so nothing runs beside it, as nothing runs beside `main` (ADR 0116
    /// IF6); the code of a task or a lambda inside it stays unplaced by its own sites.
    pub(crate) fn timing_placed(&mut self) {
        if let Some(&at) = self.timing.last()
            && let Some(frame) = self.findings.frames.tree.get_mut(at)
        {
            frame.seed.unplaced = false;
        }
    }

    /// Ends the innermost site of [`Self::timing_start`] and returns its frame, whose seed the
    /// consumers rebuild from its subtree ([`TimingFrames::seed`]). Without an open site, a
    /// frame with no item.
    pub(crate) fn timing_end(&mut self) -> TimingFrame {
        match self.timing.pop() {
            Some(at) => TimingFrame(at),
            None => {
                let frames = &mut self.findings.frames;
                let at = frames.tree.len();
                frames.tree.push(FrameNode::default());
                TimingFrame(at)
            }
        }
    }

    /// Adds `label` from `source` to the innermost site, such as a deadline's label (IF5 item
    /// 5).
    pub(crate) fn timing_source(&mut self, label: Label, source: String) {
        self.timing_record(Event::Source(label, source));
    }

    /// Records a read of the local `name` in every site it is free in (IF5 item 2).
    pub(crate) fn timing_read(&mut self, name: &str) {
        if self.timing.is_empty() {
            return;
        }
        let Some(position) = self.locals.position(name) else {
            return;
        };
        let Some(ty) = self.locals.get(position).map(|local| local.ty.clone()) else {
            return;
        };
        let label = self.deep(&ty);
        // The outermost open frame the read is free in, by a binary search: the marks of the
        // open frames never decrease inward, since a site's locals end before it does.
        let frames = &self.findings.frames;
        let outer = self.timing.partition_point(|&at| {
            // Counted, so that a test sees the O(log d) of the search (slice 8b of #1087).
            una_core::label_counters::frame_step();
            frames
                .tree
                .get(at)
                .is_none_or(|frame| frame.mark <= position)
        });
        self.timing_record(Event::Read(label, name.to_owned(), position, outer));
    }

    /// Records a value of type `ty` computed by the code being checked (#1685): `ℓ*` of `ty`,
    /// its symbolic labels dropped, joins the values of the open sites (once, in the innermost,
    /// [`TimingFrames`]), of the function being
    /// checked ([`Findings::body_labels`]) and of the handler being checked. A type parameter's
    /// label `ℓ_T` is dropped, since the values of `T` come from the callers, whose own code
    /// counts them. O(size of `ty`) per expression, so checking a body stays linear.
    pub(crate) fn timing_value(&mut self, ty: &Ty) {
        let label = self.deep(ty).concrete();
        if label.is_public() {
            return;
        }
        fn raise(values: &mut Option<Label>, label: &Label) {
            match values {
                Some(values) if label.flows_to(values) => {}
                Some(values) => *values = values.join(label),
                None => *values = Some(label.clone()),
            }
        }
        self.timing_record(Event::Value(label.clone()));
        if let Some(current) = self.current {
            let body = self.findings.body_labels.entry(current).or_insert(None);
            raise(body, &label);
        }
        if let Some(seed) = self.handler_seed() {
            raise(&mut seed.values, &label);
        }
    }

    /// The seed of the handler whose operation bodies are being checked, lambdas and tasks
    /// written in them included, which [`timing`] takes for every handler a site reaches (ADR
    /// 0116 IF5 item 3, #1436); `None` outside a handler.
    fn handler_seed(&mut self) -> Option<&mut TimingSeed> {
        let handler = self.handler?;
        Some(self.findings.handler_seeds.entry(handler).or_default())
    }

    /// Records a call of, or a reference to, the function `callee` for the open sites (once, in
    /// the innermost, [`TimingFrames`]), and in
    /// the seed of the handler being checked.
    pub(crate) fn timing_callee(&mut self, callee: usize) {
        self.timing_record(Event::Callee(callee));
        if let Some(seed) = self.handler_seed() {
            seed.callees.insert(callee);
        }
    }

    /// Records the operation `op` of `capability` for the open sites (once, in the innermost,
    /// [`TimingFrames`]), and in the seed of the
    /// handler being checked.
    pub(crate) fn timing_op(&mut self, capability: CapId, op: &str) {
        self.timing_record(Event::Op(capability, op.to_owned()));
        if let Some(seed) = self.handler_seed() {
            seed.ops.insert((capability, op.to_owned()));
        }
    }

    /// Records the installation of the handler `handler` for the open sites (once, in the
    /// innermost, [`TimingFrames`]), and in the seed
    /// of the handler being checked.
    pub(crate) fn timing_install(&mut self, handler: usize) {
        self.timing_record(Event::Install(handler));
        if let Some(seed) = self.handler_seed() {
            seed.handlers.insert(handler);
        }
    }

    /// Records a call of a function value for the open sites (once, in the innermost,
    /// [`TimingFrames`]; IF5 item 4), and in the seed of
    /// the handler being checked (#1436).
    pub(crate) fn timing_value_call(&mut self) {
        self.timing_record(Event::ValueCall);
        if let Some(seed) = self.handler_seed() {
            seed.value_call = true;
        }
    }

    /// Records, for the open sites (once, in the innermost, [`TimingFrames`]), for the function
    /// being checked and in the seed of the
    /// handler being checked, a call of the prelude
    /// function `callee`, one that applies a function argument ([`applies_function`]), whose
    /// code is not known at the call, or a use of `callee` as a value (IF5 item 4, #1023).
    pub(crate) fn timing_opaque_apply(&mut self, callee: usize) {
        if let Some(current) = self.current {
            self.findings.opaque_applies.insert((current, callee));
        }
        self.timing_record(Event::Opaque(callee));
        if let Some(seed) = self.handler_seed() {
            seed.opaque_applies.insert(callee);
        }
    }

    /// The clause that names the residue label `L_res` (ADR 0116 P9) in a message about a
    /// timing label above `bound`, when `L_res` alone is above it, and nothing otherwise: one
    /// clause after a semicolon, so the message stays one sentence.
    pub(crate) fn residue_clause(&self, bound: &Label) -> String {
        match self.env.residue.culprit(bound) {
            Some((label, source)) => format!(
                "; {source} raise the timing label of every deadline and race to `{label}` at least"
            ),
            None => String::new(),
        }
    }

    /// [`Self::residue_clause`] for a value labeled `value` that flows into `bound`: only when
    /// `L_res` alone could have raised the value to its label, through a deadline's or a race's
    /// outcome.
    pub(crate) fn residue_clause_for(&self, value: &Label, bound: &Label) -> String {
        let residue = &self.env.residue.label;
        if residue.is_public() || !value.flows_to(residue) {
            return String::new();
        }
        self.residue_clause(bound)
    }
}

/// What raised a request's label, for the message of U4001 (ADR 0117 IF4, ADR 0116 P8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Raiser {
    /// The nearest ancestor of the request in the call graph whose body is above `internal`, the
    /// function that makes the request included: how a message names it, its body's label, and
    /// whether it is the function that makes the request.
    Ancestor(String, Label, bool),
    /// Tasks that can run beside the request (`τ_conc` of the preview, `⊤`).
    Beside,
    /// The requests that cancelled tasks or the losers of a race can leave running (`L_res`,
    /// ADR 0116 P9): how a message names the construct, and its label.
    Residue(String, Label),
}

/// The label of one request for `L_lim`, and what raised it.
#[derive(Clone, Debug)]
pub(crate) struct LimitSite {
    /// The request.
    pub(crate) span: una_syntax::Span,
    /// The join of `τ⁰` of the bodies of every ancestor of its function, its function included,
    /// and `τ_conc` at it.
    pub(crate) label: Label,
    /// What raised that label above `internal`, when it is.
    pub(crate) raiser: Option<Raiser>,
}

/// `L_lim` of the package, and the label of each request (ADR 0117 IF4, ADR 0116 P8, slice T1).
#[derive(Clone, Debug)]
pub(crate) struct LimitLabels {
    /// `L_lim`: the join of the requests' labels; `public` without a request.
    pub(crate) label: Label,
    /// Each request, in the order its checking recorded it.
    pub(crate) sites: Vec<LimitSite>,
}

/// The join of `local` over every node reachable from each node along `succ`, the node itself
/// included, by Tarjan's algorithm, iteratively: a component is complete only after every
/// component it reaches, so its value is its members' `local` joined with the values of the
/// components it reaches. O(nodes + edges) joins.
fn close<T: Clone>(succ: &[Vec<usize>], local: &[T], join: impl Fn(&T, &T) -> T) -> Vec<T> {
    let n = succ.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut value: Vec<Option<T>> = vec![None; n];
    let mut next = 0usize;
    for root in 0..n {
        if index.get(root) != Some(&usize::MAX) {
            continue;
        }
        // Frames of the iterative search: a node and the position of its next successor.
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        if let (Some(i), Some(l)) = (index.get_mut(root), low.get_mut(root)) {
            *i = next;
            *l = next;
        }
        next += 1;
        stack.push(root);
        if let Some(on) = on_stack.get_mut(root) {
            *on = true;
        }
        while let Some(&mut (node, ref mut at)) = frames.last_mut() {
            let successors = succ.get(node).map_or(&[][..], Vec::as_slice);
            if let Some(&to) = successors.get(*at) {
                *at += 1;
                una_core::label_counters::pass_step();
                let to_index = index.get(to).copied().unwrap_or(0);
                if to_index == usize::MAX {
                    if let (Some(i), Some(l)) = (index.get_mut(to), low.get_mut(to)) {
                        *i = next;
                        *l = next;
                    }
                    next += 1;
                    stack.push(to);
                    if let Some(on) = on_stack.get_mut(to) {
                        *on = true;
                    }
                    frames.push((to, 0));
                } else if on_stack.get(to).copied().unwrap_or(false) {
                    let node_low = low.get(node).copied().unwrap_or(0);
                    if let Some(l) = low.get_mut(node) {
                        *l = node_low.min(to_index);
                    }
                }
                continue;
            }
            frames.pop();
            let node_low = low.get(node).copied().unwrap_or(0);
            if let Some(&(parent, _)) = frames.last() {
                let parent_low = low.get(parent).copied().unwrap_or(0);
                if let Some(l) = low.get_mut(parent) {
                    *l = parent_low.min(node_low);
                }
            }
            if Some(&node_low) != index.get(node) {
                continue;
            }
            // `node` roots a component: pop it, then join its members' values with those of
            // the components it reaches, all complete by now.
            let mut members = Vec::new();
            while let Some(member) = stack.pop() {
                if let Some(on) = on_stack.get_mut(member) {
                    *on = false;
                }
                members.push(member);
                if member == node {
                    break;
                }
            }
            let mut joined: Option<T> = None;
            for &member in &members {
                let mut parts: Vec<&T> = Vec::new();
                if let Some(own) = local.get(member) {
                    parts.push(own);
                }
                for to in succ.get(member).into_iter().flatten() {
                    if let Some(Some(done)) = value.get(*to) {
                        parts.push(done);
                    }
                }
                for part in parts {
                    joined = Some(match &joined {
                        Some(so_far) => join(so_far, part),
                        None => part.clone(),
                    });
                }
            }
            for &member in &members {
                if let Some(slot) = value.get_mut(member) {
                    *slot = joined.clone();
                }
            }
        }
    }
    value
        .into_iter()
        .zip(local)
        .map(|(done, own)| done.unwrap_or_else(|| own.clone()))
        .collect()
}

/// The call graph of ADR 0116 P1, built from the calls the checking recorded
/// ([`Checker::graph_call`]), not from the call edges of functions alone, with what each node
/// reaches:
/// - a node per function, per handler, per capability and per operation, and one node `Λ` for
///   every lambda;
/// - a function or a handler goes to what its code calls, to the operations of its `uses`, to
///   the handlers it installs, and to `Λ` when it calls a function value; a prelude function
///   with a function parameter goes to `Λ` too;
/// - an operation goes to its capability, a capability to every handler of it, and `Λ` to what
///   lambdas call and to every function named as a value.
///
/// So the operations of a handler, which a function reaches through an operation of the
/// handler's capability, are reached from it, and so is what a lambda calls, from every node
/// that calls a function value.
pub(crate) struct Graph {
    /// The successors of each node.
    succ: Vec<Vec<usize>>,
    /// What each node adds to `down` itself: `⊤` for a function that calls a function value,
    /// for a prelude function with a function parameter and for `Λ` (IF5 item 4), a handler's
    /// parameters, an operation's `ρ`.
    local: Vec<Label>,
    /// The join of `local` over every node each node reaches, itself included: `τ⁰` of what
    /// its code can run, without a function's parameters.
    down: Vec<Label>,
    /// Whether each node can reach an operation of the host's `Http` (ADR 0116 P9).
    http: Vec<bool>,
    /// The number of functions, which are the first nodes.
    functions: usize,
    /// The number of handlers, the nodes after the functions.
    handlers: usize,
    /// The number of capabilities, the nodes after the handlers.
    capabilities: usize,
    /// The node of each operation named by any `uses`, after the capabilities.
    ops: BTreeMap<(CapId, String), usize>,
    /// The node `Λ`, the last one.
    lambda: usize,
}

impl Graph {
    /// The node of a handler.
    fn handler_node(&self, handler: usize) -> usize {
        self.functions + handler
    }

    /// The node of a capability.
    fn cap_node(&self, cap: usize) -> usize {
        self.functions + self.handlers + cap
    }

    /// The node of a caller, when it is one of the graph.
    fn node_of(&self, caller: Caller) -> Option<usize> {
        match caller {
            Caller::Function(f) => (f < self.functions).then_some(f),
            Caller::Handler(h) => (h < self.handlers).then(|| self.handler_node(h)),
            Caller::Lambda => Some(self.lambda),
        }
    }
}

/// Whether `def` is a function of the prelude, or of another standard module, with a parameter
/// of function type, such as `map`, `filter` or `fold`: one that applies a function argument
/// during the call. No such function keeps the argument of a function-typed parameter after
/// the call: its result holds no copy of it. An argument of another parameter may be returned,
/// as `fold` returns `initial`, so only the former can be known code (IF5 item 4, #1023).
pub(crate) fn applies_function(env: &Env<'_>, def: &super::env::FnDef<'_>) -> bool {
    def.module >= env.prelude
        && def
            .params
            .iter()
            .any(|p| matches!(p.ty.base.unlabeled(), Ty::Fn(_)))
}

/// `ρ` of the operation `op` of `cap` (IF5 item 3): `ℓ*` of its result, its `accepts` label and
/// `ℓ*` of its failure kind; `None` for an operation the capability does not declare.
fn rho(env: &Env<'_>, cap: CapId, op: &str) -> Option<Label> {
    let kinds = env.kind_labels.as_ref();
    let def = env
        .capabilities
        .get(cap)?
        .ops
        .iter()
        .find(|d| d.name == op)?;
    let mut rho = def.result.base.deep_label(kinds);
    if let Some(accepts) = &def.accepts {
        rho = rho.join(accepts);
    }
    if let Some(fails) = &def.fails {
        rho = rho.join(&fails.deep_label(kinds));
    }
    Some(rho)
}

/// Whether `cap` is the host's `Http`, the capability whose interrupted requests can leave
/// work on helpers (ADR 0116 P9).
fn is_http(env: &Env<'_>, cap: CapId) -> bool {
    env.capabilities
        .get(cap)
        .is_some_and(|c| c.module == env.prelude && c.name == "Http")
}

/// The graph of ADR 0116 P1 for the package, with `down` and the reach of `Http`, each closed in
/// one pass over the strongly connected components: O(F + H + C + O + E) label joins and steps,
/// for `F` functions, `H` handlers, `C` capabilities, `O` operations and `E` edges.
pub(crate) fn graph(env: &Env<'_>, findings: &Findings) -> Graph {
    fn add(succ: &mut [Vec<usize>], from: usize, to: usize) {
        if let Some(edges) = succ.get_mut(from) {
            edges.push(to);
        }
    }
    fn op_edges(ops: &BTreeMap<(CapId, String), usize>, uses: &[super::env::Need]) -> Vec<usize> {
        let mut out = Vec::new();
        for need in uses {
            if let Some(cap) = need.usage.capability {
                for op in &need.usage.ops {
                    out.extend(ops.get(&(cap, op.clone())).copied());
                }
            }
        }
        out
    }
    let top = Label::top(&env.categories);
    let functions = env.functions.len();
    let handlers = env.handlers.len();
    let capabilities = env.capabilities.len();
    // The operations named by any `uses`, each one node after the capabilities; `Λ` last.
    let mut ops: BTreeMap<(CapId, String), usize> = BTreeMap::new();
    let needs = env
        .functions
        .iter()
        .flat_map(|f| f.uses.iter())
        .chain(env.handlers.iter().flat_map(|h| h.uses.iter()));
    for need in needs {
        if let Some(cap) = need.usage.capability {
            for op in &need.usage.ops {
                let at = functions + handlers + capabilities + ops.len();
                ops.entry((cap, op.clone())).or_insert(at);
            }
        }
    }
    // The operations performed where no `uses` names them, as in a lambda's body (#1148).
    for edge in &findings.graph_edges {
        if let Target::Op(cap, op) = &edge.to
            && *cap < capabilities
        {
            let at = functions + handlers + capabilities + ops.len();
            ops.entry((*cap, op.clone())).or_insert(at);
        }
    }
    let lambda = functions + handlers + capabilities + ops.len();
    let nodes = lambda + 1;
    let mut graph = Graph {
        succ: vec![Vec::new(); nodes],
        local: vec![Label::public(); nodes],
        down: Vec::new(),
        http: Vec::new(),
        functions,
        handlers,
        capabilities,
        ops,
        lambda,
    };
    for (f, def) in env.functions.iter().enumerate() {
        // What its body computes (#1685), as `timing` counts it.
        if let Some(Some(body)) = findings.body_labels.get(&f)
            && let Some(slot) = graph.local.get_mut(f)
        {
            *slot = slot.join(body);
        }
        // A call of a function value anywhere in the function, its lambdas' included (IF5 item
        // 4), as `timing` counts it.
        if findings.value_callers.contains(&f)
            && let Some(slot) = graph.local.get_mut(f)
        {
            *slot = top.clone();
        }
        // A prelude function that applies a function argument, such as `map`, runs lambdas.
        // The graph keeps `⊤` for it whatever its arguments: all lambdas are one node `Λ`
        // here, so a lambda's captures are not told apart (#1023 refines only `timing`).
        if applies_function(env, def) {
            if let Some(slot) = graph.local.get_mut(f) {
                *slot = top.clone();
            }
            add(&mut graph.succ, f, lambda);
        }
        for to in op_edges(&graph.ops, &def.uses) {
            add(&mut graph.succ, f, to);
        }
    }
    for (h, handler) in env.handlers.iter().enumerate() {
        let mut own = Label::public();
        // `⊤` for a parameter whose type may hold a function (#1436).
        for param in &handler.params {
            own = own.join(&handler_param_label(env, &param.ty.base));
        }
        // What its operation bodies compute (#1685), as `timing` counts it.
        if let Some(values) = findings
            .handler_seeds
            .get(&h)
            .and_then(|b| b.values.as_ref())
        {
            own = own.join(values);
        }
        let node = graph.handler_node(h);
        if let Some(slot) = graph.local.get_mut(node) {
            *slot = own;
        }
        for to in op_edges(&graph.ops, &handler.uses) {
            add(&mut graph.succ, node, to);
        }
        // Every handler of a capability can serve its operations (IF5 item 3, all handlers).
        if let Some(cap) = handler.capability {
            let from = graph.cap_node(cap);
            add(&mut graph.succ, from, node);
        }
    }
    // Each edge checking recorded at its call site (P1): a call or a function named as a
    // value, a call of a function value (to `Λ`), an operation (#1148), an installation.
    for edge in &findings.graph_edges {
        let Some(from) = edge.from.and_then(|from| graph.node_of(from)) else {
            continue;
        };
        match &edge.to {
            Target::Function(callee) if *callee < functions => add(&mut graph.succ, from, *callee),
            Target::Function(_) => {}
            Target::Lambda => {
                if let Some(slot) = graph.local.get_mut(from) {
                    *slot = top.clone();
                }
                add(&mut graph.succ, from, lambda);
            }
            Target::Op(cap, op) => {
                if let Some(&to) = graph.ops.get(&(*cap, op.clone())) {
                    add(&mut graph.succ, from, to);
                }
            }
            Target::Install(h) if *h < handlers => {
                let to = graph.handler_node(*h);
                add(&mut graph.succ, from, to);
            }
            Target::Install(_) => {}
        }
    }
    // A function named as a value may run wherever a value is called.
    for &f in &findings.function_values {
        if f < functions {
            add(&mut graph.succ, lambda, f);
        }
    }
    if let Some(slot) = graph.local.get_mut(lambda) {
        *slot = top.clone();
    }
    let mut http_local = vec![false; nodes];
    let op_nodes: Vec<((CapId, String), usize)> = graph
        .ops
        .iter()
        .map(|(key, &node)| (key.clone(), node))
        .collect();
    for ((cap, op), node) in op_nodes {
        if let Some(rho) = rho(env, cap, &op)
            && let Some(slot) = graph.local.get_mut(node)
        {
            *slot = rho;
        }
        if is_http(env, cap)
            && let Some(slot) = http_local.get_mut(node)
        {
            *slot = true;
        }
        let to = graph.cap_node(cap);
        add(&mut graph.succ, node, to);
    }
    // One edge per pair of nodes, in the order of the nodes: the edges come per call site,
    // so the searches below meet them in an order that does not depend on the source's.
    // O(E log E) in all.
    for edges in &mut graph.succ {
        edges.sort_unstable();
        edges.dedup();
    }
    graph.down = close(&graph.succ, &graph.local, |a: &Label, b: &Label| a.join(b));
    graph.http = close(&graph.succ, &http_local, |a: &bool, b: &bool| *a || *b);
    graph
}

/// `τ⁰` of a site from its seed alone, over the graph (ADR 0116 IF5), and whether it can reach
/// an operation of the host's `Http`: its own sources, `down` of every function it calls,
/// handler it installs and operation it performs, and `⊤` with `Λ` for a call of a function
/// value. O(size of the seed) lookups, so a pass over every seed is linear in what the checking
/// recorded.
fn seed_label(env: &Env<'_>, graph: &Graph, seed: &TimingSeed) -> (Label, bool) {
    let top = Label::top(&env.categories);
    let mut label = seed.sources.iter().fold(
        seed.values.clone().unwrap_or_else(Label::public),
        |tau, (label, _)| tau.join(label),
    );
    let mut http = false;
    let reach = |node: usize, label: &mut Label, http: &mut bool| {
        if let Some(down) = graph.down.get(node) {
            *label = label.join(down);
        }
        *http |= graph.http.get(node).copied().unwrap_or(false);
    };
    for &callee in &seed.callees {
        if callee < graph.functions {
            reach(callee, &mut label, &mut http);
        }
    }
    for &handler in &seed.handlers {
        if handler < graph.handlers {
            reach(graph.handler_node(handler), &mut label, &mut http);
        }
    }
    for (cap, op) in &seed.ops {
        match graph.ops.get(&(*cap, op.clone())) {
            Some(&node) => reach(node, &mut label, &mut http),
            // An operation no `uses` names: its `ρ`, and its capability's handlers.
            None => {
                if let Some(rho) = rho(env, *cap, op) {
                    label = label.join(&rho);
                }
                http |= is_http(env, *cap);
                if *cap < graph.capabilities {
                    reach(graph.cap_node(*cap), &mut label, &mut http);
                }
            }
        }
    }
    if seed.value_call {
        label = label.join(&top);
        reach(graph.lambda, &mut label, &mut http);
    }
    (label, http)
}

/// [`seed_label`] of every frame of `frames`, without rebuilding a seed (slice 8b of #1087): its
/// own sources and the reads free in it ([`TimingFrames::sources_up`]), joined with what the
/// items of its subtree reach over the graph of P1 ([`TimingFrames::fold_up`]), and whether
/// they reach an operation of `Http`. O(F + N) joins and O((R + F) log F) for the reads.
fn coarse_labels(env: &Env<'_>, graph: &Graph, frames: &TimingFrames) -> Vec<(Label, bool)> {
    let top = Label::top(&env.categories);
    let reached = |node: usize| {
        Some((
            graph.down.get(node).cloned().unwrap_or_else(Label::public),
            graph.http.get(node).copied().unwrap_or(false),
        ))
    };
    let item = |event: &Event| -> Option<(Label, bool)> {
        match event {
            Event::Callee(f) if *f < graph.functions => reached(*f),
            Event::Install(h) if *h < graph.handlers => reached(graph.handler_node(*h)),
            Event::Op(cap, op) => match graph.ops.get(&(*cap, op.clone())) {
                Some(&node) => reached(node),
                // An operation no `uses` names: its `ρ`, and its capability's handlers.
                None => {
                    let mut label = rho(env, *cap, op).unwrap_or_else(Label::public);
                    let mut http = is_http(env, *cap);
                    if *cap < graph.capabilities
                        && let Some((down, more)) = reached(graph.cap_node(*cap))
                    {
                        label = label.join(&down);
                        http |= more;
                    }
                    Some((label, http))
                }
            },
            Event::ValueCall => {
                let (down, http) = reached(graph.lambda).unwrap_or((Label::public(), false));
                Some((top.join(&down), http))
            }
            Event::Value(label) => Some((label.clone(), false)),
            _ => None,
        }
    };
    let items = frames.fold_up(&(Label::public(), false), item, |a, b| {
        (a.0.join(&b.0), a.1 || b.1)
    });
    let sources = frames.sources_up();
    items
        .into_iter()
        .enumerate()
        .map(|(at, (label, http))| {
            let label = sources
                .get(at)
                .map_or_else(|| label.clone(), |own| own.join(&label));
            (label, http)
        })
        .collect()
}

/// Why the code of a node can push frames without a bound that its own code shows, for the
/// reach of a `start` (ADR 0123 IF6, #1674).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unbounded {
    /// It lies on a cycle of the graph: it can recurse.
    Cycle(usize),
    /// `Λ`: a call of a function value, or of a `uses of` parameter, runs code that the call
    /// does not show.
    ValueCall,
    /// A function or handler of a standard module other than the prelude, or of a dependency
    /// not checked with the package, whose code the findings do not hold.
    Unseen(usize),
}

/// What each node of the graph of ADR 0116 P1 reaches, for IF6 (#1674): a node that can push
/// frames without a bound its code shows ([`Unbounded`]), and a node whose code can hold data
/// above `internal`, each as one witness. Both are closed once per package over the strongly
/// connected components, O(V + E), so each `start` costs O(size of its seed) lookups.
pub(crate) struct Reach {
    /// A node each node reaches, itself included, that is [`Unbounded`], with why.
    unbounded: Vec<Option<Unbounded>>,
    /// A node each node reaches, itself included, whose own label is above `internal`.
    labeled: Vec<Option<usize>>,
    /// Each node's own label: the deep labels of its signature, of what its code binds and of
    /// the answers of its operations, a function's and `Λ`'s without symbols.
    local: Vec<Label>,
    /// Whether each node reaches one that can hand the task a function value made outside it.
    foreign: Vec<bool>,
    /// Whether each node reaches `Λ`.
    to_lambda: Vec<bool>,
    /// Whether each node reaches a function with a call that data above `internal` decide, a
    /// check point of `spec/27 §8` rule 5, or a call that instantiates a generic function whose
    /// calls such data decide (#1682).
    decided: Vec<bool>,
    /// Whether each node reaches code whose calls the findings do not attribute to a function,
    /// or do not record with their labels: `Λ`, a handler's operations, a `via` function, a
    /// predicate or a body that evaluates a `checked` predicate, or code the checker does not
    /// see (#1682).
    imprecise: Vec<bool>,
    /// The label of `Λ` with its symbols, when a lambda of the package holds a value whose
    /// label is a type parameter's.
    lambda_symbols: Option<Label>,
    /// The number of functions, handlers and capabilities, to name a node.
    functions: usize,
    handlers: usize,
    capabilities: usize,
    /// The node of each operation, and `Λ`.
    ops: BTreeMap<(CapId, String), usize>,
    lambda: usize,
}

/// Whether each node lies on a cycle of `succ`: in a strongly connected component of more than
/// one node, or with an edge to itself. Tarjan's algorithm, iteratively, O(V + E).
fn on_cycle(succ: &[Vec<usize>]) -> Vec<bool> {
    // Each node's index and low link, unset as `usize::MAX`, and whether it is on the stack.
    let n = succ.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut cyclic = vec![false; n];
    let mut next = 0usize;
    let get = |values: &[usize], node: usize| values.get(node).copied().unwrap_or(0);
    let set = |values: &mut [usize], node: usize, value: usize| {
        if let Some(slot) = values.get_mut(node) {
            *slot = value;
        }
    };
    let mark = |flags: &mut [bool], node: usize, value: bool| {
        if let Some(slot) = flags.get_mut(node) {
            *slot = value;
        }
    };
    for root in 0..n {
        if get(&index, root) != usize::MAX {
            continue;
        }
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        set(&mut index, root, next);
        set(&mut low, root, next);
        next += 1;
        stack.push(root);
        mark(&mut on_stack, root, true);
        while let Some(&mut (node, ref mut at)) = frames.last_mut() {
            una_core::label_counters::pass_step();
            let successors = succ.get(node).map_or(&[][..], Vec::as_slice);
            if let Some(&to) = successors.get(*at) {
                *at += 1;
                if get(&index, to) == usize::MAX {
                    set(&mut index, to, next);
                    set(&mut low, to, next);
                    next += 1;
                    stack.push(to);
                    mark(&mut on_stack, to, true);
                    frames.push((to, 0));
                } else if on_stack.get(to).copied().unwrap_or(false) {
                    let lowest = get(&low, node).min(get(&index, to));
                    set(&mut low, node, lowest);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                let lowest = get(&low, parent).min(get(&low, node));
                set(&mut low, parent, lowest);
            }
            if get(&low, node) != get(&index, node) {
                continue;
            }
            let mut members = Vec::new();
            while let Some(member) = stack.pop() {
                mark(&mut on_stack, member, false);
                members.push(member);
                if member == node {
                    break;
                }
            }
            let looped = members.len() > 1 || successors.contains(&node);
            for member in members {
                mark(&mut cyclic, member, looped);
            }
        }
    }
    cyclic
}

/// `label` without its symbolic labels `ℓ_T` (#1674), for the node of a generic function `g`
/// only, whose symbols are those of `g`'s own type parameters. In a task, `g` runs only when the
/// task calls it, so the values of its type parameters are what the task passes, which count
/// where the task holds them: in the seed of the `start`, kept with its symbols, since a `start`
/// in a generic body takes its type parameters' values from callers outside the task, or in a
/// node the task reaches.
fn concrete(label: &Label) -> Label {
    if label.symbols.is_empty() {
        return label.clone();
    }
    Label {
        symbols: Default::default(),
        ..label.clone()
    }
}

/// The [`Reach`] of the package: one graph, one search for cycles and two closures, O(V + E).
pub(crate) fn start_reaches(env: &Env<'_>, findings: &Findings) -> Reach {
    let graph = graph(env, findings);
    let kinds = env.kind_labels.as_ref();
    let nodes = graph.succ.len();
    let mut succ = graph.succ.clone();
    for &(policy, via) in &findings.policy_vias {
        if policy < graph.functions
            && via < graph.functions
            && let Some(edges) = succ.get_mut(policy)
        {
            edges.push(via);
        }
    }
    let cyclic = on_cycle(&succ);
    let apart = |module: usize| {
        let standard = module >= env.prelude;
        let prelude = env.modules.get(module).is_some_and(|m| m.path == "prelude");
        (standard && !prelude) || (!standard && !env.reports(module))
    };
    let mut local = vec![Label::public(); nodes];
    let mut own: Vec<Option<Unbounded>> = vec![None; nodes];
    // Whether a node can hand the task a function value made outside it: a handler whose
    // parameters may hold one, an operation whose answer may.
    let mut foreign = vec![false; nodes];
    for (f, def) in env.functions.iter().enumerate() {
        let mut label = def.result.base.deep_label(kinds);
        for param in &def.params {
            label = label.join(&param.ty.base.deep_label(kinds));
        }
        if let Some(slot) = local.get_mut(f) {
            *slot = label;
        }
        if apart(def.module)
            && let Some(slot) = own.get_mut(f)
        {
            *slot = Some(Unbounded::Unseen(f));
        }
    }
    for (h, handler) in env.handlers.iter().enumerate() {
        let node = graph.handler_node(h);
        let mut label = Label::public();
        for param in &handler.params {
            label = label.join(&param.ty.base.deep_label(kinds));
        }
        if let Some(slot) = local.get_mut(node) {
            *slot = label;
        }
        if handler
            .params
            .iter()
            .any(|param| holds_function(env, &param.ty.base))
            && let Some(slot) = foreign.get_mut(node)
        {
            *slot = true;
        }
        if apart(handler.module)
            && let Some(slot) = own.get_mut(node)
        {
            *slot = Some(Unbounded::Unseen(node));
        }
    }
    for ((cap, op), &node) in &graph.ops {
        if let Some(rho) = rho(env, *cap, op)
            && let Some(slot) = local.get_mut(node)
        {
            *slot = rho;
        }
        let answers_function = env
            .capabilities
            .get(*cap)
            .and_then(|c| c.ops.iter().find(|d| d.name == *op))
            .is_some_and(|d| holds_function(env, &d.result.base));
        if answers_function && let Some(slot) = foreign.get_mut(node) {
            *slot = true;
        }
    }
    for (caller, label) in &findings.reach_labels {
        if let Some(slot) = graph.node_of(*caller).and_then(|node| local.get_mut(node)) {
            *slot = slot.join(label);
        }
    }
    // A function's own symbols are what the task passes it (see [`concrete`]).
    for slot in local.iter_mut().take(graph.functions) {
        *slot = concrete(slot);
    }
    // `Λ` holds every lambda of the package, whose captures may carry the symbols of the
    // generic function that wrote it: kept apart, since a lambda made outside the task may
    // reach it (`lambda_symbols`).
    let lambda_symbols = local
        .get(graph.lambda)
        .filter(|label| !label.symbols.is_empty())
        .cloned();
    if let Some(slot) = local.get_mut(graph.lambda) {
        *slot = concrete(slot);
    }
    if let Some(slot) = own.get_mut(graph.lambda) {
        *slot = Some(Unbounded::ValueCall);
    }
    for (node, slot) in own.iter_mut().enumerate() {
        if slot.is_none() && cyclic.get(node).copied().unwrap_or(false) {
            *slot = Some(Unbounded::Cycle(node));
        }
    }
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    // #1682: the functions whose own calls data above `internal` decide, as the checking of
    // their bodies recorded them for `spec/27 §8` rule 5, whether or not that rule reports them.
    let mut decided_own = vec![false; nodes];
    for f in decided_functions(findings) {
        if f < graph.functions
            && let Some(slot) = decided_own.get_mut(f)
        {
            *slot = true;
        }
    }
    let mut imprecise_own = vec![false; nodes];
    for (node, slot) in imprecise_own.iter_mut().enumerate() {
        let handler = node >= graph.functions && node < graph.functions + graph.handlers;
        let unseen = matches!(own.get(node), Some(Some(Unbounded::Unseen(_))));
        // A `via` body is checked without labels (`spec/07 §5` rule 3), so what decides the
        // calls it and its callees make is not recorded.
        let via = node < graph.functions && env.vias.contains_key(&node);
        // A predicate, and a body that evaluates a `checked` predicate, count as imprecise,
        // conservatively: a `reflect` predicate's body records its check points, and `checked`
        // on labeled data is refused (`spec/07 §3`), so this only makes the test coarser.
        let unlabeled = node < graph.functions
            && (findings.unlabeled_checks.contains(&node)
                || env
                    .functions
                    .get(node)
                    .is_some_and(|def| matches!(def.kind, super::env::FnKind::Predicate { .. })));
        *slot = handler || unseen || via || unlabeled || node == graph.lambda;
    }
    let above: Vec<Option<usize>> = local
        .iter()
        .enumerate()
        .map(|(node, label)| (!label.flows_to(&internal)).then_some(node))
        .collect();
    Reach {
        unbounded: close(
            &succ,
            &own,
            |a: &Option<Unbounded>, b: &Option<Unbounded>| a.or(*b),
        ),
        labeled: close(&succ, &above, |a: &Option<usize>, b: &Option<usize>| {
            a.or(*b)
        }),
        foreign: close(&succ, &foreign, |a: &bool, b: &bool| *a || *b),
        decided: close(&succ, &decided_own, |a: &bool, b: &bool| *a || *b),
        imprecise: close(&succ, &imprecise_own, |a: &bool, b: &bool| *a || *b),
        to_lambda: {
            let mut is_lambda = vec![false; nodes];
            if let Some(slot) = is_lambda.get_mut(graph.lambda) {
                *slot = true;
            }
            close(&succ, &is_lambda, |a: &bool, b: &bool| *a || *b)
        },
        lambda_symbols,
        local,
        functions: graph.functions,
        handlers: graph.handlers,
        capabilities: graph.capabilities,
        ops: graph.ops,
        lambda: graph.lambda,
    }
}

/// The functions whose body holds a call, an operation or a loop that data above `internal`
/// decide (`spec/27 §8` rule 5), with a concrete label, lambdas written in them included, or a
/// call of a generic function whose calls a type argument above `internal` decides. O((sites +
/// calls) log n).
pub(crate) fn decided_functions(findings: &Findings) -> BTreeSet<usize> {
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let by_type = super::switch::decided_by_type(findings, &internal);
    let mut out: BTreeSet<usize> = BTreeSet::new();
    for site in &findings.switch_sites {
        if !site.label.concrete().flows_to(&internal)
            && let Some(f) = site.function
        {
            out.insert(f);
        }
    }
    for call in &findings.switch_calls {
        if by_type.contains(&call.callee)
            && !call.label.concrete().flows_to(&internal)
            && let Some(f) = call.caller
        {
            out.insert(f);
        }
    }
    out
}

/// What a `start` reaches (#1674), as [`start_reach`] finds it.
pub(crate) struct StartReach {
    /// A node it reaches that is [`Unbounded`].
    pub(crate) unbounded: Option<Unbounded>,
    /// A source of its own seed whose label is above the bound, as a message names it.
    pub(crate) source: Option<String>,
    /// A node it reaches whose own label is above `internal`.
    pub(crate) labeled: Option<usize>,
    /// The join of its own data and of that node's label.
    pub(crate) label: Label,
    /// Whether it reaches a function whose calls data above `internal` decide (#1682).
    pub(crate) decided: bool,
    /// Whether it reaches code whose calls are not attributed to a function, or its own data
    /// carry a type parameter's label: the precise test does not apply (#1682).
    pub(crate) imprecise: bool,
}

/// What a `start` whose expression's checking recorded `seed` reaches (#1674): a node that is
/// [`Unbounded`], and its own data or a node whose label is above `bound`, each as one witness
/// for a message. O(size of the seed) lookups.
///
/// A lambda that the task calls may have been made outside it, when its captures can hold a
/// function value (`captures_function`) or a node it reaches can hand it one: the symbols of
/// `Λ` then count, since the generic call that made the lambda need not be the task's.
pub(crate) fn start_reach(
    reach: &Reach,
    seed: &TimingSeed,
    captures_function: bool,
    bound: &Label,
) -> StartReach {
    let mut unbounded = None;
    let mut labeled: Option<usize> = None;
    let mut lambda = false;
    let mut foreign = captures_function;
    let mut decided = false;
    let mut imprecise = false;
    let mut look = |node: usize| {
        decided |= reach.decided.get(node).copied().unwrap_or(false);
        imprecise |= reach.imprecise.get(node).copied().unwrap_or(true);
        if unbounded.is_none() {
            unbounded = reach.unbounded.get(node).copied().flatten();
        }
        if labeled.is_none() {
            labeled = reach.labeled.get(node).copied().flatten();
        }
        lambda |= reach.to_lambda.get(node).copied().unwrap_or(false);
        foreign |= reach.foreign.get(node).copied().unwrap_or(false);
    };
    for &callee in &seed.callees {
        if callee < reach.functions {
            look(callee);
        }
    }
    for &handler in &seed.handlers {
        if handler < reach.handlers {
            look(reach.functions + handler);
        }
    }
    for (cap, op) in &seed.ops {
        match reach.ops.get(&(*cap, op.clone())) {
            Some(&node) => look(node),
            None if *cap < reach.capabilities => look(reach.functions + reach.handlers + cap),
            None => {}
        }
    }
    if seed.value_call || !seed.opaque_applies.is_empty() {
        look(reach.lambda);
    }
    // The site's own data: the program counter, the bindings it reads from outside it, what it
    // binds and receives itself.
    let mut label = seed.bound.clone().unwrap_or_else(Label::public);
    let mut source = None;
    if !label.flows_to(bound) {
        source = Some("what it binds".to_owned());
    }
    // Kept with their symbols: a `start` in a generic body takes the values of its type
    // parameters from callers outside the task.
    for (own, named) in &seed.sources {
        label = label.join(own);
        if source.is_none() && !own.flows_to(bound) {
            source = Some(named.clone());
        }
    }
    if let Some(own) = labeled.and_then(|node| reach.local.get(node)) {
        label = label.join(own);
    }
    if lambda
        && foreign
        && let Some(symbols) = &reach.lambda_symbols
    {
        label = label.join(symbols);
        if source.is_none() {
            source = Some(
                "a lambda that may be made outside the task, whose captures carry a type parameter's label"
                    .to_owned(),
            );
        }
    }
    // A `start` in a generic body takes its type parameters' values from callers outside it. A
    // lambda in its reach is `Λ`, imprecise already.
    imprecise |= !label.symbols.is_empty();
    StartReach {
        unbounded,
        source,
        labeled,
        label,
        decided,
        imprecise,
    }
}

impl Reach {
    /// How a message names the node `node` as code.
    pub(crate) fn name(&self, env: &Env<'_>, node: usize) -> String {
        if node < self.functions {
            if let Some(def) = env.functions.get(node) {
                return match env.modules.get(def.module).map(|m| m.path.as_str()) {
                    Some(path) if !path.is_empty() => format!("`{path}.{}`", def.name),
                    _ => format!("`{}`", def.name),
                };
            }
        } else if node < self.functions + self.handlers
            && let Some(handler) = env.handlers.get(node - self.functions)
        {
            return format!(
                "the handler `{}`",
                env.text(handler.module, handler.ast.name.name)
            );
        }
        if node == self.lambda {
            return "a lambda".to_owned();
        }
        self.ops
            .iter()
            .find(|(_, at)| **at == node)
            .and_then(|((cap, op), _)| {
                env.capabilities
                    .get(*cap)
                    .map(|c| format!("the operation `{}.{op}`", c.name))
            })
            .unwrap_or_else(|| "an operation".to_owned())
    }
}

/// A `tasks` statement checked (ADR 0117 rule 7), for the residue label of ADR 0116 P9: where
/// it is, the function whose body holds it, and the seed of `τ⁰` of the whole statement, its
/// bound, its block and every `start` in it.
#[derive(Clone, Debug)]
pub(crate) struct TasksSite {
    /// The statement.
    pub(crate) span: una_syntax::Span,
    /// The function whose body holds it.
    pub(crate) function: Option<usize>,
    /// The frame of `κ(T)`, recorded while the statement was checked.
    pub(crate) frame: TimingFrame,
}

/// The residue label `L_res` of ADR 0116 P9, with each construct that raises it, as a message
/// names it. Until the reactor ends an interrupted request with its task (ADR 0117 step 6), a
/// request whose task is cancelled before its own time-out keeps its helper, its slots and the
/// cores its exchange spends until its next I/O step, beside whatever code runs later. So every
/// timing site of the package (C), and `L_lim` (P10), joins it.
#[derive(Clone, Debug)]
pub(crate) struct Residue {
    /// `L_res`: `public` when no construct can leave a request behind.
    pub(crate) label: Label,
    /// Each construct whose requests can be left behind, with its label, in source order.
    pub(crate) sources: Vec<(Label, String)>,
}

impl Residue {
    /// No residue: a package with no `tasks` statement and no `first of` that reaches `Http`.
    pub(crate) fn none() -> Residue {
        Residue {
            label: Label::public(),
            sources: Vec::new(),
        }
    }

    /// The first source whose label does not flow to `bound`, for a message.
    pub(crate) fn culprit(&self, bound: &Label) -> Option<&(Label, String)> {
        self.sources
            .iter()
            .find(|(label, _)| !label.flows_to(bound))
    }

    /// `timing` with `L_res` joined in, each source after the site's own, so that a message
    /// names the site's own source first when it is above the bound (ADR 0116 C).
    pub(crate) fn join_into(&self, mut timing: Timing) -> Timing {
        if self.label.is_public() {
            return timing;
        }
        timing.label = timing.label.join(&self.label);
        timing.sources.extend(self.sources.iter().cloned());
        timing
    }
}

/// `L_res` of the package (ADR 0116 P9), as slice 4 of the amendment of 2026-10-25 lands it
/// (issue #1148), with the precise `τ_conc` of slice 4 of the plan on #1087. It is the join of:
/// - over every `tasks` statement `T` whose parts can reach an operation of `Http`,
///   `κ(T) ⊔ τ_conc(T)`;
/// - over every `first of` `F` whose branches can reach an operation of `Http`,
///   `t(F) ⊔ τ_conc(F)`, which is at least `internal`, from `ρ` of `Http`.
///
/// **The terms.** `κ` and `t` are `τ⁰` of the whole construct, its frame's label bottom up over
/// the graph of P1 ([`coarse_labels`], which a build with debug assertions holds to
/// [`seed_label`] of the rebuilt seed), so the reach of `Http` goes through handlers, capabilities and
/// `Λ`. `τ_conc` is that of the construct's own site, from [`Concurrency`] (`spec/27 §11`):
/// `conc` of its node joined with `ctx` at it, `⊤` in no node. `L_res` joins no timing label
/// directly; it reaches one only through the values of item 7, which the probe of pass 2 raises
/// to their fixpoint with the labels of the sites.
///
/// **Conservative.** It counts every such statement, not only one whose block can fail, since a
/// cancellation from outside it cancels its tasks too, and every such race. A `within`'s cut
/// leaves nothing: the helper ends the exchange at the covering deadline itself, and the task
/// waits for it (P9).
///
/// **Linear.** O(V + E) for the graph, which the caller builds once, and O(size of the seeds)
/// for the constructs, with one map of the sources' messages to merge a site checked twice.
pub(crate) fn residue(
    env: &Env<'_>,
    findings: &Findings,
    graph: &Graph,
    concurrency: &Concurrency,
) -> Residue {
    let name = |function: Option<usize>| -> String {
        function.and_then(|id| env.functions.get(id)).map_or_else(
            || "a handler's operation".to_owned(),
            |def| match env.modules.get(def.module).map(|m| m.path.as_str()) {
                Some(path) if !path.is_empty() => format!("`{path}.{}`", def.name),
                _ => format!("`{}`", def.name),
            },
        )
    };
    let mut residue = Residue::none();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let outside = outside_constructs(env, graph);
    let mut add = |label: Label, source: String, residue: &mut Residue| {
        residue.label = residue.label.join(&label);
        match index.get(&source) {
            Some(&at) => {
                if let Some((joined, _)) = residue.sources.get_mut(at) {
                    *joined = joined.join(&label);
                }
            }
            None => {
                index.insert(source.clone(), residue.sources.len());
                residue.sources.push((label, source));
            }
        }
    };
    // First, so that a message names them first: they are `⊤`.
    for (label, source) in outside {
        add(label, source, &mut residue);
    }
    // The precise `τ_conc` of the construct's own site (`spec/27 §11`, P9: "the analysis then
    // computes `L_res` with P4's labels").
    let conc = |seed: &TimingSeed| concurrency.tau_conc(seed).unwrap_or_else(Label::public);
    let mut tasks: Vec<&TasksSite> = findings.tasks_sites.iter().collect();
    tasks.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    for site in tasks {
        let (kappa, http) = concurrency.coarse(site.frame);
        if !http {
            continue;
        }
        let label = kappa.join(&conc(&findings.frames.own(site.frame)));
        add(
            label,
            format!(
                "the requests that the tasks of the `tasks` statement in {} can leave running when they are cancelled",
                name(site.function)
            ),
            &mut residue,
        );
    }
    let mut races: Vec<&super::within::WithinSite> = findings
        .withins
        .iter()
        .filter(|site| site.construct == super::within::Construct::FirstOf)
        .collect();
    races.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    for site in races {
        let (t, http) = concurrency.coarse(site.frame);
        if !http {
            continue;
        }
        let label = t.join(&conc(&findings.frames.own(site.frame)));
        add(
            label,
            format!(
                "the requests that the losing branches of the `first of` in {} can leave running",
                name(site.function)
            ),
            &mut residue,
        );
    }
    residue
}

/// Why a function the package reaches makes `L_res` `⊤` (#1148, failing closed until #1166).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unseen {
    /// A function of a standard module whose body holds a `tasks` statement or a `first of`:
    /// the standard modules are checked apart, so their constructs are not in the findings.
    StandardTasks,
    /// A function of a dependency whose code the checker does not see in the findings: a module
    /// of a dependency that was not checked with the package, or a function without a body.
    UncheckedDependency,
}

/// Whether a reached function makes `L_res` `⊤`, and why: `standard` when its module is a
/// standard one; `dependency` when it belongs to a dependency of the package; `checked` when
/// its body was checked into the package's findings, so its constructs count precisely; and
/// `holds_construct` when it has a body holding a `tasks` statement or a `first of`, `None`
/// when it has no body. The root package's own functions never make it `⊤`: their constructs
/// are in the findings.
pub(crate) fn unseen(
    standard: bool,
    dependency: bool,
    checked: bool,
    holds_construct: Option<bool>,
) -> Option<Unseen> {
    if standard {
        return (holds_construct == Some(true)).then_some(Unseen::StandardTasks);
    }
    if dependency && (!checked || holds_construct.is_none()) {
        return Some(Unseen::UncheckedDependency);
    }
    None
}

/// The nodes reachable along `succ` from `roots`, the roots included: one search, O(V + E).
pub(crate) fn reached_from(
    succ: &[Vec<usize>],
    roots: impl IntoIterator<Item = usize>,
) -> Vec<bool> {
    let mut seen = vec![false; succ.len()];
    let mut stack: Vec<usize> = Vec::new();
    for node in roots {
        if let Some(slot) = seen.get_mut(node)
            && !*slot
        {
            *slot = true;
            stack.push(node);
        }
    }
    while let Some(node) = stack.pop() {
        for &to in succ.get(node).map_or(&[][..], Vec::as_slice) {
            if let Some(slot) = seen.get_mut(to)
                && !*slot
            {
                *slot = true;
                stack.push(to);
            }
        }
    }
    seen
}

/// The `via` functions of the package and the code each reaches (`spec/07 §5` rule 7, ADR 0098
/// amendment of 2026-11-06, #1673), over the graph of ADR 0116 P1: the call edges, the edge of
/// each policy call to its `via`, the operations and every handler that can serve them.
///
/// **Lambdas.** All lambdas are one node `Λ` in that graph, so following its edges blindly
/// would count every lambda of the package as reached from a `via` that applies one lambda of
/// its own. Here a lambda's code is taken with the function or handler it is written in (its
/// `home`): a reached node reaches what its lambdas call, perform and install. `Λ`, every
/// lambda's code with every function named as a value, counts as reached only from a reached
/// node whose code can run code it does not show: it calls a function value, or applies a
/// prelude function to an argument other than a lambda written at the call (`value_callers`,
/// `opaque_applies`, the same facts as IF5 item 4 and #1023). A prelude function such as
/// `map` runs only the functions it is given, which those two facts cover, so its own edge to
/// `Λ` is not followed.
///
/// **Function values from outside** (#1710). Even then, `Λ` is opened only when a function
/// value made outside the code the `via`s reach can come into it ([`foreign_values`]): the
/// search runs first without `Λ`, and every function value that code calls is then a lambda
/// written in a reached node or a function named in one, which that search follows.
pub(crate) struct ViaReach {
    /// For each node, the `via` function the search reached it from first, `None` when no
    /// `via` function reaches it.
    by_node: Vec<Option<usize>>,
    /// For each node, whether the search reached it only through `Λ`.
    through_lambda: Vec<bool>,
    /// The number of functions and of handlers, and the node `Λ`, to find a caller's node as
    /// [`Graph::node_of`] does.
    functions: usize,
    handlers: usize,
    lambda: usize,
}

/// How a `via` function reaches a site, for [`ViaReach::via_of`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViaPath {
    /// The site is in the `via` body, a lambda written there included.
    Body,
    /// The site is in code the `via` body calls, performs or installs.
    Calls,
    /// The site is in a lambda's code, or code it reaches, and the `via` reaches `Λ`: it calls a
    /// function value or applies one whose code is not known.
    Lambda,
}

impl ViaReach {
    fn node(&self, caller: Caller) -> Option<usize> {
        match caller {
            Caller::Function(f) => (f < self.functions).then_some(f),
            Caller::Handler(h) => (h < self.handlers).then_some(self.functions + h),
            Caller::Lambda => Some(self.lambda),
        }
    }

    /// A `via` function that reaches the site whose checking recorded `seed`, and how: through
    /// the node its code is in, or, in a lambda, through the function or handler the lambda is
    /// written in. `None` when no `via` function reaches it.
    pub(crate) fn via_of(&self, seed: &TimingSeed) -> Option<(usize, ViaPath)> {
        let path = |node: usize, via: usize| {
            if self.through_lambda.get(node).copied().unwrap_or(false) {
                ViaPath::Lambda
            } else if node == via {
                ViaPath::Body
            } else {
                ViaPath::Calls
            }
        };
        let at = |caller: Option<Caller>| {
            let node = self.node(caller?)?;
            let via = self.by_node.get(node).copied().flatten()?;
            Some((via, path(node, via)))
        };
        if seed.node == Some(Caller::Lambda) {
            at(seed.home).or_else(|| at(Some(Caller::Lambda)))
        } else {
            at(seed.node)
        }
    }
}

/// [`ViaReach`] of the package: the graph of ADR 0116 P1, O(F + H + C + O + E), and one search
/// of it from every `via` function at once, each node taking the first `via` that reaches it,
/// with each lambda's edges taken from its home: O(V log F + E + A log A), for `A` opaque
/// applications, since each node looks up the sets of value callers and opaque appliers.
pub(crate) fn via_reach(env: &Env<'_>, findings: &Findings) -> ViaReach {
    let graph = graph(env, findings);
    let nodes = graph.succ.len();
    let lambda = graph.lambda;
    // The edges of the lambdas written in each function or handler, by its node.
    let mut written: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for edge in &findings.graph_edges {
        if edge.from != Some(Caller::Lambda) {
            continue;
        }
        let Some(home) = edge.home.and_then(|home| graph.node_of(home)) else {
            continue;
        };
        let to = match &edge.to {
            Target::Function(f) => (*f < graph.functions).then_some(*f),
            Target::Lambda => Some(lambda),
            Target::Op(cap, op) => graph.ops.get(&(*cap, op.clone())).copied(),
            Target::Install(h) => (*h < graph.handlers).then(|| graph.handler_node(*h)),
        };
        if let (Some(to), Some(slot)) = (to, written.get_mut(home)) {
            slot.push(to);
        }
    }
    // The nodes whose code runs code it does not show, and so reaches `Λ`.
    let opaque_functions: BTreeSet<usize> = findings
        .opaque_applies
        .iter()
        .map(|&(caller, _)| caller)
        .collect();
    let opens_lambda = |node: usize| {
        if node < graph.functions {
            findings.value_callers.contains(&node) || opaque_functions.contains(&node)
        } else if node < graph.functions + graph.handlers {
            findings
                .handler_seeds
                .get(&(node - graph.functions))
                .is_some_and(|seed| seed.value_call || !seed.opaque_applies.is_empty())
        } else {
            false
        }
    };
    // #1710: a search without `Λ` first. The function values that the code it reaches can
    // call are then made in that code, a lambda written in a node it reaches or a function it
    // names as a value, which it follows already, unless one can come from outside it
    // ([`foreign_values`]): only then is `Λ`, every lambda and every value, opened.
    let closed = search(&graph, &written, &opens_lambda, false, env);
    if !foreign_values(env, &graph, &opens_lambda, &closed.by_node) {
        return closed;
    }
    search(&graph, &written, &opens_lambda, true, env)
}

/// The search of [`via_reach`] from every `via` function at once, with `Λ` opened from code
/// that runs code it does not show when `open_lambda`, else never. O(V log F + E).
fn search(
    graph: &Graph,
    written: &[Vec<usize>],
    opens_lambda: &dyn Fn(usize) -> bool,
    open_lambda: bool,
    env: &Env<'_>,
) -> ViaReach {
    let nodes = graph.succ.len();
    let lambda = graph.lambda;
    let mut by_node: Vec<Option<usize>> = vec![None; nodes];
    let mut through_lambda = vec![false; nodes];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for &via in env.vias.keys() {
        if via < graph.functions
            && let Some(slot) = by_node.get_mut(via)
            && slot.is_none()
        {
            *slot = Some(via);
            queue.push_back(via);
        }
    }
    while let Some(node) = queue.pop_front() {
        let first = by_node.get(node).copied().flatten();
        let late = through_lambda.get(node).copied().unwrap_or(false) || node == lambda;
        let own = graph.succ.get(node).map_or(&[][..], Vec::as_slice);
        let lambdas = written.get(node).map_or(&[][..], Vec::as_slice);
        let open = (open_lambda && (opens_lambda(node) || node == lambda)).then_some(lambda);
        for &to in own.iter().chain(lambdas).chain(open.iter()) {
            // `Λ` only from code that runs code it does not show, or from `Λ` itself.
            if to == lambda && open.is_none() {
                continue;
            }
            if let Some(slot) = by_node.get_mut(to)
                && slot.is_none()
            {
                *slot = first;
                if let Some(flag) = through_lambda.get_mut(to) {
                    *flag = late || to == lambda;
                }
                queue.push_back(to);
            }
        }
    }
    ViaReach {
        by_node,
        through_lambda,
        functions: graph.functions,
        handlers: graph.handlers,
        lambda,
    }
}

/// Whether a function value made outside the code that the `via` functions reach (`reached`,
/// by [`search`] without `Λ`) can reach it (#1710), so that a call of a function value there may
/// run any lambda or function named as a value:
/// - a `via` function with a parameter whose type can hold a function value, the policy's
///   `from` value;
/// - an operation whose answer can hold one;
/// - a handler reached, whose parameters can hold one or whose operations call a function
///   value, since its state and parameters come from code outside;
/// - a function whose code the findings do not hold (a standard module, a dependency not
///   checked with the package) whose result or parameters can hold one.
///
/// Otherwise every function value that code calls is a lambda written in a node it reaches or
/// a function it names, whose code [`search`] follows from that node. O(V + O log O) for the O
/// operations of the graph (`tests/via_reach_scaling.rs`).
fn foreign_values(
    env: &Env<'_>,
    graph: &Graph,
    opens_lambda: &dyn Fn(usize) -> bool,
    reached: &[Option<usize>],
) -> bool {
    // The prelude's functions are host code, which calls only the function values it is given.
    // Only the module named exactly `prelude`: `prelude.map` and `std.collections` are
    // modules apart, whose functions count here when their types can hold a function value,
    // as their type parameters can.
    let apart = |module: usize| {
        let prelude = env.modules.get(module).is_some_and(|m| m.path == "prelude");
        !prelude && (module >= env.prelude || !env.reports(module))
    };
    // Whether each operation's answer can hold a function value, by its node, built once: each
    // capability's operations by name, then one lookup per operation node, O(O log O). A node
    // whose operation is not found fails closed.
    let mut answers = vec![true; graph.succ.len()];
    let mut by_name: BTreeMap<CapId, BTreeMap<&str, &super::env::OpDef<'_>>> = BTreeMap::new();
    for ((cap, op), &node) in &graph.ops {
        una_core::label_counters::timing_step();
        let ops = by_name.entry(*cap).or_insert_with(|| {
            env.capabilities
                .get(*cap)
                .map(|c| c.ops.iter().map(|d| (d.name, d)).collect())
                .unwrap_or_default()
        });
        if let Some(slot) = answers.get_mut(node) {
            *slot = ops
                .get(op.as_str())
                .is_none_or(|d| holds_function(env, &d.result.base));
        }
    }
    for (node, by) in reached.iter().enumerate() {
        una_core::label_counters::timing_step();
        if by.is_none() {
            continue;
        }
        if node < graph.functions {
            let Some(def) = env.functions.get(node) else {
                return true;
            };
            let params = def.params.iter().any(|p| holds_function(env, &p.ty.base));
            if env.vias.contains_key(&node) && params {
                return true;
            }
            if apart(def.module) && (params || holds_function(env, &def.result.base)) {
                return true;
            }
        } else if node < graph.functions + graph.handlers {
            let h = node - graph.functions;
            let Some(handler) = env.handlers.get(h) else {
                return true;
            };
            if opens_lambda(node)
                || handler
                    .params
                    .iter()
                    .any(|p| holds_function(env, &p.ty.base))
            {
                return true;
            }
        } else if node < graph.functions + graph.handlers + graph.capabilities {
            // A capability's node leads to its handlers, each checked as a node here.
        } else if node != graph.lambda && answers.get(node).copied().unwrap_or(true) {
            return true;
        }
    }
    false
}

/// Whether the preview's `τ_conc` of a site is `⊤` (ADR 0116 IF6): it lies inside a lambda, a
/// task or a handler's operation, or in a function that a part of a task construct can reach
/// (`concurrent`, [`concurrent_functions`]); `public` otherwise.
pub(crate) fn preview_concurrent(
    seed: &TimingSeed,
    function: Option<usize>,
    concurrent: &BTreeSet<usize>,
) -> bool {
    seed.unplaced || function.is_some_and(|f| concurrent.contains(&f))
}

/// The functions outside what the package's findings hold precisely that its code can reach,
/// each `⊤` for `L_res`, with the message that names it (#1148, failing closed until #1166):
/// - a function of a standard module whose body holds a `tasks` statement or a `first of`,
///   since the standard modules are checked apart and their constructs are not in the findings
///   (none does today, which `tests/std_constructs.rs` keeps);
/// - a function of a dependency whose code the checker does not see: a module not checked with
///   the package, or a function without a body. A path dependency's modules are checked with the
///   package, so its constructs count precisely, like the root's.
///
/// Linear: one search of the graph from the package's own nodes, O(V + E), and one scan of the
/// body of each reachable standard function, O(the size of those bodies).
fn outside_constructs(env: &Env<'_>, graph: &Graph) -> Vec<(Label, String)> {
    let top = Label::top(&env.categories);
    let dependency = |module: usize| {
        module < env.prelude && env.modules.get(module).is_some_and(|m| m.mount.is_some())
    };
    let own = |module: usize| module < env.prelude && !dependency(module);
    let roots = env
        .functions
        .iter()
        .enumerate()
        .filter(|(_, def)| own(def.module))
        .map(|(f, _)| f)
        .chain(
            env.handlers
                .iter()
                .enumerate()
                .filter(|(_, handler)| own(handler.module))
                .map(|(h, _)| graph.handler_node(h)),
        );
    let seen = reached_from(&graph.succ, roots);
    let mut out = Vec::new();
    for (f, def) in env.functions.iter().enumerate() {
        if own(def.module) || !seen.get(f).copied().unwrap_or(false) {
            continue;
        }
        let standard = def.module >= env.prelude;
        let holds = match def.ast {
            // A dependency's bodies are only scanned when unchecked; a standard one always.
            super::env::FnAst::Function(decl) if standard || !env.reports(def.module) => {
                let text =
                    |ident: &una_syntax::ast::Ident| env.text(def.module, ident.name).to_owned();
                Some(super::effects::holds_task_construct(&decl.body, &text))
            }
            super::env::FnAst::Function(_) => Some(false),
            // A predicate or a policy of a checked module has its body in the findings too.
            _ if standard || env.reports(def.module) => Some(false),
            _ => None,
        };
        let Some(why) = unseen(
            standard,
            dependency(def.module),
            env.reports(def.module),
            holds,
        ) else {
            continue;
        };
        let name = match env.modules.get(def.module).map(|m| m.path.as_str()) {
            Some(path) if !path.is_empty() => format!("`{path}.{}`", def.name),
            _ => format!("`{}`", def.name),
        };
        let source = match why {
            Unseen::StandardTasks => {
                format!("the tasks that {name}, outside this package, may leave running")
            }
            Unseen::UncheckedDependency => format!(
                "the tasks that {name}, outside this package, may leave running, since its code is not checked with the package"
            ),
        };
        out.push((top.clone(), source));
    }
    out
}

/// `L_res` of the package (ADR 0116 P9), with its graph built and the precise `τ_conc` of its
/// constructs computed here (`spec/27 §11`): O(V + E log E + F + size of the seeds). `L_srv`
/// keeps the preview's `τ_conc` (`concurrent`) until slice 6 of the plan on #1087.
pub(crate) fn residue_of(env: &Env<'_>, findings: &Findings) -> Residue {
    let graph = graph(env, findings);
    let concurrent = concurrent_functions(findings);
    let concurrency = concurrency(env, findings, &graph);
    let residue = residue(env, findings, &graph, &concurrency);
    server_label(env, findings, &graph, &concurrent, residue)
}

/// `L_res` with `L_srv` joined in (ADR 0123, S4 amendment IF-S1, `spec/31 §1`): in a package
/// whose code performs an operation of `HttpServer`, the serve threads' work after each
/// hand-off site can shift every timing site, so
///
/// ```text
/// L_srv = internal ⊔ ⨆ over hand-off sites s of ( sfx(s) ⊔ Suf*(node(s)) ⊔ τ_conc(s) ) ⊔ L_res
/// ```
///
/// joins `τ` of every timing site, as `L_res` does. The hand-off sites are the operations of
/// `HttpServer` and the `start`s that move reply rights (IF6).
///
/// **The preview's terms.** `sfx(s) ⊔ Suf*(node(s))`, the code that can run after `s`, is
/// over-approximated by `τ⁰` of the bodies of every ancestor of the function that holds `s`, that
/// function included, the rule of `L_lim` ([`ancestor_labels`]); `τ_conc` is the preview's,
/// `⊤` for a site in a task, a `tasks` block, a lambda or a handler's operation. Both are sound,
/// and coarser than the amendment's; they MAY become more precise and MUST NOT become stricter.
/// `L_srv` is `public` in a package without an operation of `HttpServer`, which starts no serve
/// thread.
///
/// **Linear.** One pass of [`ancestor_labels`] over the graph, O(V + E), and O(sites).
pub(crate) fn server_label(
    env: &Env<'_>,
    findings: &Findings,
    graph: &Graph,
    concurrent: &BTreeSet<usize>,
    mut residue: Residue,
) -> Residue {
    if findings.server_sites.is_empty() && env.serve_driver.is_none() {
        return residue;
    }
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let mut sites: Vec<HttpSite> = findings.server_sites.clone();
    // ADR 0123 rule 16 and IF-S1: the driver of a `serve` section is a hand-off site, its
    // `next` inside the `tasks` statement whose started tasks run `handle`, so `τ_conc` at it
    // is the preview's `⊤`, as for the language form's `next` (rule 15).
    if let Some(span) = env.serve_driver {
        sites.push(HttpSite {
            span,
            function: None,
            unplaced: true,
            node: None,
            construct: None,
        });
    }
    sites.extend(
        findings
            .server_starts
            .iter()
            .filter(|site| super::server::moves_rights(env, site))
            .map(|site| HttpSite {
                span: site.keyword,
                function: site.function,
                unplaced: site.unplaced,
                node: None,
                construct: None,
            }),
    );
    sites.sort_by_key(|site| (site.span.file, site.span.start, site.span.end));
    let labels = ancestor_labels(
        env,
        graph,
        &preview_beside(env, concurrent),
        &residue,
        &sites,
    );
    let label = internal.join(&labels.label);
    // The site a message names: the first whose label is the highest, else the first.
    let named = labels
        .sites
        .iter()
        .zip(&sites)
        .find(|(limit, _)| label.flows_to(&limit.label))
        .or_else(|| labels.sites.iter().zip(&sites).next())
        .map(|(_, site)| site);
    let driver = named.is_some_and(|site| Some(site.span) == env.serve_driver);
    let place = match named
        .and_then(|site| site.function)
        .and_then(|id| env.functions.get(id))
    {
        Some(def) => match env.modules.get(def.module).map(|m| m.path.as_str()) {
            Some(path) if !path.is_empty() => format!("`{path}.{}`", def.name),
            _ => format!("`{}`", def.name),
        },
        None if driver => "the driver of the `serve` section".to_owned(),
        None => "a handler's operation".to_owned(),
    };
    residue.label = residue.label.join(&label);
    residue.sources.push((
        label,
        format!(
            "the requests that the serve threads handle after the server operations in {place}"
        ),
    ));
    residue
}

/// `L_lim` of the package and the label of each request (ADR 0117 IF4, as ADR 0116 P8 states
/// it, slice T1): the label of a request is the join of `τ⁰` of the body of every node of the
/// call graph from which the function that makes it can be reached, that function included, of
/// `τ_conc` at it, the precise one of `spec/27 §11` for a request in a function's body and `⊤`
/// for one in a lambda's body or a handler's operation (#1154), and of `L_res`
/// (ADR 0116 P9, P10), the work that cancelled requests can leave on the helpers. The code that
/// runs between two requests of one task lies in the body of an ancestor of the later one, or in
/// what such a body calls, so the ancestors' `τ⁰` covers it.
///
/// **What `τ_conc` adds.** At a request in a function's body, `τ_conc` never exceeds the
/// ancestors' `τ⁰`: every construct whose parts can run beside the request is in the body of an
/// ancestor of the request's function, either because the request is in one of its parts or
/// because `conc` of the function comes from an edge out of a part, whose node reaches the
/// function; and that ancestor's `τ⁰` joins `down`, which covers the parts' work, the values
/// they compute (#1685) and `⊤` for their calls of function values included. So the precise
/// rule's only effect is to drop the preview's `⊤`; the ancestors' term refuses every request
/// it refused. The `⊤` of a request elsewhere is a fail-closed default that no program reaches
/// today: a lambda cannot perform an operation, and a request in a handler's operation has no
/// function, so its ancestors' term is `⊤` already.
///
/// **The graph** is [`graph`]'s, that of ADR 0116 P1. So the operations of a handler, which a
/// function reaches through an operation of the handler's capability, are ancestors of what they
/// call, and so are the functions that call a value, of what a lambda calls.
///
/// **`τ⁰` of a body** is its parameters' labels joined with `down` of its node: the labels of
/// everything it reaches, by the one rule of [`timing`]: the operations' `ρ`, the handlers'
/// parameters, a call of a function value, and a prelude function with a function parameter
/// (`⊤`). `Λ`'s body is `⊤` (IF5 item 4); an operation's is its `ρ`, a capability's `public`.
///
/// **Linear.** `down` is one pass over the strongly connected components of the graph, the join
/// over the ancestors one pass over those of the reversed graph, and the nearest ancestor above
/// `internal`, for the message, one breadth-first search from all such nodes at once:
/// O(F + H + C + O + E) label joins and steps, for `F` functions, `H` handlers, `C`
/// capabilities, `O` operations and `E` edges, with no search per request.
pub(crate) fn limit_labels(
    env: &Env<'_>,
    findings: &Findings,
    graph: &Graph,
    concurrency: &Concurrency,
    residue: &Residue,
) -> LimitLabels {
    site_labels(env, graph, concurrency, residue, &findings.http_sites)
}

/// The rule of [`limit_labels`] over `sites`: the join of `τ⁰` of the bodies of every ancestor of
/// each site, of its precise `τ_conc` in a function's body (`⊤` elsewhere), and of `residue`.
/// Shared by `L_lim` and by `L_clk`, the label of a reading of `Clock` (ADR 0116 amendment of
/// 2026-11-09, #898, C1), whose code that ran before it lies in the same ancestors.
pub(crate) fn site_labels(
    env: &Env<'_>,
    graph: &Graph,
    concurrency: &Concurrency,
    residue: &Residue,
    sites: &[HttpSite],
) -> LimitLabels {
    let beside = |site: &HttpSite| beside_label(env, concurrency, site);
    ancestor_labels(env, graph, &beside, residue, sites)
}

/// The precise `τ_conc` of a request (ADR 0116 amendment of 2026-10-25, C, `spec/27 §11`, issue
/// #1154), at most the ancestors' `τ⁰` there (see [`limit_labels`]); `None` when nothing runs
/// beside it. Any other site keeps `⊤`, fail-closed: none reaches it today, and the rule of P10
/// counts a handler's request as `⊤`.
pub(crate) fn beside_label(
    env: &Env<'_>,
    concurrency: &Concurrency,
    site: &HttpSite,
) -> Option<Label> {
    match site.node {
        Some(Caller::Function(_)) => concurrency.tau_conc(&TimingSeed {
            node: site.node,
            construct: site.construct,
            ..TimingSeed::default()
        }),
        _ => Some(Label::top(&env.categories)),
    }
}

/// The preview's `τ_conc` of a site of `L_srv` (ADR 0123, S4 amendment IF-S1), which keeps it
/// until its own card: `⊤` inside a lambda, a task, a `tasks` block or a handler's operation, or
/// in a function that a part of a task construct can reach; nothing beside it otherwise.
pub(crate) fn preview_beside<'a>(
    env: &Env<'_>,
    concurrent: &'a BTreeSet<usize>,
) -> impl Fn(&HttpSite) -> Option<Label> + 'a {
    let top = Label::top(&env.categories);
    move |site: &HttpSite| {
        (site.unplaced
            || site.function.is_none()
            || site.function.is_some_and(|f| concurrent.contains(&f)))
        .then(|| top.clone())
    }
}

/// [`limit_labels`] over `sites`: the label of each site, the join of `τ⁰` of the bodies of
/// every ancestor of its function, its function included, of `beside`, its `τ_conc` (`None` when
/// nothing runs beside it), and of `residue`; and their join. The one rule for `L_lim` (ADR 0117
/// IF4, the precise `τ_conc`) and for `L_srv` (ADR 0123, S4 amendment IF-S1, the preview's,
/// [`preview_beside`]), whose sites are the server's hand-off sites.
pub(crate) fn ancestor_labels(
    env: &Env<'_>,
    graph: &Graph,
    beside: &dyn Fn(&HttpSite) -> Option<Label>,
    residue: &Residue,
    at: &[HttpSite],
) -> LimitLabels {
    fn add(succ: &mut [Vec<usize>], from: usize, to: usize) {
        if let Some(edges) = succ.get_mut(from) {
            edges.push(to);
        }
    }
    if at.is_empty() {
        return LimitLabels {
            label: Label::public(),
            sites: Vec::new(),
        };
    }
    let kinds = env.kind_labels.as_ref();
    let top = Label::top(&env.categories);
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let functions = graph.functions;
    let handlers = graph.handlers;
    let capabilities = graph.capabilities;
    let lambda = graph.lambda;
    let nodes = lambda + 1;
    let succ = &graph.succ;
    let local = &graph.local;
    let down = &graph.down;
    let ops = &graph.ops;
    // `τ⁰` of each node's body: a function's parameters and what it reaches; a handler's
    // parameters, in its `local`, and what it reaches; an operation's `ρ`; `Λ`'s `⊤`.
    let body: Vec<Label> = (0..nodes)
        .map(|node| {
            if node < functions {
                let params = env.functions.get(node).map_or_else(Label::public, |def| {
                    def.params.iter().fold(Label::public(), |label, param| {
                        label.join(&param.ty.base.deep_label(kinds))
                    })
                });
                down.get(node).map_or(params.clone(), |d| params.join(d))
            } else if node < functions + handlers {
                down.get(node).cloned().unwrap_or_else(|| top.clone())
            } else {
                local.get(node).cloned().unwrap_or_else(|| top.clone())
            }
        })
        .collect();
    // The ancestors: the reversed graph, closed once.
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for (from, tos) in succ.iter().enumerate() {
        for &to in tos {
            add(&mut pred, to, from);
        }
    }
    let up = close(&pred, &body, |a: &Label, b: &Label| a.join(b));
    // The nearest ancestor above `internal` of each node: a breadth-first search along the
    // graph from every such node at once, the first to reach a node naming it.
    let mut nearest: Vec<Option<usize>> = vec![None; nodes];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (node, label) in body.iter().enumerate() {
        if !label.flows_to(&internal)
            && let Some(slot) = nearest.get_mut(node)
        {
            *slot = Some(node);
            queue.push_back(node);
        }
    }
    while let Some(node) = queue.pop_front() {
        let witness = nearest.get(node).copied().flatten();
        for &to in succ.get(node).map_or(&[][..], Vec::as_slice) {
            una_core::label_counters::pass_step();
            if let Some(slot) = nearest.get_mut(to)
                && slot.is_none()
            {
                *slot = witness;
                queue.push_back(to);
            }
        }
    }
    // The operation of each operation node, to name it.
    let mut op_of: Vec<Option<(CapId, &str)>> = vec![None; ops.len()];
    for ((cap, op), &node) in ops {
        if let Some(slot) = op_of.get_mut(node - functions - handlers - capabilities) {
            *slot = Some((*cap, op.as_str()));
        }
    }
    let ops_start = functions + handlers + capabilities;
    let name = |node: usize| -> String {
        let qualified =
            |module: usize, name: &str| match env.modules.get(module).map(|m| m.path.as_str()) {
                Some(path) if !path.is_empty() => format!("`{path}.{name}`"),
                _ => format!("`{name}`"),
            };
        match node {
            f if f < functions => env
                .functions
                .get(f)
                .map_or_else(String::new, |def| qualified(def.module, def.name)),
            h if h < functions + handlers => {
                env.handlers
                    .get(h - functions)
                    .map_or_else(String::new, |h| {
                        format!(
                            "the handler {}",
                            qualified(h.module, env.text(h.module, h.ast.name.name))
                        )
                    })
            }
            c if c < ops_start => env
                .capabilities
                .get(c - functions - handlers)
                .map_or_else(String::new, |c| format!("the capability `{}`", c.name)),
            o if o < lambda => op_of
                .get(o - ops_start)
                .copied()
                .flatten()
                .and_then(|(cap, op)| {
                    env.capabilities
                        .get(cap)
                        .map(|c| format!("the operation `{}.{op}`", c.name))
                })
                .unwrap_or_default(),
            _ => "functions called as values".to_owned(),
        }
    };
    // What `L_res` raises a request to, and the construct a message names (ADR 0116 P9, P10).
    let residue_raiser = residue
        .culprit(&internal)
        .map(|(label, source)| Raiser::Residue(source.clone(), label.clone()));
    let mut sites = Vec::new();
    let mut label = Label::public();
    for site in at {
        una_core::label_counters::pass_step();
        let beside = beside(site);
        let (own, raiser) = match site.function {
            Some(f) => {
                let own = up.get(f).cloned().unwrap_or_else(|| top.clone());
                let raiser = if own.flows_to(&internal) {
                    None
                } else {
                    nearest.get(f).copied().flatten().map(|node| {
                        Raiser::Ancestor(
                            name(node),
                            body.get(node).cloned().unwrap_or_else(|| top.clone()),
                            node == f,
                        )
                    })
                };
                (own, raiser)
            }
            None => (top.clone(), None),
        };
        let (site_label, raiser) = match beside {
            Some(beside) if !beside.flows_to(&internal) => {
                (own.join(&beside), raiser.or(Some(Raiser::Beside)))
            }
            Some(beside) => (own.join(&beside), raiser),
            None => (own, raiser),
        };
        let site_label = site_label.join(&residue.label);
        let raiser = raiser.or_else(|| residue_raiser.clone());
        label = label.join(&site_label);
        sites.push(LimitSite {
            span: site.span,
            label: site_label,
            raiser,
        });
    }
    LimitLabels {
        label: label.join(&residue.label),
        sites,
    }
}

/// The callees of each function, from the package's call edges, with the `via` function of
/// each policy called, which the policy's call runs (#1673).
fn callees_of(findings: &Findings) -> BTreeMap<usize, Vec<usize>> {
    let mut callees: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(caller, callee, _) in &findings.edges {
        callees.entry(caller).or_default().push(callee);
    }
    for &(policy, via) in &findings.policy_vias {
        callees.entry(policy).or_default().push(via);
    }
    callees
}

/// The functions reachable from `roots` along `callees`, the roots included.
fn reach(
    callees: &BTreeMap<usize, Vec<usize>>,
    roots: impl IntoIterator<Item = usize>,
) -> BTreeSet<usize> {
    let mut reached = BTreeSet::new();
    let mut pending: Vec<usize> = roots.into_iter().collect();
    while let Some(f) = pending.pop() {
        if reached.insert(f) {
            pending.extend(callees.get(&f).into_iter().flatten().copied());
        }
    }
    reached
}

/// The functions that can run while a part of a task construct runs (ADR 0116 IF6), so beside
/// a site of another part, or of a function such a part reaches: what the parts call, and,
/// when the package has a task construct at all, every function that a function value, a
/// lambda or a handler's operation can run, since a value or a handler made outside a
/// construct may run inside it. Computed once per package.
pub(crate) fn concurrent_functions(findings: &Findings) -> BTreeSet<usize> {
    if findings.task_functions.is_empty() {
        return BTreeSet::new();
    }
    let callees = callees_of(findings);
    let roots = findings
        .task_edges
        .iter()
        .chain(&findings.lambda_edges)
        .chain(&findings.handler_edges)
        .chain(&findings.function_values)
        .copied();
    reach(&callees, roots)
}

/// Whether a value of `ty` may hold a function value: a function type, or one inside a list, a
/// map, a collection, an `Option`, a `Result` or a kind's fields, at any depth (#1436). A type
/// parameter, a unification variable and a kind the table does not know count as holding one,
/// failing closed. The one rule for the timing label, the known arguments of #1023 and the
/// function values that cross a boundary (ADR 0093 rule 8).
pub(crate) fn holds_function(env: &Env<'_>, ty: &Ty) -> bool {
    holds_function_by(env, ty, &|ty: &Ty| ty.clone())
}

/// [`holds_function`] with `resolve` applied to each type first, as a checker resolves the
/// unification variables it has solved (review R2 of #1440).
pub(crate) fn holds_function_by(env: &Env<'_>, ty: &Ty, resolve: &dyn Fn(&Ty) -> Ty) -> bool {
    match resolve(ty) {
        Ty::Fn(_) | Ty::Param(_) | Ty::Var(_) => true,
        Ty::List(inner)
        | Ty::Dense(_, inner)
        | Ty::Labeled(_, inner)
        | Ty::Refined(_, inner)
        | Ty::Measured(_, inner) => holds_function_by(env, &inner, resolve),
        Ty::Map(key, value) => {
            holds_function_by(env, &key, resolve) || holds_function_by(env, &value, resolve)
        }
        Ty::Coll(_, args) => args.iter().any(|arg| holds_function_by(env, arg, resolve)),
        Ty::Kind(id, args) => {
            env.holds_function_kinds.get(id).copied().unwrap_or(true)
                || args.iter().any(|arg| holds_function_by(env, arg, resolve))
        }
        _ => false,
    }
}

/// The label a handler's parameter adds to a timing label (ADR 0116 IF5 item 3): `⊤` when its
/// type may hold a function, whose running time its type does not bound, as for a parameter
/// of function type, and its deep label otherwise (#1436).
fn handler_param_label(env: &Env<'_>, ty: &Ty) -> Label {
    if holds_function(env, ty) {
        Label::top(&env.categories)
    } else {
        ty.deep_label(env.kind_labels.as_ref())
    }
}

/// What [`timing`] has gathered of a site: the functions it reaches, the prelude functions
/// applied to arguments whose code is not known at the call, the handlers and operations still
/// to take, and the sources so far, in the order a message names them.
struct Gather<'f> {
    /// The callees of each function, from the package's call edges.
    callees: &'f BTreeMap<usize, Vec<usize>>,
    /// The functions reached so far.
    reached: BTreeSet<usize>,
    /// The prelude functions applied, in the site or in what it reaches, to an argument whose
    /// code is not known at the call (#1023).
    opaque: BTreeSet<usize>,
    /// The functions of `opaque` already counted as a source.
    flagged: BTreeSet<usize>,
    /// The functions added to `opaque` since [`Gather::flag_opaque`] last ran.
    late: Vec<usize>,
    /// The handlers that can serve the operations reached.
    handlers: BTreeSet<usize>,
    /// The handlers of `handlers` not taken yet.
    queue: Vec<usize>,
    /// The operations still to take.
    needs: Vec<(CapId, String)>,
    /// The sources so far.
    sources: Vec<(Label, String)>,
    /// The values that the reached functions and handlers compute in their own code, each with
    /// the source a message names, joined after every other source so that a message names
    /// what a site reads or performs first (#1685).
    values: Vec<(Label, String)>,
}

impl Gather<'_> {
    /// Adds the handler `h`, to be taken once.
    fn add_handler(&mut self, h: usize) {
        if self.handlers.insert(h) {
            self.queue.push(h);
        }
    }

    /// Adds the prelude function `f` to those applied to an argument whose code is not known.
    fn add_opaque(&mut self, f: usize) {
        if self.opaque.insert(f) {
            self.late.push(f);
        }
    }

    /// Reaches the functions `roots` and what they call, and takes each function newly reached,
    /// in ascending order: a call of a function value in it, an application of it to an
    /// argument whose code is not known, the handlers it installs and the operations its `uses`
    /// names. Each function is taken once, and its installs and opaque applications are looked
    /// up by a range of the sets ordered by caller, so all the calls over one site take
    /// O((F + E + I + A) log n) in all, for `F` functions reached, `E` their call edges, `I`
    /// their installs and `A` their opaque applications (review R1 of #1440).
    fn visit(
        &mut self,
        env: &Env<'_>,
        findings: &Findings,
        roots: impl IntoIterator<Item = usize>,
    ) {
        let top = Label::top(&env.categories);
        let mut fresh: BTreeSet<usize> = BTreeSet::new();
        let mut pending: Vec<usize> = roots.into_iter().collect();
        while let Some(f) = pending.pop() {
            una_core::label_counters::timing_step();
            if !self.reached.contains(&f) && fresh.insert(f) {
                pending.extend(self.callees.get(&f).into_iter().flatten().copied());
            }
        }
        self.reached.extend(fresh.iter().copied());
        for &f in &fresh {
            let own: Vec<usize> = findings
                .opaque_applies
                .range((f, 0)..=(f, usize::MAX))
                .map(|&(_, callee)| callee)
                .collect();
            for callee in own {
                una_core::label_counters::timing_step();
                self.add_opaque(callee);
            }
            let installs: Vec<usize> = findings
                .installs
                .range((f, 0)..=(f, usize::MAX))
                .map(|&(_, h)| h)
                .collect();
            for h in installs {
                una_core::label_counters::timing_step();
                self.add_handler(h);
            }
        }
        for &f in &fresh {
            let Some(def) = env.functions.get(f) else {
                continue;
            };
            if findings.value_callers.contains(&f) {
                self.sources.push((
                    top.clone(),
                    format!("a call of a function value in `{}`", def.name),
                ));
            }
            // What its body computes, as `let k: secret Int = 3000000` (#1685).
            if let Some(Some(body)) = findings.body_labels.get(&f) {
                self.values
                    .push((body.clone(), format!("a value computed in `{}`", def.name)));
            }
            // A prelude function that applies a function argument, such as `map`, when the
            // site or a function it reaches calls it with an argument whose code is not known
            // at the call, or names it as a value (#1023). Called only with lambdas written at
            // the call, it runs nothing the seed and the reached functions do not already
            // count: the lambdas' captures are the site's free variables (IF5 item 2) or locals
            // of a reached function, and what their bodies call and perform is recorded where
            // they are written.
            if applies_function(env, def) && self.opaque.contains(&f) && self.flagged.insert(f) {
                self.sources.push((
                    top.clone(),
                    format!("the function argument of `{}`", def.name),
                ));
            }
            for need in &def.uses {
                if let Some(cap) = need.usage.capability {
                    self.needs
                        .extend(need.usage.ops.iter().map(|op| (cap, op.clone())));
                }
            }
        }
        self.flag_opaque(env);
    }

    /// Counts each prelude function reached earlier that a later part of the site applies to an
    /// argument whose code is not known, as a handler's operation body may (#1436). Only the
    /// functions added since the last call are looked at: one not reached yet is counted when it
    /// is reached, by [`Gather::visit`].
    fn flag_opaque(&mut self, env: &Env<'_>) {
        let top = Label::top(&env.categories);
        let mut late = std::mem::take(&mut self.late);
        late.sort_unstable();
        late.dedup();
        late.retain(|f| self.reached.contains(f) && !self.flagged.contains(f));
        for f in late {
            let Some(def) = env.functions.get(f) else {
                continue;
            };
            if applies_function(env, def) && self.flagged.insert(f) {
                self.sources.push((
                    top.clone(),
                    format!("the function argument of `{}`", def.name),
                ));
            }
        }
    }
}

/// `τ(e)` of a site whose checking recorded `seed`, in the function `function` (ADR 0116 IF5,
/// IF6), with the handlers of host capabilities that `host` counts. `concurrent` is
/// [`concurrent_functions`] of the package.
///
/// The preview over-approximations of `spec/26 §2` rule 5 apply, and may only get less strict:
/// a function value called anywhere the site reaches makes it `⊤`, and so does a call of a
/// prelude function with a function parameter there, unless every argument of that call that may
/// hold a function is a lambda written at the call for a parameter of function type, whose
/// captures and body the seed and the
/// reached functions already count (#1023); so does concurrent work, where `τ_conc` is `⊤`
/// rather than the join of the siblings' `τ⁰`.
///
/// **Handlers** (ADR 0116 IF5 item 3, #1436). Each handler that can serve an operation the site
/// reaches counts by its parameters, `⊤` for one whose type may hold a function
/// ([`holds_function`]), and by what its operation bodies run, their lambdas and tasks
/// included, taken as the site's own code is: the functions they call, with what those reach,
/// the operations they perform, the handlers they install, a call of a function value (`⊤`)
/// and a prelude function applied to an argument whose code is not known (`⊤`).
///
/// **Values** (ADR 0116 IF5 item 7, amendment of 2026-11-06, `spec/26 §7` rule 5 item 7,
/// #1685). The values that the site's own code, each
/// function reached and each handler taken compute count by `ℓ*` of their types, symbolic labels
/// dropped: a value a callee labels from literals, as `let k: secret Int = 3000000`, raises `τ`
/// as the same value passed in does under IF5 item 2. They join after every other source, so a
/// message names what the site reads or performs first. One label per function, recorded while
/// its body is checked, so this adds O(F + H) joins to a site.
///
/// **Complexity** (review R1 of #1440): each function, handler and operation is taken once per
/// site, and each lookup is in an ordered map or set, so a site costs
/// O((F + E + H + O + S) log n), for `F` functions, `E` call edges, `H` handlers, `O`
/// operations and `S` the size of the seeds of the handlers it reaches; the call edges are
/// indexed once per site. `tests/handler_timing_scaling.rs` keeps it linear in a chain of
/// handlers.
pub(crate) fn timing(
    env: &Env<'_>,
    findings: &Findings,
    seed: &TimingSeed,
    function: Option<usize>,
    host: HostHandlers,
    concurrent: &BTreeSet<usize>,
) -> Timing {
    // The preview's `τ_conc` (IF6): `⊤` when a part of a task construct can run beside the site.
    let beside =
        preview_concurrent(seed, function, concurrent).then(|| Label::top(&env.categories));
    timing_with(env, findings, seed, host, beside)
}

/// [`timing`] with the precise `τ_conc` of the amendment of 2026-10-25 (`spec/27 §11`):
/// `conc` of the site's node joined with `ctx` at it, `⊤` for a site in no node (P4).
pub(crate) fn timing_precise(
    env: &Env<'_>,
    findings: &Findings,
    seed: &TimingSeed,
    host: HostHandlers,
    concurrency: &Concurrency,
) -> Timing {
    timing_with(env, findings, seed, host, concurrency.tau_conc(seed))
}

/// `τ⁰` of every function, handler and operation, closed once per package over the graph that
/// [`timing_with`] searches for each site (ADR 0116 amendment of 2026-10-25, E: summaries
/// instead of a search per site; slice 7 of the plan on #1087), with the handlers of every
/// capability counted ([`HostHandlers::All`]).
///
/// **The graph.** Its nodes are the functions, the handlers and the operations named anywhere.
/// A function goes to what its body calls or names, its lambdas' included (the package's call
/// edges, and a policy to its `via`), to the handlers it installs and to the operations of its
/// `uses`; a handler goes to the operations of its `uses` and to what its operation bodies call,
/// perform and install; an operation goes to every handler of its capability. A node's own
/// label is what [`timing_with`] takes when it reaches the node:
/// - a function: `⊤` when it calls a function value, `⊤` when it applies a prelude function such
///   as `map` to an argument whose code is not known (#1023), and the values its body computes
///   (IF5 item 7);
/// - a handler: its parameters (`⊤` for one that may hold a function), the values its operation
///   bodies compute, and `⊤` for a call of a function value or an opaque application there;
/// - an operation: its `ρ`.
///
/// So `τ⁰` of a site is its own sources, values and calls of function values, joined with the
/// summary of every node its seed names: the same join as the search, which reaches exactly the
/// nodes the summary closes over. The search stays, run only for a site whose label is
/// reported, to name its largest source (E, "the error path").
///
/// **Cost.** O(V + E) to build, for the `V` nodes and `E` edges above, then O(V + E) joins and
/// O((V + E) · h) for the `h` operations of `Http` in the package, one pass over the strongly
/// connected components each. A site then costs O(size of its seed) lookups, instead of a search
/// of the graph.
pub(crate) struct Summaries {
    /// The number of functions, and of handlers, which are the first nodes.
    functions: usize,
    handlers: usize,
    /// The node of each operation, after the handlers.
    ops: BTreeMap<(CapId, String), usize>,
    /// The handlers of each capability.
    of_capability: BTreeMap<CapId, Vec<usize>>,
    /// The summary of each node.
    down: Vec<Label>,
    /// The operations of the host's `Http` each node reaches, by their nodes.
    http: Vec<BTreeSet<usize>>,
    /// The operation of each operation node, after the handlers.
    op_of: Vec<(CapId, String)>,
}

impl Summaries {
    /// The node of the handler `h`.
    fn handler_node(&self, h: usize) -> usize {
        self.functions + h
    }
}

/// The most sites of one kind whose summary [`cross_check`] holds to the search, in a build with
/// debug assertions: every package of the conformance suite is far below it, and the scaling
/// tests, which must stay linear, are above it.
pub(crate) const CROSS_CHECKED: usize = 200;

/// In a build with debug assertions, and for a package with at most [`CROSS_CHECKED`] sites of
/// the kind, `fast` from the summaries is the search's `slow`: the same label, and the same
/// operations of `Http` (slice 7 of #1087). Release builds skip it.
pub(crate) fn cross_check(
    env: &Env<'_>,
    sites: usize,
    fast: &Timing,
    slow: impl FnOnce() -> Timing,
) {
    if !cfg!(debug_assertions) || sites > CROSS_CHECKED {
        return;
    }
    let slow = slow();
    let http: BTreeSet<(CapId, String)> = slow
        .ops
        .iter()
        .filter(|(cap, op)| {
            is_http(env, *cap)
                && env
                    .capabilities
                    .get(*cap)
                    .is_some_and(|c| c.ops.iter().any(|d| d.name == *op))
        })
        .cloned()
        .collect();
    // INVARIANT: the summaries close over exactly the nodes the search reaches.
    assert_eq!(
        fast.label, slow.label,
        "the summary and the search disagree on a label"
    );
    // INVARIANT: as above, for the operations of `Http` the callers read.
    assert_eq!(
        fast.ops, http,
        "the summary and the search disagree on the operations of `Http`"
    );
}

/// The timing label of every frame of a pass from the summaries (slice 8b of #1087), without
/// rebuilding any seed: a frame's own sources and the reads free in it
/// ([`TimingFrames::sources_up`]), joined with what the items of its subtree reach through
/// [`Summaries`] ([`TimingFrames::fold_up`]): the summary of each callee, handler installed and
/// operation (for an operation no header names, its `ρ` and its capability's handlers), `⊤` for
/// a call of a function value and for an opaque application of a prelude function such as
/// `map` (#1023), and the values of IF5 item 7. That is the join the search makes over the
/// frame's rebuilt seed, which [`cross_check`] holds it to. O(F + N) joins and O((R + F) log F)
/// for the reads.
pub(crate) struct FrameTimings {
    /// Each frame's label before `τ_conc`.
    label: Vec<Label>,
    /// The operations of `Http` each frame's subtree reaches.
    ops: Vec<BTreeSet<(CapId, String)>>,
    /// `⊤`, for a frame it does not hold, failing closed (P4, F item 6).
    top: Label,
}

impl FrameTimings {
    /// `τ` of the site of `frame` with `beside` as `τ_conc`: the label and the operations of
    /// `Http`, without the sources, which the search adds for a site that is reported. O(1)
    /// joins.
    pub(crate) fn timing(&self, frame: TimingFrame, beside: Option<Label>) -> Timing {
        let mut label = self
            .label
            .get(frame.0)
            .cloned()
            .unwrap_or_else(|| self.top.clone());
        if let Some(beside) = beside
            && !beside.is_public()
        {
            label = label.join(&beside);
        }
        Timing {
            label,
            sources: Vec::new(),
            ops: self.ops.get(frame.0).cloned().unwrap_or_default(),
        }
    }
}

impl Summaries {
    /// [`FrameTimings`] of `frames`.
    pub(crate) fn frames(&self, env: &Env<'_>, frames: &TimingFrames) -> FrameTimings {
        let top = Label::top(&env.categories);
        let start = self.functions + self.handlers;
        let item = |event: &Event| -> Option<(Label, BTreeSet<(CapId, String)>)> {
            let reached = |node: usize| {
                let label = self.down.get(node).cloned().unwrap_or_else(Label::public);
                let ops = self
                    .http
                    .get(node)
                    .into_iter()
                    .flatten()
                    .filter_map(|&op| self.op_of.get(op.checked_sub(start)?).cloned())
                    .collect();
                Some((label, ops))
            };
            match event {
                Event::Callee(f) if *f < self.functions => reached(*f),
                Event::Install(h) if *h < self.handlers => reached(self.handler_node(*h)),
                Event::Op(cap, op) => match self.ops.get(&(*cap, op.clone())) {
                    Some(&node) => reached(node),
                    None => {
                        let capability = env.capabilities.get(*cap)?;
                        let mut label = rho(env, *cap, op).unwrap_or_else(Label::public);
                        let mut ops = BTreeSet::new();
                        if is_http(env, *cap) && capability.ops.iter().any(|d| d.name == *op) {
                            ops.insert((*cap, op.clone()));
                        }
                        for &h in self.of_capability.get(cap).map_or(&[][..], Vec::as_slice) {
                            if let Some((down, more)) = reached(self.handler_node(h)) {
                                label = label.join(&down);
                                ops.extend(more);
                            }
                        }
                        Some((label, ops))
                    }
                },
                Event::ValueCall => Some((top.clone(), BTreeSet::new())),
                Event::Opaque(f) => env
                    .functions
                    .get(*f)
                    .is_some_and(|def| applies_function(env, def))
                    .then(|| (top.clone(), BTreeSet::new())),
                Event::Value(label) => Some((label.clone(), BTreeSet::new())),
                _ => None,
            }
        };
        let zero = (Label::public(), BTreeSet::new());
        let items = frames.fold_up(&zero, item, |a, b| {
            (a.0.join(&b.0), a.1.union(&b.1).cloned().collect())
        });
        let sources = frames.sources_up();
        let mut label = Vec::with_capacity(items.len());
        let mut ops = Vec::with_capacity(items.len());
        for (at, (items, reached)) in items.into_iter().enumerate() {
            label.push(
                sources
                    .get(at)
                    .map_or_else(|| items.clone(), |own| own.join(&items)),
            );
            ops.push(reached);
        }
        FrameTimings { label, ops, top }
    }
}

/// [`Summaries`] of the package: O(V + E) to build, and two passes over the strongly connected
/// components of its graph.
pub(crate) fn summaries(env: &Env<'_>, findings: &Findings) -> Summaries {
    fn add(succ: &mut [Vec<usize>], from: usize, to: usize) {
        if let Some(edges) = succ.get_mut(from) {
            edges.push(to);
        }
    }
    let top = Label::top(&env.categories);
    let functions = env.functions.len();
    let handlers = env.handlers.len();
    let mut of_capability: BTreeMap<CapId, Vec<usize>> = BTreeMap::new();
    for (id, handler) in env.handlers.iter().enumerate() {
        if let Some(cap) = handler.capability {
            of_capability.entry(cap).or_default().push(id);
        }
    }
    // Every operation a header or a handler's operation body names gets a node.
    let mut ops: BTreeMap<(CapId, String), usize> = BTreeMap::new();
    let mut op_of: Vec<(CapId, String)> = Vec::new();
    let mut name = |cap: CapId, op: &str, ops: &mut BTreeMap<(CapId, String), usize>| {
        let key = (cap, op.to_owned());
        if let Some(&node) = ops.get(&key) {
            return node;
        }
        let node = functions + handlers + op_of.len();
        op_of.push(key.clone());
        ops.insert(key, node);
        node
    };
    let mut uses: Vec<(usize, usize)> = Vec::new();
    let needs = env
        .functions
        .iter()
        .enumerate()
        .map(|(f, def)| (f, &def.uses))
        .chain(
            env.handlers
                .iter()
                .enumerate()
                .map(|(h, handler)| (functions + h, &handler.uses)),
        );
    for (from, needs) in needs {
        for need in needs {
            if let Some(cap) = need.usage.capability {
                for op in &need.usage.ops {
                    uses.push((from, name(cap, op, &mut ops)));
                }
            }
        }
    }
    for (&h, seed) in &findings.handler_seeds {
        if h >= handlers {
            continue;
        }
        for (cap, op) in &seed.ops {
            uses.push((functions + h, name(*cap, op, &mut ops)));
        }
    }
    let nodes = functions + handlers + op_of.len();
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    let mut local: Vec<Label> = vec![Label::public(); nodes];
    for (from, to) in uses {
        add(&mut succ, from, to);
    }
    for (caller, callees) in callees_of(findings) {
        for callee in callees {
            if callee < functions {
                add(&mut succ, caller, callee);
            }
        }
    }
    for &(f, h) in &findings.installs {
        if h < handlers {
            add(&mut succ, f, functions + h);
        }
    }
    let applies = |f: usize| {
        env.functions
            .get(f)
            .is_some_and(|def| applies_function(env, def))
    };
    for (f, slot) in local.iter_mut().enumerate().take(functions) {
        if findings.value_callers.contains(&f) {
            *slot = top.clone();
        }
        if let Some(Some(body)) = findings.body_labels.get(&f) {
            *slot = slot.join(body);
        }
    }
    for &(f, callee) in &findings.opaque_applies {
        if applies(callee)
            && let Some(slot) = local.get_mut(f)
        {
            *slot = top.clone();
        }
    }
    for (h, handler) in env.handlers.iter().enumerate() {
        let node = functions + h;
        let mut own = Label::public();
        for param in &handler.params {
            own = own.join(&handler_param_label(env, &param.ty.base));
        }
        if let Some(seed) = findings.handler_seeds.get(&h) {
            if let Some(values) = &seed.values {
                own = own.join(values);
            }
            if seed.value_call || seed.opaque_applies.iter().any(|&f| applies(f)) {
                own = own.join(&top);
            }
            for &h2 in &seed.handlers {
                if h2 < handlers {
                    add(&mut succ, node, functions + h2);
                }
            }
            for &f in seed.callees.iter().chain(&seed.opaque_applies) {
                if f < functions {
                    add(&mut succ, node, f);
                }
            }
        }
        if let Some(slot) = local.get_mut(node) {
            *slot = own;
        }
    }
    let mut http_local: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nodes];
    for (i, (cap, op)) in op_of.iter().enumerate() {
        let node = functions + handlers + i;
        let Some(capability) = env.capabilities.get(*cap) else {
            continue;
        };
        if let Some(rho) = rho(env, *cap, op)
            && let Some(slot) = local.get_mut(node)
        {
            *slot = rho;
        }
        if is_http(env, *cap)
            && capability.ops.iter().any(|d| d.name == *op)
            && let Some(slot) = http_local.get_mut(node)
        {
            slot.insert(node);
        }
        for &h in of_capability.get(cap).map_or(&[][..], Vec::as_slice) {
            add(&mut succ, node, functions + h);
        }
    }
    for edges in &mut succ {
        edges.sort_unstable();
        edges.dedup();
    }
    let down = close(&succ, &local, |a: &Label, b: &Label| a.join(b));
    let http = close(
        &succ,
        &http_local,
        |a: &BTreeSet<usize>, b: &BTreeSet<usize>| a.union(b).copied().collect(),
    );
    Summaries {
        functions,
        handlers,
        ops,
        of_capability,
        down,
        http,
        op_of,
    }
}

/// [`timing`] with `beside` as `τ_conc`, `None` when no part of a construct can run beside the
/// site.
pub(crate) fn timing_with(
    env: &Env<'_>,
    findings: &Findings,
    seed: &TimingSeed,
    host: HostHandlers,
    beside: Option<Label>,
) -> Timing {
    let kinds = env.kind_labels.as_ref();
    let top = Label::top(&env.categories);
    let callees = callees_of(findings);
    // The handlers of each capability, so that an operation finds its handlers without a scan
    // of every handler of the package.
    let mut of_capability: BTreeMap<CapId, Vec<usize>> = BTreeMap::new();
    for (id, handler) in env.handlers.iter().enumerate() {
        una_core::label_counters::timing_step();
        if let Some(cap) = handler.capability {
            of_capability.entry(cap).or_default().push(id);
        }
    }
    let mut gather = Gather {
        callees: &callees,
        reached: BTreeSet::new(),
        opaque: seed.opaque_applies.clone(),
        flagged: BTreeSet::new(),
        late: Vec::new(),
        handlers: seed.handlers.clone(),
        queue: seed.handlers.iter().copied().collect(),
        needs: seed.ops.iter().cloned().collect(),
        sources: seed.sources.clone(),
        values: Vec::new(),
    };
    // What the site's own code computes, its own bindings included (#1685).
    if let Some(values) = &seed.values {
        gather.values.push((
            values.clone(),
            "a value computed in its own code".to_owned(),
        ));
    }
    if seed.value_call {
        gather
            .sources
            .push((top.clone(), "a call of a function value".to_owned()));
    }
    gather.visit(env, findings, seed.callees.iter().copied());
    // The operations, and the handlers that can serve them, with the operations those
    // handlers need and what their operation bodies run, to a fixpoint over capabilities.
    let mut seen: BTreeSet<(CapId, String)> = BTreeSet::new();
    let mut handled: BTreeSet<usize> = BTreeSet::new();
    while !gather.needs.is_empty() || !gather.queue.is_empty() {
        while let Some((cap, op)) = gather.needs.pop() {
            una_core::label_counters::timing_step();
            if !seen.insert((cap, op.clone())) {
                continue;
            }
            let Some(capability) = env.capabilities.get(cap) else {
                continue;
            };
            if let Some(def) = capability.ops.iter().find(|d| d.name == op) {
                let mut rho = def.result.base.deep_label(kinds);
                if let Some(accepts) = &def.accepts {
                    rho = rho.join(accepts);
                }
                if let Some(fails) = &def.fails {
                    rho = rho.join(&fails.deep_label(kinds));
                }
                gather
                    .sources
                    .push((rho, format!("the operation `{}.{op}`", capability.name)));
            }
            if capability.module != env.prelude || host == HostHandlers::All {
                for &id in of_capability.get(&cap).map_or(&[][..], Vec::as_slice) {
                    una_core::label_counters::timing_step();
                    gather.add_handler(id);
                }
            }
        }
        // The handlers added since the last round, in ascending order.
        let mut next = std::mem::take(&mut gather.queue);
        next.sort_unstable();
        for id in next {
            una_core::label_counters::timing_step();
            if !handled.insert(id) {
                continue;
            }
            let Some(handler) = env.handlers.get(id) else {
                continue;
            };
            for param in &handler.params {
                gather.sources.push((
                    handler_param_label(env, &param.ty.base),
                    format!("the parameter `{}` of a handler", param.name),
                ));
            }
            for need in &handler.uses {
                if let Some(cap) = need.usage.capability {
                    gather
                        .needs
                        .extend(need.usage.ops.iter().map(|op| (cap, op.clone())));
                }
            }
            // What its operation bodies run, taken as the site's own code is (#1436).
            if let Some(body) = findings.handler_seeds.get(&id) {
                if let Some(values) = &body.values {
                    gather.values.push((
                        values.clone(),
                        format!(
                            "a value computed in the handler `{}`",
                            env.text(handler.module, handler.ast.name.name)
                        ),
                    ));
                }
                if body.value_call {
                    gather.sources.push((
                        top.clone(),
                        format!(
                            "a call of a function value in the handler `{}`",
                            env.text(handler.module, handler.ast.name.name)
                        ),
                    ));
                }
                for &h in &body.handlers {
                    gather.add_handler(h);
                }
                gather.needs.extend(body.ops.iter().cloned());
                for &f in &body.opaque_applies {
                    gather.add_opaque(f);
                }
                gather.visit(
                    env,
                    findings,
                    body.callees.iter().chain(&body.opaque_applies).copied(),
                );
            }
        }
    }
    let mut sources = gather.sources;
    sources.extend(gather.values);
    // `τ_conc` (IF6), last, so that a message names the site's own sources first.
    if let Some(beside) = beside
        && !beside.is_public()
    {
        sources.push((beside, "tasks that can run beside it".to_owned()));
    }
    let label = sources
        .iter()
        .fold(Label::public(), |tau, (label, _)| tau.join(label));
    Timing {
        label,
        sources,
        ops: seen,
    }
}

/// `a ⊔ b` over `None` as the least element: `None` when no construct counts.
fn join_some(a: &Option<Label>, b: &Option<Label>) -> Option<Label> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.join(b)),
        (Some(l), None) | (None, Some(l)) => Some(l.clone()),
        (None, None) => None,
    }
}

/// The labels of concurrent work (ADR 0116 P2 to P4), computed once per package.
pub(crate) struct Concurrency {
    /// `κ(C)` of each construct, by its id: `τ⁰` of its whole seed, closed over the graph of
    /// P1 (P2), so a part that calls a function value or `map` makes it `⊤` (F, item 7).
    pub(crate) kappa: Vec<Label>,
    /// `ctx` of each construct: the join of `κ` over it and the constructs around it in its
    /// body's tree (P3).
    pub(crate) ctx: Vec<Label>,
    /// `conc(n)` of each node of the graph (P3), `None` when no part of a construct reaches it.
    pub(crate) conc: Vec<Option<Label>>,
    /// `τ⁰` of every frame over the graph of P1, and whether it reaches an operation of `Http`
    /// ([`coarse_labels`]): what [`seed_label`] gives of the frame's rebuilt seed.
    coarse: Vec<(Label, bool)>,
    /// The number of functions and of handlers, and the node `Λ`, to find a caller's node as
    /// [`Graph::node_of`] does.
    functions: usize,
    handlers: usize,
    lambda: usize,
    /// `⊤`, for a site in no node (P4).
    top: Label,
}

impl Concurrency {
    /// `ctx` at a place whose innermost construct is `construct`, `None` outside every one, and
    /// `⊤` for an id that names no construct, failing closed.
    pub(crate) fn ctx_at(&self, construct: Option<usize>) -> Option<Label> {
        construct.map(|id| {
            self.ctx
                .get(id)
                .cloned()
                .unwrap_or_else(|| self.top.clone())
        })
    }

    /// [`seed_label`] of the frame `frame` ([`coarse_labels`]): `⊤` for an id that names no
    /// frame, failing closed.
    pub(crate) fn coarse(&self, frame: TimingFrame) -> (Label, bool) {
        self.coarse
            .get(frame.0)
            .cloned()
            .unwrap_or_else(|| (self.top.clone(), true))
    }

    /// `conc` of the function `f` (P3), `None` when no part of a construct reaches it: the label
    /// a tick of a driven `step` takes (ADR 0116 C), since every call of `step` counts.
    pub(crate) fn conc_of_function(&self, f: usize) -> Option<Label> {
        if f < self.functions {
            self.conc.get(f).cloned().flatten()
        } else {
            Some(self.top.clone())
        }
    }

    /// `τ_conc` of a site whose checking recorded `seed` (P4): `conc` of its node joined with
    /// `ctx` at it; `⊤` for a site in no node of the graph; `None` when nothing runs beside it.
    pub(crate) fn tau_conc(&self, seed: &TimingSeed) -> Option<Label> {
        let node = seed.node.and_then(|caller| match caller {
            Caller::Function(f) => (f < self.functions).then_some(f),
            Caller::Handler(h) => (h < self.handlers).then_some(self.functions + h),
            Caller::Lambda => Some(self.lambda),
        });
        let Some(node) = node else {
            return Some(self.top.clone());
        };
        let conc = self.conc.get(node).cloned().flatten();
        join_some(&conc, &self.ctx_at(seed.construct))
    }
}

/// [`concurrency`] of the package, with its graph built here: O(V + E log E) for the graph.
pub(crate) fn concurrency_of(env: &Env<'_>, findings: &Findings) -> Concurrency {
    concurrency(env, findings, &graph(env, findings))
}

/// `κ`, `ctx` and `conc` of the package (ADR 0116 P2 to P4).
///
/// - **`κ(C)`** is `τ⁰` of the frame of `C` over `graph`, its label bottom up
///   ([`coarse_labels`]), O(1) once the labels of every frame are computed.
/// - **`ctx`** comes from the root of each body's tree down, as `ctx(parent) ⊔ κ(C)`: a parent
///   is recorded before the constructs nested in it, so one pass in the order of the ids gives
///   it, O(F) joins.
/// - **`conc`** is the least solution of `conc(n) ≥ conc(m) ⊔ ctx(s)` for every edge from `m`
///   at the site `s` to `n` (P3). The `ctx(s)` of each recorded edge is a constant, so it seeds
///   the node the edge goes to (an operation's node for an operation, whose edges lead on to its
///   capability and its handlers), and `conc(n)` is the join of the seeds of every node that
///   reaches `n`, `n` included: one pass of [`close`] over the reversed graph, whose strongly
///   connected components share one label (a cycle of calls shares one `conc`). O(V + E) joins.
///
/// In all O(V + E + F + size of the seeds) joins, each O(k) for `k` categories. The `within` and
/// `first of` sites take their `τ_conc` from it ([`timing_precise`], slices 3 and 4 of the plan
/// on #1087), and so do the constructs `L_res` counts ([`residue`]), `for ever within` and the
/// tick (slice 5), and `L_lim` (#1154, [`limit_labels`]); limiters, `L_srv` and the rule of a
/// `via`'s timing sites keep the preview's.
pub(crate) fn concurrency(env: &Env<'_>, findings: &Findings, graph: &Graph) -> Concurrency {
    let coarse = coarse_labels(env, graph, &findings.frames);
    // In a build with debug assertions, and for a package with at most [`CROSS_CHECKED`]
    // constructs and `tasks` statements, the labels bottom up are those of the rebuilt seeds.
    let frames = findings
        .constructs
        .iter()
        .filter_map(|c| c.frame)
        .chain(findings.tasks_sites.iter().map(|site| site.frame));
    if cfg!(debug_assertions)
        && findings.constructs.len() + findings.tasks_sites.len() <= CROSS_CHECKED
    {
        for frame in frames {
            let rebuilt = seed_label(env, graph, &findings.frames.seed_of(frame.0, false));
            // INVARIANT: `coarse_labels` joins what `seed_label` joins over the rebuilt seed.
            assert_eq!(
                coarse.get(frame.0),
                Some(&rebuilt),
                "a frame's label bottom up disagrees"
            );
        }
    }
    let kappa: Vec<Label> = findings
        .constructs
        .iter()
        .map(|c| match c.frame {
            Some(frame) => coarse
                .get(frame.0)
                .map_or_else(|| Label::top(&env.categories), |(label, _)| label.clone()),
            // A construct whose frame was not recorded fails closed (P4, F item 6).
            None => Label::top(&env.categories),
        })
        .collect();
    let mut ctx: Vec<Label> = Vec::with_capacity(kappa.len());
    let top = Label::top(&env.categories);
    for (id, c) in findings.constructs.iter().enumerate() {
        una_core::label_counters::pass_step();
        let own = kappa.get(id).cloned().unwrap_or_else(|| top.clone());
        // A parent always has a lower id; one that names no construct counts as `⊤`.
        let outer = c
            .parent
            .map(|p| ctx.get(p).cloned().unwrap_or_else(|| top.clone()));
        ctx.push(outer.map_or_else(|| own.clone(), |outer| outer.join(&own)));
    }
    let nodes = graph.succ.len();
    let mut seed: Vec<Option<Label>> = vec![None; nodes];
    for edge in &findings.graph_edges {
        una_core::label_counters::pass_step();
        // An id that names no construct counts as `⊤`, failing closed.
        let Some(at) = edge.construct.map(|id| ctx.get(id).unwrap_or(&top)) else {
            continue;
        };
        let to = match &edge.to {
            Target::Function(f) => (*f < graph.functions).then_some(*f),
            Target::Lambda => Some(graph.lambda),
            Target::Op(cap, op) => graph.ops.get(&(*cap, op.clone())).copied(),
            Target::Install(h) => (*h < graph.handlers).then(|| graph.handler_node(*h)),
        };
        if let Some(slot) = to.and_then(|to| seed.get_mut(to)) {
            *slot = join_some(slot, &Some(at.clone()));
        }
    }
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for (from, tos) in graph.succ.iter().enumerate() {
        for &to in tos {
            if let Some(edges) = pred.get_mut(to) {
                edges.push(from);
            }
        }
    }
    let conc = close(&pred, &seed, join_some);
    Concurrency {
        kappa,
        ctx,
        conc,
        coarse,
        functions: graph.functions,
        handlers: graph.handlers,
        lambda: graph.lambda,
        top,
    }
}

/// A node of the graph of ADR 0116 P1, as [`TimingGraph`] shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimingNode {
    /// The body of a function, by its qualified name, `module.name`.
    Function(String),
    /// The operations of a handler, by its qualified name.
    Handler(String),
    /// `Λ`, the node of every lambda.
    Lambda,
}

/// What an edge of [`TimingGraph`] goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimingTarget {
    /// A function called or named as a value, by its qualified name.
    Function(String),
    /// `Λ`: a call of a function value.
    Lambda,
    /// An operation, by its capability's name and its own.
    Operation(String, String),
    /// A handler installed, by its qualified name.
    Install(String),
}

/// An edge of [`TimingGraph`]: the node whose code holds the call, what it goes to, the call
/// site, and the innermost task construct whose part holds the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingEdge {
    /// The node whose code holds the call.
    pub from: TimingNode,
    /// What it goes to.
    pub to: TimingTarget,
    /// The call site.
    pub span: una_syntax::Span,
    /// The innermost construct whose part holds it, an index of [`TimingGraph::constructs`].
    pub construct: Option<usize>,
}

/// A task construct, or a timing site, of [`TimingGraph`], placed in its body's construct tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingPlace {
    /// Its words: `together`, `together for`, `first of` or `tasks` for a construct, `within`,
    /// `first of`, `for ever within` or `tasks` for a site.
    pub kind: &'static str,
    /// Where it is.
    pub span: una_syntax::Span,
    /// The node whose code holds it, `None` outside every node.
    pub node: Option<TimingNode>,
    /// The innermost construct whose part holds it, an index of [`TimingGraph::constructs`].
    pub construct: Option<usize>,
    /// For a construct, `κ` (ADR 0116 P2). For a site, `τ_conc` (P4): `None` when no part of a
    /// construct can run beside it, which is `τ_conc = public`.
    pub label: Option<Label>,
}

/// The graph of ADR 0116 P1 and the construct trees of P3, as checking recorded them, for the
/// completeness test of P1 (`una-cli/tests/timing_graph.rs`), which holds them to the Core IR
/// of every package of the conformance suite.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimingGraph {
    /// Each edge, at its call site, in the order checking recorded them.
    pub edges: Vec<TimingEdge>,
    /// Each task construct, by the ids edges and places record; its `construct` is its parent.
    pub constructs: Vec<TimingPlace>,
    /// Each timing site, in the order checking recorded them.
    pub sites: Vec<TimingPlace>,
}

/// [`TimingGraph`] of the package from its findings: O(edges + constructs + sites).
pub(crate) fn recorded_graph(env: &Env<'_>, findings: &Findings) -> TimingGraph {
    let conc = concurrency(env, findings, &graph(env, findings));
    let qualified =
        |module: usize, name: &str| match env.modules.get(module).map(|m| m.path.as_str()) {
            Some(path) if !path.is_empty() => format!("{path}.{name}"),
            _ => name.to_owned(),
        };
    let function = |f: usize| {
        env.functions
            .get(f)
            .map_or_else(String::new, |def| qualified(def.module, def.name))
    };
    let handler = |h: usize| {
        env.handlers.get(h).map_or_else(String::new, |def| {
            qualified(def.module, env.text(def.module, def.ast.name.name))
        })
    };
    let node = |caller: Caller| match caller {
        Caller::Function(f) => TimingNode::Function(function(f)),
        Caller::Handler(h) => TimingNode::Handler(handler(h)),
        Caller::Lambda => TimingNode::Lambda,
    };
    // The edges of code outside every node have no node to show.
    let edges = findings
        .graph_edges
        .iter()
        .filter_map(|edge| Some((edge, edge.from?)))
        .map(|(edge, from)| TimingEdge {
            from: node(from),
            to: match &edge.to {
                Target::Function(f) => TimingTarget::Function(function(*f)),
                Target::Lambda => TimingTarget::Lambda,
                Target::Op(cap, op) => TimingTarget::Operation(
                    env.capabilities
                        .get(*cap)
                        .map_or_else(String::new, |c| c.name.to_owned()),
                    op.clone(),
                ),
                Target::Install(h) => TimingTarget::Install(handler(*h)),
            },
            span: edge.span,
            construct: edge.construct,
        })
        .collect();
    let constructs = findings
        .constructs
        .iter()
        .enumerate()
        .map(|(id, c)| TimingPlace {
            kind: c.kind.words(),
            span: c.span,
            node: c.node.map(node),
            construct: c.parent,
            label: conc.kappa.get(id).cloned(),
        })
        .collect();
    let place = |kind: &'static str, span: una_syntax::Span, seed: &TimingSeed| TimingPlace {
        kind,
        span,
        node: seed.node.map(node),
        construct: seed.construct,
        label: conc.tau_conc(seed),
    };
    let mut sites: Vec<TimingPlace> = findings
        .withins
        .iter()
        .map(|site| {
            place(
                site.construct.words(),
                site.span,
                &findings.frames.own(site.frame),
            )
        })
        .collect();
    sites.extend(findings.deadline_loops.iter().map(|site| {
        place(
            "for ever within",
            site.span,
            &findings.frames.own(site.frame),
        )
    }));
    sites.extend(
        findings
            .tasks_sites
            .iter()
            .map(|site| place("tasks", site.span, &findings.frames.own(site.frame))),
    );
    TimingGraph {
        edges,
        constructs,
        sites,
    }
}

#[cfg(test)]
mod tests {
    use super::{Unseen, reached_from, unseen};

    #[test]
    fn an_unchecked_dependency_or_a_standard_task_construct_is_top_and_nothing_else_is() {
        // #1148, failing closed until #1166. A dependency's function whose code the checker
        // does not see, a module not checked with the package or a function without a body,
        // is `⊤` whenever the package reaches it, whatever its body would hold.
        assert_eq!(
            unseen(false, true, false, Some(false)),
            Some(Unseen::UncheckedDependency)
        );
        assert_eq!(
            unseen(false, true, true, None),
            Some(Unseen::UncheckedDependency)
        );
        // A checked dependency counts precisely, through the findings.
        assert_eq!(unseen(false, true, true, Some(true)), None);
        assert_eq!(unseen(false, true, true, Some(false)), None);
        // A standard function only when its body holds a construct: those without a body are
        // the host's, which run no Una code (ADR 0116 P0).
        assert_eq!(
            unseen(true, false, false, Some(true)),
            Some(Unseen::StandardTasks)
        );
        assert_eq!(unseen(true, false, false, Some(false)), None);
        assert_eq!(unseen(true, false, false, None), None);
        // The root package's own functions never.
        assert_eq!(unseen(false, false, true, Some(true)), None);
    }

    #[test]
    fn only_what_the_package_reaches_counts() {
        // Node 0 is the package's, 1 a dependency function it calls, 2 one it does not.
        let succ = vec![vec![1], vec![], vec![1]];
        assert_eq!(reached_from(&succ, [0]), [true, true, false]);
        // A cycle, and a root given twice.
        let succ = vec![vec![1], vec![0], vec![]];
        assert_eq!(reached_from(&succ, [1, 1]), [true, true, false]);
    }
}
