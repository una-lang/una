//! The editor's hovers of types (ADR 0080): a `Send` snapshot of each checked type, made once
//! per shared node of the checker's types, and printed only when the editor asks for it.
//!
//! The checker's types share their nodes through `Rc` (ADR 0077), which cannot leave the
//! checking thread, so the index keeps a copy whose nodes are `Arc`. A copy is made once per
//! shared checker node, under ADR 0077's memo rules: keyed by the node's address, with the node
//! kept alive beside its copy, so no address is reused while the memo lives. Building every
//! hover of a package is then O(n), where formatting each one was O(n·d), and a hover costs the
//! size of the one type it shows.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use una_core::IntType;

use super::label::Label;
use super::shared::Shared;
use super::ty::{FnTy, KindId, Names, ShowLabeled, Ty, TypeView, View};

/// A checked type as the editor's index keeps it: immutable, and `Send`, so it leaves the
/// checking thread with the index (ADR 0080).
#[derive(Clone, Debug)]
pub struct HoverType(Arc<Node>);

/// A node of a [`HoverType`]: the shape the printers read ([`View`]), and nothing else.
#[derive(Debug)]
enum Node {
    Leaf(Leaf),
    Labeled(Label, HoverType),
    /// A measured type (ADR 0104): its unit, owned, and its magnitude type.
    Measured(super::units::Unit, HoverType),
    List(HoverType),
    Dense(super::ty::Dense, HoverType),
    Map(HoverType, HoverType),
    Kind(KindId, Arc<[HoverType]>),
    Param(usize),
    Fn(Arc<[HoverType]>, HoverType),
}

/// A node without children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaf {
    Int,
    Fixed(IntType),
    Bool,
    Text,
    Unit,
    Hashed,
    F64,
    Decimal(u8),
    Var,
    Never,
    Error,
}

impl TypeView for HoverType {
    fn view(&self) -> View<'_, HoverType> {
        match self.0.as_ref() {
            Node::Leaf(leaf) => match *leaf {
                Leaf::Int => View::Int,
                Leaf::Fixed(int) => View::Fixed(int),
                Leaf::Bool => View::Bool,
                Leaf::Text => View::Text,
                Leaf::Unit => View::Unit,
                Leaf::Hashed => View::Hashed,
                Leaf::F64 => View::F64,
                Leaf::Decimal(scale) => View::Decimal(scale),
                Leaf::Var => View::Var,
                Leaf::Never => View::Never,
                Leaf::Error => View::Error,
            },
            Node::Labeled(label, inner) => View::Labeled(label, inner),
            Node::Measured(unit, inner) => View::Measured(unit, inner),
            Node::List(element) => View::List(element),
            Node::Dense(dense, element) => View::Dense(*dense, element),
            Node::Map(key, value) => View::Map(key, value),
            Node::Kind(id, args) => View::Kind(*id, args),
            Node::Param(index) => View::Param(*index),
            Node::Fn(params, result) => View::Fn(params, result),
        }
    }
}

/// The names of the package's kinds, by [`KindId`], as declared: one table for every hover of
/// a check.
#[derive(Debug, Default)]
pub(crate) struct HoverNames {
    pub(crate) kinds: Vec<String>,
}

/// The type of one expression, for its hover: the type, the type parameters of the declaration
/// it is in, and the names of the package's kinds. Its text is what [`fmt::Display`] prints,
/// made when it is asked for.
#[derive(Clone, Debug)]
pub struct Hover {
    ty: HoverType,
    params: Arc<[String]>,
    names: Arc<HoverNames>,
}

impl Hover {
    pub(crate) fn new(ty: HoverType, params: Arc<[String]>, names: Arc<HoverNames>) -> Hover {
        Hover { ty, params, names }
    }

    /// The type as written in Una, with its labels, as the hover shows it.
    pub fn text(&self) -> String {
        self.to_string()
    }
}

impl Names for Hover {
    fn kind_name(&self, id: KindId) -> String {
        self.names
            .kinds
            .get(id)
            .cloned()
            .unwrap_or_else(|| "?".to_owned())
    }

    fn param_name(&self, index: usize) -> String {
        self.params
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("T{index}"))
    }
}

impl fmt::Display for Hover {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ShowLabeled(&self.ty, self))
    }
}

/// What building the hovers of a check did, for the scaling test (ADR 0080): counted per
/// build, never in a process-wide counter, so concurrent checks do not disturb it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HoverStats {
    /// The nodes of hover types made.
    pub nodes: usize,
}

/// The copies of shared lists of types, by the address of each list, with the list kept alive.
type ListMemo = HashMap<*const (), (Shared<[Ty]>, Arc<[HoverType]>)>;

/// The copies of checker types made for one build of the index, each shared checker node once
/// (ADR 0077's memo rules: keyed by address, the node kept alive beside its copy).
#[derive(Default)]
pub(crate) struct Snapshots {
    types: HashMap<*const (), (Shared<Ty>, HoverType)>,
    lists: ListMemo,
    functions: HashMap<*const (), (Shared<FnTy>, HoverType)>,
    leaves: Vec<(Leaf, HoverType)>,
    pub(crate) stats: HoverStats,
}

impl Snapshots {
    /// The hover type of `ty`.
    pub(crate) fn of(&mut self, ty: &Ty) -> HoverType {
        match ty {
            Ty::Int => self.leaf(Leaf::Int),
            Ty::Fixed(int) => self.leaf(Leaf::Fixed(*int)),
            Ty::Bool => self.leaf(Leaf::Bool),
            Ty::Text => self.leaf(Leaf::Text),
            Ty::Unit => self.leaf(Leaf::Unit),
            Ty::Hashed => self.leaf(Leaf::Hashed),
            Ty::F64 => self.leaf(Leaf::F64),
            Ty::Decimal(scale) => self.leaf(Leaf::Decimal(*scale)),
            Ty::Var(_) => self.leaf(Leaf::Var),
            Ty::Never => self.leaf(Leaf::Never),
            Ty::Error(_) => self.leaf(Leaf::Error),
            Ty::Labeled(label, inner) => {
                let inner = self.shared(inner);
                self.node(Node::Labeled(label.clone(), inner))
            }
            Ty::Measured(unit, inner) => {
                let inner = self.shared(inner);
                self.node(Node::Measured((**unit).clone(), inner))
            }
            // A hover shows a refined position's base (ADR 0103).
            Ty::Refined(_, inner) => self.of(inner),
            Ty::List(element) => {
                let element = self.shared(element);
                self.node(Node::List(element))
            }
            Ty::Dense(dense, element) => {
                let element = self.shared(element);
                self.node(Node::Dense(*dense, element))
            }
            Ty::Map(key, value) => {
                let (key, value) = (self.shared(key), self.shared(value));
                self.node(Node::Map(key, value))
            }
            Ty::Kind(id, args) => {
                let args = self.list(args);
                self.node(Node::Kind(*id, args))
            }
            Ty::Param(index) => self.node(Node::Param(*index)),
            Ty::Fn(function) => self.function(function),
        }
    }

    /// The copy of the shared node `ty`, made once.
    fn shared(&mut self, ty: &Shared<Ty>) -> HoverType {
        if let Some((_, copy)) = self.types.get(&ty.address()) {
            return copy.clone();
        }
        let copy = self.of(ty);
        self.types.insert(ty.address(), (ty.clone(), copy.clone()));
        copy
    }

    /// The copies of the shared list `types`, made once.
    fn list(&mut self, types: &Shared<[Ty]>) -> Arc<[HoverType]> {
        if let Some((_, copy)) = self.lists.get(&types.address()) {
            return Arc::clone(copy);
        }
        let copy: Arc<[HoverType]> = types.iter().map(|ty| self.of(ty)).collect();
        self.lists
            .insert(types.address(), (types.clone(), Arc::clone(&copy)));
        copy
    }

    /// The copy of the shared function type `function`, made once.
    fn function(&mut self, function: &Shared<FnTy>) -> HoverType {
        if let Some((_, copy)) = self.functions.get(&function.address()) {
            return copy.clone();
        }
        let params: Arc<[HoverType]> = function.params.iter().map(|p| self.of(p)).collect();
        let result = self.of(&function.result);
        let copy = self.node(Node::Fn(params, result));
        self.functions
            .insert(function.address(), (function.clone(), copy.clone()));
        copy
    }

    /// The node for `leaf`, one per kind of leaf.
    fn leaf(&mut self, leaf: Leaf) -> HoverType {
        if let Some((_, node)) = self.leaves.iter().find(|(known, _)| *known == leaf) {
            return node.clone();
        }
        let node = self.node(Node::Leaf(leaf));
        self.leaves.push((leaf, node.clone()));
        node
    }

    fn node(&mut self, node: Node) -> HoverType {
        self.stats.nodes += 1;
        HoverType(Arc::new(node))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::label::Label;
    use super::super::shared::Shared;
    use super::super::ty::{FnRefinements, FnTy, KindId, Names, ShowLabeled, Ty};
    use super::{Hover, HoverNames, HoverType, Snapshots};
    use una_core::{IntType, Level};

    /// The names a declaration with type parameters `T` and `U` sees, as the index's old
    /// `DeclNames` gave them.
    struct Plain;

    impl Names for Plain {
        fn kind_name(&self, id: KindId) -> String {
            ["Box", "Pair"]
                .get(id)
                .map_or_else(|| "?".to_owned(), |name| (*name).to_owned())
        }

        fn param_name(&self, index: usize) -> String {
            ["T", "U"]
                .get(index)
                .map_or_else(|| format!("T{index}"), |name| (*name).to_owned())
        }
    }

    fn hover(snapshots: &mut Snapshots, ty: &Ty) -> Hover {
        let names = Arc::new(HoverNames {
            kinds: vec!["Box".to_owned(), "Pair".to_owned()],
        });
        let params: Arc<[String]> = Arc::from(vec!["T".to_owned(), "U".to_owned()]);
        Hover::new(snapshots.of(ty), params, names)
    }

    fn secret() -> Label {
        let mut label = Label::public();
        label.level = Level::Secret;
        label
    }

    /// Types of every shape the printers read, nested, labeled and shared.
    fn shapes() -> Vec<Ty> {
        let text = Shared::new(Ty::Text);
        let labeled = Ty::Labeled(secret(), Shared::new(Ty::Int));
        let function = Ty::Fn(Shared::new(FnTy {
            params: vec![Ty::Param(0), labeled.clone()],
            result: Ty::List(text.clone()),
            uses: Vec::new(),
            fails: None,
            refinements: FnRefinements::default(),
        }));
        let args: Shared<[Ty]> = vec![Ty::Param(1), labeled.clone()].into();
        vec![
            Ty::Int,
            Ty::Fixed(IntType::U8),
            Ty::Bool,
            Ty::Text,
            Ty::Unit,
            Ty::Hashed,
            Ty::F64,
            Ty::Var(3),
            Ty::Never,
            Ty::error(),
            Ty::Param(0),
            Ty::Param(7),
            labeled.clone(),
            Ty::List(Shared::new(labeled.clone())),
            Ty::Map(text.clone(), Shared::new(labeled.clone())),
            Ty::Kind(0, Vec::new().into()),
            Ty::Kind(1, args.clone()),
            Ty::Kind(9, args),
            function.clone(),
            Ty::Labeled(secret(), Shared::new(function)),
            Ty::List(Shared::new(Ty::List(Shared::new(Ty::List(text))))),
        ]
    }

    #[test]
    fn a_hover_prints_what_a_message_prints_for_every_shape() {
        // ADR 0080: one printer over two trees; the copy keeps every node, label and name.
        let mut snapshots = Snapshots::default();
        for ty in shapes() {
            assert_eq!(
                hover(&mut snapshots, &ty).text(),
                ShowLabeled(&ty, &Plain).to_string(),
                "{ty:?}"
            );
        }
    }

    #[test]
    fn a_shared_node_is_copied_once() {
        // ADR 0080: the copy of a node shared by many types is made once, so the index is
        // linear in the checker's nodes, as ADR 0077 made them.
        let shared = Shared::new(Ty::List(Shared::new(Ty::Labeled(
            secret(),
            Shared::new(Ty::Int),
        ))));
        let mut snapshots = Snapshots::default();
        let first = snapshots.of(&Ty::List(shared.clone()));
        let made = snapshots.stats.nodes;
        for _ in 0..100 {
            let again = snapshots.of(&Ty::List(shared.clone()));
            assert_eq!(again.to_string_for_test(), first.to_string_for_test());
        }
        // Each further type adds only its own top node.
        assert_eq!(snapshots.stats.nodes, made + 100);
    }

    #[test]
    fn hovers_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<HoverType>();
        assert_send_sync::<Hover>();
        assert_send_sync::<super::super::Index>();
    }

    impl HoverType {
        fn to_string_for_test(&self) -> String {
            ShowLabeled(self, &Plain).to_string()
        }
    }
}
