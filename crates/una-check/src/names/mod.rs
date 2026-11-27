//! Names (M3, ADR 0023): module paths (U1008), declarations and duplicates (U1002), imports
//! (U1001, U1003, U1004), every name of every body (U1001), naming conventions (U1005),
//! requirements (U1002, U1006, U1007) and trusted modules (U7005).

mod conventions;
pub(crate) mod prelude;
mod resolve;

use std::collections::{BTreeMap, BTreeSet};

use una_core::IntType;
use una_diag::{Code, Diagnostic, Edit, Escape, Related, Repair, Widening};
use una_project::{Edition, Manifest, is_module_path};
use una_syntax::ast::{ClauseKind, DeclKind, File, Ident, KindBody, RequirementSource};
use una_syntax::{FileId, Interner, SourceDb, Span};

/// A module of the package, parsed.
#[derive(Clone, Copy, Debug)]
pub struct ModuleInput<'a> {
    /// Its file.
    pub id: FileId,
    /// Its syntax tree.
    pub tree: &'a File,
    /// Whether it has syntax errors: then its bodies are not resolved (ADR 0023 rule 6).
    pub broken: bool,
}

/// A package, parsed.
#[derive(Clone, Debug)]
pub struct PackageInput<'a> {
    /// Every file of the package.
    pub sources: &'a SourceDb,
    /// The interner of every module.
    pub interner: &'a Interner,
    /// Whether the package is a single file checked on its own (ADR 0021 rule 1).
    pub single: bool,
    /// The modules, in path order.
    pub modules: Vec<ModuleInput<'a>>,
    /// `una.toml`, for a package directory.
    pub manifest: Option<FileId>,
    /// The settings of `una.toml`, or the defaults.
    pub settings: &'a Manifest,
    /// The entry module and grants of `una run`, when the package is checked to run it.
    pub run: Option<RunInput<'a>>,
    /// Whether the package has an error that the type check does not see: a syntax, manifest
    /// or name error. Edition 0.3's U3025 and U3026 wait for them (ADR 0100 rule 4).
    pub earlier_errors: bool,
    /// The locked functions of `una.toml`'s `[contracts]`, edition 0.3 (ADR 0092 rule 8, as
    /// amended), with their clauses parsed.
    pub locks: &'a [Lock],
}

/// A locked function of `[contracts]` (ADR 0092 rule 8, as amended).
#[derive(Clone, Debug)]
pub struct Lock {
    /// The function's qualified name, as the key writes it.
    pub function: String,
    /// Where the key is written in `una.toml`.
    pub key: Span,
    /// The locked `uses` clauses.
    pub clauses: Vec<LockClause>,
}

/// A locked `uses` clause of `[contracts]`.
#[derive(Clone, Debug)]
pub struct LockClause {
    /// Where its string is written in `una.toml`.
    pub span: Span,
    /// The clause, or the first syntax error of its text.
    pub parsed: Result<una_syntax::ast::UsesClause, una_syntax::SyntaxError>,
    /// The clause as its string reads, with its escapes decoded.
    pub value: String,
    /// Where each byte boundary of `value` is written, from the start of `span`; `None` when
    /// the string could not be read back, and `parsed` is then an error (#727).
    pub offsets: Option<una_project::literal::Offsets>,
}

impl LockClause {
    /// The text at `span`, a span of the parsed clause, as the string reads: a condition
    /// written `name == \"a\"` in `una.toml` reads `name == "a"` (#727).
    pub fn text(&self, span: Span) -> Option<&str> {
        let start = span.start.checked_sub(self.span.start)?;
        let end = span.end.checked_sub(self.span.start)?;
        let range = self.offsets.as_ref()?.value_range(start, end)?;
        self.value.get(range)
    }
}

/// What `una run` adds to a check: the module whose `main` runs, and the grants of
/// `[run] grants` and `--grant` together, which are final (`spec/04 §8`).
#[derive(Clone, Copy, Debug)]
pub struct RunInput<'a> {
    /// The path of the entry module.
    pub entry: &'a str,
    /// The grant specs.
    pub grants: &'a [String],
}

/// The name diagnostics of `package`.
pub fn check_names(package: &PackageInput<'_>) -> Vec<Diagnostic> {
    check_names_indexed(package).0
}

/// The names that the broken declarations of a module would have declared, had they parsed
/// (ADR 0096 rule 8). A reference to one of them is unknown, never U1001.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Poison {
    /// The names read from the broken declarations' tokens.
    pub names: BTreeSet<String>,
    /// Whether a broken declaration has no name that can be read: then every name of the
    /// module that does not resolve, and every name imported from it, is unknown.
    pub unnamed: bool,
}

/// [`check_names`] for a package some of whose modules have broken declarations: the
/// declarations that parsed are resolved, and the names in `poison`, by module, are unknown.
pub fn check_names_poisoned(
    package: &PackageInput<'_>,
    poison: &BTreeMap<FileId, Poison>,
) -> (Vec<Diagnostic>, Vec<(Span, Span)>) {
    check_names_with(package, Some(poison))
}

/// [`check_names`], with each name that resolves to a declaration or a local of the package
/// and the span of its declaration, for the language server (ADR 0063).
pub fn check_names_indexed(package: &PackageInput<'_>) -> (Vec<Diagnostic>, Vec<(Span, Span)>) {
    check_names_with(package, None)
}

fn check_names_with(
    package: &PackageInput<'_>,
    poison: Option<&BTreeMap<FileId, Poison>>,
) -> (Vec<Diagnostic>, Vec<(Span, Span)>) {
    let mut checker = Checker {
        package,
        diagnostics: Vec::new(),
        references: Vec::new(),
        poison,
        // ADR 0122 rule 49: each module's edition, its package's, found once per module.
        editions: package
            .modules
            .iter()
            .filter_map(|module| {
                let path = package.sources.file(module.id).path();
                package
                    .settings
                    .mount_of(path)
                    .map(|mount| (module.id, mount.edition))
            })
            .collect(),
    };
    let mut tables: Vec<Table<'_>> = package
        .modules
        .iter()
        .map(|module| checker.declarations(module))
        .collect();
    let paths = checker.module_paths(&tables);
    checker.local_name_clashes(&tables);
    checker.requirements();
    for index in 0..tables.len() {
        checker.imports(&mut tables, index, &paths);
    }
    checker.cycles(&tables);
    checker.trusted(&tables, &paths);
    let requirements = checker.requirement_ids();
    for table in &tables {
        if !table.module.broken {
            resolve::module(&mut checker, table, &requirements);
        }
    }
    (checker.diagnostics, checker.references)
}

/// What an imported name refers to.
enum Found<'a> {
    /// A declaration of a module of the package.
    Declared(Declared<'a>, FileId),
    /// A function of `std.testing`.
    Standard,
    /// Nothing: U1001.
    Missing,
    /// Unknown, since the module has syntax errors.
    Unknown,
}

/// What a top-level name declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum What {
    Kind,
    Variant,
    Capability,
    Function,
    Predicate,
    Handler,
    Policy,
    /// A unit of measure, edition 0.3 (ADR 0104 rule 1): in the upper namespace with kinds,
    /// so a unit and a kind of the same name clash (U1002).
    Measure,
    /// A module constant, edition 0.5 (ADR 0126 rule 2): in the lower namespace with
    /// functions, so a constant and a function of the same name clash (U1002).
    Constant,
}

impl What {
    fn word(self) -> &'static str {
        match self {
            What::Kind => "kind",
            What::Variant => "variant",
            What::Capability => "capability",
            What::Function => "function",
            What::Predicate => "predicate",
            What::Handler => "handler",
            What::Policy => "policy",
            What::Measure => "unit",
            What::Constant => "constant",
        }
    }
}

/// A top-level name of a module.
#[derive(Clone, Debug)]
struct Declared<'a> {
    what: What,
    public: bool,
    /// Where the name is written.
    span: Span,
    /// Where the declaration starts, for the edit that adds `pub`.
    start: usize,
    /// The variants of a sum kind.
    variants: Vec<&'a str>,
}

/// The names a module declares and imports.
#[derive(Debug)]
struct Table<'a> {
    module: ModuleInput<'a>,
    /// Its path, from its header.
    path: Option<String>,
    upper: BTreeMap<&'a str, Declared<'a>>,
    lower: BTreeMap<&'a str, Declared<'a>>,
    /// Names imported explicitly, with where the import names them.
    imported: BTreeMap<&'a str, Span>,
    /// Variants of the sum kinds it imports (ADR 0023 rule 2).
    imported_variants: BTreeSet<&'a str>,
    /// Imports of modules of the package: (module, where the import names it).
    edges: Vec<(usize, Span)>,
}

struct Checker<'p, 'a> {
    package: &'p PackageInput<'a>,
    diagnostics: Vec<Diagnostic>,
    /// Each name that resolves to a declaration or a local of the package, and where that is
    /// declared (ADR 0063).
    references: Vec<(Span, Span)>,
    /// The poisoned names of each module with broken declarations (ADR 0096 rule 8).
    poison: Option<&'p BTreeMap<FileId, Poison>>,
    /// The edition of each module of a dependency (ADR 0122 rule 49); every other file is of
    /// the package's edition.
    editions: BTreeMap<FileId, Edition>,
}

impl<'p, 'a> Checker<'p, 'a> {
    /// Whether `name` is unknown in the module `file`, since a broken declaration of it may
    /// declare it (ADR 0096 rule 8).
    fn poisoned(&self, file: FileId, name: &str) -> bool {
        self.poison
            .and_then(|poison| poison.get(&file))
            .is_some_and(|poison| poison.unnamed || poison.names.contains(name))
    }

    /// The local name of the dependency whose module is the file `id`, if it is one.
    fn mount(&self, id: FileId) -> Option<&'a str> {
        let settings: &'a Manifest = self.package.settings;
        settings
            .mount_of(self.package.sources.file(id).path())
            .map(|mount| mount.local.as_str())
    }

    /// The edition the module of file `id` is checked under: its package's (ADR 0122 rule 49),
    /// never the importing package's.
    /// O(log m) for m dependency modules.
    fn edition(&self, id: FileId) -> Edition {
        self.editions
            .get(&id)
            .copied()
            .unwrap_or(self.package.settings.edition)
    }

    fn name(&self, ident: &Ident) -> &'a str {
        self.package.interner.resolve(ident.name)
    }

    fn make(&self, code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic::error(code, message, self.package.sources.location(span))
    }

    fn error(&mut self, code: Code, span: Span, message: impl Into<String>) {
        let diagnostic = self.make(code, span, message);
        self.diagnostics.push(diagnostic);
    }

    /// A repair without an edit.
    fn advice(description: String) -> Repair {
        Repair {
            widens: None,
            rank: 1,
            escape: None,
            description,
            edit: None,
        }
    }

    /// A repair without an edit that takes the escape hatch `escape`: output ranks it after
    /// every repair that keeps the guarantee (ADR 0095).
    fn escape(escape: Escape, description: String) -> Repair {
        Repair {
            escape: Some(escape),
            ..Self::advice(description)
        }
    }

    /// A repair that replaces `span` with `replacement`.
    fn edit(&self, description: String, span: Span, replacement: String) -> Repair {
        Repair {
            widens: None,
            rank: 1,
            escape: None,
            description,
            edit: Some(Edit {
                location: self.package.sources.location(span),
                replacement,
            }),
        }
    }

    fn related(&self, span: Span, message: &str) -> Related {
        Related {
            location: self.package.sources.location(span),
            message: message.to_owned(),
        }
    }

    /// U1002 at `span` for a name first defined at `first`.
    fn duplicate(&mut self, span: Span, first: Span, message: String) {
        let mut diagnostic = self.make(Code::U1002, span, message);
        diagnostic
            .related
            .push(self.related(first, "first defined here"));
        self.diagnostics.push(diagnostic);
    }

    /// U1005 when a lower-case name breaks the convention.
    fn lower_convention(&mut self, ident: &Ident, what: &str) {
        let name = self.name(ident);
        if let Some((problem, suggestion)) = conventions::lower(name) {
            self.convention(ident.span, what, name, &problem, &suggestion);
        }
    }

    /// U1005 when an upper-case name breaks the convention.
    fn upper_convention(&mut self, ident: &Ident, what: &str) {
        let name = self.name(ident);
        if let Some((problem, suggestion)) = conventions::upper(name) {
            self.convention(ident.span, what, name, &problem, &suggestion);
        }
    }

    fn convention(&mut self, span: Span, what: &str, name: &str, problem: &str, suggestion: &str) {
        let mut diagnostic = self.make(
            Code::U1005,
            span,
            format!("{what} `{name}` {problem}, as in `{suggestion}`"),
        );
        diagnostic.repairs.push(Self::advice(format!(
            "rename `{name}` to `{suggestion}` here and where it is used"
        )));
        self.diagnostics.push(diagnostic);
    }

    /// U1005 for the requirement identifier `id` (#743). The name suggested is derived from
    /// `id` and free in the package. When no `derived from` clause names `id`, the repair is the
    /// edit of the identifier; otherwise renaming it also changes those clauses, in other
    /// declarations, so the repair is advice that lists them.
    fn requirement_convention(
        &mut self,
        span: Span,
        id: &str,
        names: &conventions::RequirementNames<'_>,
        cited: Option<&Vec<Span>>,
    ) {
        let suggestion = names.unused(conventions::requirement_name(id));
        let mut diagnostic = self.make(
            Code::U1005,
            span,
            format!(
                "requirement identifier `{id}` must be upper snake case ending with `_` and a number, as in `{suggestion}`"
            ),
        );
        let repair = match cited {
            None => self.edit(
                format!("rename `{id}` to `{suggestion}`; no `derived from` clause names it"),
                span,
                suggestion.clone(),
            ),
            Some(sites) => {
                let places: Vec<String> = sites
                    .iter()
                    .map(|site| {
                        let location = self.package.sources.location(*site);
                        format!("{}:{}", location.file, location.start.line)
                    })
                    .collect();
                Self::advice(format!(
                    "rename `{id}` to `{suggestion}` here and in the `derived from` clauses that name it, at {}",
                    places.join(", ")
                ))
            }
        };
        diagnostic.repairs.push(repair);
        self.diagnostics.push(diagnostic);
    }

    /// The top-level names of a module, with U1002 for repeats and U1005 for conventions.
    fn declarations(&mut self, module: &ModuleInput<'a>) -> Table<'a> {
        let tree = module.tree;
        let mut table = Table {
            module: *module,
            path: tree.module.as_ref().map(|header| {
                let names: Vec<&str> = header
                    .path
                    .segments
                    .iter()
                    .map(|segment| self.name(segment))
                    .collect();
                // A dependency's module `m` is `<local>.m` in the program (ADR 0122 rule 47).
                match self.mount(module.id) {
                    Some(local) => format!("{local}.{}", names.join(".")),
                    None => names.join("."),
                }
            }),
            upper: BTreeMap::new(),
            lower: BTreeMap::new(),
            imported: BTreeMap::new(),
            imported_variants: BTreeSet::new(),
            edges: Vec::new(),
        };
        if let Some(header) = &tree.module {
            for segment in &header.path.segments {
                self.lower_convention(segment, "module path segment");
            }
            self.module_path(module, header.path.span);
            self.reserved_module(header);
        }
        let mut examples: BTreeMap<String, Span> = BTreeMap::new();
        for decl in &tree.decls {
            let start = decl.span.start;
            match &decl.kind {
                DeclKind::Kind(kind) => {
                    let variants: Vec<&'a str> = match &kind.body {
                        KindBody::Sum(variants) => variants
                            .iter()
                            .map(|variant| self.name(&variant.name))
                            .collect(),
                        KindBody::Alias { .. } | KindBody::Record(_) => Vec::new(),
                    };
                    self.declare(
                        &mut table,
                        &kind.name,
                        What::Kind,
                        kind.public,
                        start,
                        variants,
                    );
                    if let KindBody::Sum(variants) = &kind.body {
                        for variant in variants {
                            self.declare(
                                &mut table,
                                &variant.name,
                                What::Variant,
                                kind.public,
                                start,
                                Vec::new(),
                            );
                        }
                    }
                }
                DeclKind::Capability(capability) => {
                    self.declare(
                        &mut table,
                        &capability.name,
                        What::Capability,
                        capability.public,
                        start,
                        Vec::new(),
                    );
                }
                DeclKind::Function(function) => {
                    self.declare(
                        &mut table,
                        &function.name,
                        What::Function,
                        function.public,
                        start,
                        Vec::new(),
                    );
                }
                DeclKind::Predicate(predicate) => {
                    self.declare(
                        &mut table,
                        &predicate.name,
                        What::Predicate,
                        predicate.public,
                        start,
                        Vec::new(),
                    );
                }
                DeclKind::Handler(handler) => {
                    self.declare(
                        &mut table,
                        &handler.name,
                        What::Handler,
                        handler.public,
                        start,
                        Vec::new(),
                    );
                }
                DeclKind::Policy(policy) => {
                    self.declare(
                        &mut table,
                        &policy.name,
                        What::Policy,
                        policy.public,
                        start,
                        Vec::new(),
                    );
                    self.lower_convention(&policy.owner, "owner");
                }
                DeclKind::Requirement(_) => {}
                DeclKind::Measure(measure) => {
                    self.declare(
                        &mut table,
                        &measure.name,
                        What::Measure,
                        measure.public,
                        start,
                        Vec::new(),
                    );
                }
                DeclKind::Const(constant) => {
                    self.declare(
                        &mut table,
                        &constant.name,
                        What::Constant,
                        constant.public,
                        start,
                        Vec::new(),
                    );
                }
                DeclKind::Example(example) => {
                    let text = text_of(&example.name);
                    match examples.get(&text) {
                        Some(&first) => self.duplicate(
                            example.name.span,
                            first,
                            format!(
                                "{} \"{text}\" is defined twice in this module",
                                self.edition(example.name.span.file).test_word()
                            ),
                        ),
                        None => {
                            examples.insert(text, example.name.span);
                        }
                    }
                }
            }
        }
        table
    }

    fn declare(
        &mut self,
        table: &mut Table<'a>,
        ident: &Ident,
        what: What,
        public: bool,
        start: usize,
        variants: Vec<&'a str>,
    ) {
        let name = self.name(ident);
        // The names built in by the edition of the module that declares it (ADR 0122 rule 49).
        let edition = self.edition(ident.span.file);
        let upper = matches!(
            what,
            What::Kind | What::Variant | What::Capability | What::Measure
        );
        if upper {
            self.upper_convention(ident, what.word());
        } else {
            self.lower_convention(ident, what.word());
        }
        // Edition 0.2: the fixed-width integer types are built in (`spec/18 §1` rule 1).
        if upper && edition >= Edition::V0_2 && IntType::from_name(name).is_some() {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`{name}` is a built-in fixed-width integer type from edition 0.2, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.5: `Decimal` is built in (`spec/29 §16`, ADR 0120 rule 3).
        if upper && edition >= Edition::V0_5 && name == "Decimal" {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`Decimal` is a built-in decimal type from edition 0.5, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.3: `F64` is built in (`spec/29`, ADR 0114 rule 1.4).
        if upper && edition >= Edition::V0_3 && name == "F64" {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`F64` is a built-in floating-point type from edition 0.3, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.5: the dense types are built in, and `Tensor` and `Lu` are reserved for
        // them (`spec/29 §17`, ADR 0120 rule 3).
        if upper && edition >= Edition::V0_5 && matches!(name, "Array" | "Matrix" | "Tensor" | "Lu")
        {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`{name}` is a built-in dense type from edition 0.5, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.5: the other types of the numeric foundation are built in (`spec/29`, ADR
        // 0120 rule 3), as `F64` is; `Decimal` and the dense types have their own messages above.
        if upper && edition >= Edition::V0_5 && matches!(name, "F32" | "Complex") {
            let mut diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`{name}` is a built-in numeric type from edition 0.5, so a {} cannot take its name",
                    what.word()
                ),
            );
            diagnostic.repairs.push(Repair {
                rank: 1,
                escape: None,
                description: format!("rename the declaration, for example to `{name}Kind`"),
                edit: None,
                widens: None,
            });
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.2: the built-in types, and the types of the prelude that every program
        // uses, cannot be shadowed by a declaration (`spec/03 §1`, ADR 0079): in edition 0.1 a
        // kind named `Int` silently replaced it (#130).
        if upper
            && edition >= Edition::V0_2
            && matches!(
                name,
                "Int" | "Nat" | "Bool" | "Text" | "Unit" | "Hashed" | "List" | "Option" | "Result"
            )
        {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`{name}` is a built-in type from edition 0.2, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.2: maps and their entries are built in (`spec/21 §1` rule 1).
        if upper && edition >= Edition::V0_2 && matches!(name, "Map" | "Entry") {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`{name}` is a built-in type of maps from edition 0.2, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.2: the kinds of JSON are names of the prelude (`spec/23 §1` rule 2); their
        // variants may be shadowed, as `Some` may.
        if upper
            && edition >= Edition::V0_2
            && matches!(
                name,
                "Json" | "JsonNumber" | "JsonError" | "Http" | "Header" | "Response" | "HttpError"
            )
        {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`{name}` is a name of the prelude from edition 0.2, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        // Edition 0.2: the constraints are names of the prelude (`spec/20 §2` rule 3).
        if upper && edition >= Edition::V0_2 && matches!(name, "Eq" | "Ord" | "Integer") {
            let diagnostic = self.make(
                Code::U1002,
                ident.span,
                format!(
                    "`{name}` is a built-in constraint from edition 0.2, so a {} cannot take its name",
                    what.word()
                ),
            );
            self.diagnostics.push(diagnostic);
        }
        let names = if upper {
            &mut table.upper
        } else {
            &mut table.lower
        };
        if let Some(first) = names.get(name) {
            let (first_span, first_what) = (first.span, first.what);
            self.duplicate(
                ident.span,
                first_span,
                // R-034: the construct is named once when both definitions are of one kind
                // (D-158).
                if first_what == what {
                    format!(
                        "`{name}` is defined twice in this module, both times as a {}",
                        what.word()
                    )
                } else {
                    format!(
                        "`{name}` is defined twice in this module: as a {} and as a {}",
                        first_what.word(),
                        what.word()
                    )
                },
            );
            return;
        }
        names.insert(
            name,
            Declared {
                what,
                public,
                span: ident.span,
                start,
                variants,
            },
        );
    }

    /// U1009: a module path whose first segment is `std` or `prelude` is reserved for the
    /// standard library, in a package and in a single file (ADR 0038 D7, `spec/10 §11` rule 8).
    fn reserved_module(&mut self, header: &una_syntax::ast::ModuleHeader) {
        let Some(first) = header.path.segments.first() else {
            return;
        };
        let segment = self.name(first);
        if !prelude::RESERVED_SEGMENTS.contains(&segment) {
            return;
        }
        let names: Vec<&str> = header
            .path
            .segments
            .iter()
            .map(|segment| self.name(segment))
            .collect();
        let path = names.join(".");
        self.error(
            Code::U1009,
            header.span,
            format!(
                "module `{path}` starts with `{segment}`, a segment that only the standard library may use"
            ),
        );
    }

    /// U1008: `src/a/b.una` holds `module a.b` (packages only, ADR 0021 rule 1).
    fn module_path(&mut self, module: &ModuleInput<'a>, path_span: Span) {
        if self.package.single {
            return;
        }
        let file = self.package.sources.file(module.id).path();
        // A dependency's modules are under its own `src/` (ADR 0122 rule 47).
        let settings: &'a Manifest = self.package.settings;
        let src = settings
            .mount_of(file)
            .map_or("src/", |mount| mount.src.as_str());
        let expected = file
            .strip_prefix(src)
            .and_then(|rest| rest.strip_suffix(".una"))
            .map(|rest| rest.replace('/', "."))
            .unwrap_or_default();
        let declared = self
            .package
            .sources
            .file(module.id)
            .text()
            .get(path_span.start..path_span.end)
            .unwrap_or("")
            .to_owned();
        if declared == expected {
            return;
        }
        let message = if is_module_path(&expected) {
            format!("module `{declared}` is in `{file}`, which must hold module `{expected}`")
        } else {
            format!(
                "`{file}` cannot hold a module: file and directory names must be lower-case module path segments"
            )
        };
        let mut diagnostic = self.make(Code::U1008, path_span, message);
        if is_module_path(&expected) {
            diagnostic.repairs.push(self.edit(
                format!("declare `module {expected}`, or move the file to match `{declared}`"),
                path_span,
                expected,
            ));
        }
        self.diagnostics.push(diagnostic);
    }

    /// U1010: a module of the root whose first path segment is a dependency's local name, so
    /// its path could be a dependency's (ADR 0122 rule 47). O(m) for m modules.
    fn local_name_clashes(&mut self, tables: &[Table<'a>]) {
        let settings: &'a Manifest = self.package.settings;
        if settings.mounts.is_empty() {
            return;
        }
        for table in tables {
            let (Some(path), Some(header)) = (&table.path, &table.module.tree.module) else {
                continue;
            };
            if self.mount(table.module.id).is_some() {
                continue;
            }
            let first = path.split('.').next().unwrap_or("");
            if settings.mounts.iter().any(|mount| mount.local == first) {
                self.error(
                    Code::U1010,
                    header.path.span,
                    format!(
                        "module `{path}` starts with `{first}`, the local name of a dependency; give the need another local name with `as`"
                    ),
                );
            }
        }
    }

    /// The modules by path; a path declared by two modules is U1002.
    fn module_paths(&mut self, tables: &[Table<'a>]) -> BTreeMap<String, usize> {
        let mut paths: BTreeMap<String, usize> = BTreeMap::new();
        for (index, table) in tables.iter().enumerate() {
            let (Some(path), Some(header)) = (&table.path, &table.module.tree.module) else {
                continue;
            };
            match paths.get(path) {
                Some(&first) => {
                    let first_span = tables
                        .get(first)
                        .and_then(|first| first.module.tree.module.as_ref())
                        .map_or(header.path.span, |first| first.path.span);
                    self.duplicate(
                        header.path.span,
                        first_span,
                        format!("module `{path}` is declared by two files"),
                    );
                }
                None => {
                    paths.insert(path.clone(), index);
                }
            }
        }
        paths
    }

    /// Requirements: unique identifiers (U1002), their conventions (U1005), and generated
    /// requirements that are not proposed (U1007), across the package.
    fn requirements(&mut self) {
        let mut first: BTreeMap<&'a str, Span> = BTreeMap::new();
        // Every requirement identifier, and where each is named by a `derived from` clause,
        // for the U1005 repair (#743).
        let mut ids: BTreeSet<&'a str> = BTreeSet::new();
        let mut cited: BTreeMap<&'a str, Vec<Span>> = BTreeMap::new();
        for module in &self.package.modules {
            for decl in &module.tree.decls {
                match &decl.kind {
                    DeclKind::Requirement(requirement) => {
                        ids.insert(self.name(&requirement.id));
                    }
                    DeclKind::Function(function) => {
                        for clause in &function.clauses {
                            if let ClauseKind::DerivedFrom(requirements) = &clause.kind {
                                for requirement in requirements {
                                    cited
                                        .entry(self.name(requirement))
                                        .or_default()
                                        .push(requirement.span);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut names = None;
        for module in &self.package.modules {
            for decl in &module.tree.decls {
                let DeclKind::Requirement(requirement) = &decl.kind else {
                    continue;
                };
                let id = self.name(&requirement.id);
                if !conventions::requirement(id) {
                    // Read once, on the first identifier that breaks the convention (#1474).
                    let names =
                        names.get_or_insert_with(|| conventions::RequirementNames::new(&ids));
                    self.requirement_convention(requirement.id.span, id, names, cited.get(id));
                }
                self.lower_convention(&requirement.owner, "owner");
                match first.get(id) {
                    Some(&earlier) => self.duplicate(
                        requirement.id.span,
                        earlier,
                        format!("requirement `{id}` is defined twice in this package"),
                    ),
                    None => {
                        first.insert(id, requirement.id.span);
                    }
                }
                if matches!(requirement.source, RequirementSource::Generated(_))
                    && requirement.active
                {
                    let mut diagnostic = self.make(
                        Code::U1007,
                        requirement.status,
                        format!("requirement `{id}` has source `generated`, so its status must be `proposed`"),
                    );
                    diagnostic.repairs.push(
                        self.edit(
                            "mark it `proposed`; a person activates it by changing its source"
                                .to_owned(),
                            requirement.status,
                            "proposed".to_owned(),
                        ),
                    );
                    self.diagnostics.push(diagnostic);
                }
            }
        }
    }

    /// The requirement identifiers of the package.
    fn requirement_ids(&self) -> BTreeSet<&'a str> {
        self.package
            .modules
            .iter()
            .flat_map(|module| &module.tree.decls)
            .filter_map(|decl| match &decl.kind {
                DeclKind::Requirement(requirement) => Some(self.name(&requirement.id)),
                _ => None,
            })
            .collect()
    }

    /// The imports of the module `index` (ADR 0023 rule 5).
    fn imports(&mut self, tables: &mut [Table<'a>], index: usize, paths: &BTreeMap<String, usize>) {
        let Some(tree) = tables.get(index).map(|table| table.module.tree) else {
            return;
        };
        for import in &tree.imports {
            let names: Vec<&str> = import
                .module
                .segments
                .iter()
                .map(|segment| self.name(segment))
                .collect();
            let path = names.join(".");
            // A dependency's module imports its own modules by their own paths, and the
            // standard modules; never a module of the root (ADR 0122 rule 47).
            let mount = tables
                .get(index)
                .and_then(|table| self.mount(table.module.id));
            let is_standard = path == prelude::STD_TESTING || path == prelude::STD_HOST;
            let target = match mount {
                Some(local) if !is_standard => paths.get(&format!("{local}.{path}")).copied(),
                _ => paths.get(&path).copied(),
            };
            if let Some(target) = target
                && let Some(table) = tables.get_mut(index)
            {
                table.edges.push((target, import.module.span));
            }
            // The importable standard modules (`spec/10 §11` rule 10), `std.collections` among
            // them from edition 0.5 (ADR 0119 rule 0).
            let edition = self.edition(import.module.span.file);
            let importable = prelude::importable(edition);
            let is_std = importable.contains(&path.as_str());
            let standard: &[&str] = match path.as_str() {
                _ if !is_std => &[],
                prelude::STD_TESTING => prelude::std_testing_names(edition),
                prelude::STD_HOST => prelude::std_host_names(edition),
                prelude::STD_TIME => &prelude::STD_TIME_NAMES,
                prelude::STD_DRIVE => &prelude::STD_DRIVE_NAMES,
                prelude::STD_COLLECTIONS => &prelude::STD_COLLECTIONS_NAMES,
                prelude::STD_LIMIT => &prelude::STD_LIMIT_NAMES,
                _ => &[],
            };
            if target.is_none() && !is_std {
                let listed: Vec<String> = importable.iter().map(|m| format!("`{m}`")).collect();
                let listed = match listed.split_last() {
                    Some((last, rest)) if !rest.is_empty() => {
                        format!("{} and {last}", rest.join(", "))
                    }
                    _ => listed.join(""),
                };
                let mut diagnostic = self.make(
                    Code::U1001,
                    import.module.span,
                    format!(
                        "module `{path}` is not a module of this package; the standard modules that can be imported are {listed}"
                    ),
                );
                // ADR 0139 rule 7: `std.collections` moved to edition 0.5.
                if edition == Edition::V0_4 && prelude::moved_to_0_5(&path) {
                    diagnostic
                        .repairs
                        .push(Checker::advice(prelude::moved_to_0_5_advice(&path)));
                }
                self.diagnostics.push(diagnostic);
                continue;
            }
            for ident in &import.names {
                let name = self.name(ident);
                let found = match target.and_then(|target| tables.get(target)) {
                    // A module with syntax errors may lack the name.
                    Some(module) if module.module.broken => Found::Unknown,
                    Some(module) => match module.upper.get(name).or_else(|| module.lower.get(name))
                    {
                        Some(declared) => Found::Declared(declared.clone(), module.module.id),
                        None if self.poisoned(module.module.id, name) => Found::Unknown,
                        None => Found::Missing,
                    },
                    None if standard.contains(&name) => Found::Standard,
                    None => Found::Missing,
                };
                match found {
                    Found::Missing => {
                        self.error(
                            Code::U1001,
                            ident.span,
                            format!("module `{path}` declares no `{name}`"),
                        );
                    }
                    Found::Declared(declared, file) if !declared.public => {
                        self.private(ident.span, name, &path, &declared, file);
                    }
                    Found::Declared(declared, _) => {
                        if declared.what == What::Kind
                            && let Some(table) = tables.get_mut(index)
                        {
                            table
                                .imported_variants
                                .extend(declared.variants.iter().copied());
                        }
                    }
                    // ADR 0123 rule 1: an import of a sum kind of the server brings its variants,
                    // as an import of a kind of the package does (ADR 0023 rule 2).
                    Found::Standard if path == prelude::STD_HOST => {
                        if let Some(variants) = prelude::server_variants(name)
                            && let Some(table) = tables.get_mut(index)
                        {
                            table.imported_variants.extend(variants.iter().copied());
                        }
                    }
                    Found::Standard | Found::Unknown => {}
                }
                self.import_name(tables, index, ident, name);
            }
        }
    }

    /// U1004 with an edit that makes the declaration public.
    fn private(
        &mut self,
        span: Span,
        name: &str,
        path: &str,
        declared: &Declared<'a>,
        file: FileId,
    ) {
        let mut diagnostic = self.make(
            Code::U1004,
            span,
            format!(
                "{} `{name}` is private to module `{path}`",
                declared.what.word()
            ),
        );
        diagnostic
            .related
            .push(self.related(declared.span, "declared here without `pub`"));
        // Exporting a declaration widens what other modules may call: `una fix` never applies
        // this edit (ADR 0084).
        let mut repair = self.edit(
            format!("declare `{name}` with `pub`"),
            Span::at(file, declared.start),
            "pub ".to_owned(),
        );
        repair.widens = Some(Widening::Visibility);
        diagnostic.repairs.push(repair);
        self.diagnostics.push(diagnostic);
    }

    /// Records an imported name; U1002 when it repeats a declaration or another import.
    fn import_name(
        &mut self,
        tables: &mut [Table<'a>],
        index: usize,
        ident: &Ident,
        name: &'a str,
    ) {
        let Some(table) = tables.get_mut(index) else {
            return;
        };
        let clash = table
            .upper
            .get(name)
            .or_else(|| table.lower.get(name))
            .map(|declared| declared.span)
            .or_else(|| table.imported.get(name).copied());
        match clash {
            Some(first) => self.duplicate(
                ident.span,
                first,
                format!("`{name}` is already defined in this module"),
            ),
            None => {
                table.imported.insert(name, ident.span);
            }
        }
    }

    /// U1003, once per import cycle, at the first import of the cycle's first module in path
    /// order.
    fn cycles(&mut self, tables: &[Table<'a>]) {
        let components = strongly_connected(tables);
        for component in components {
            let cyclic = component.len() > 1
                || component.first().is_some_and(|&only| {
                    tables
                        .get(only)
                        .is_some_and(|table| table.edges.iter().any(|(target, _)| *target == only))
                });
            if !cyclic {
                continue;
            }
            let Some(&first) = component
                .iter()
                .min_by_key(|&&index| tables.get(index).and_then(|table| table.path.clone()))
            else {
                continue;
            };
            let Some(table) = tables.get(first) else {
                continue;
            };
            let Some(&(_, span)) = table
                .edges
                .iter()
                .find(|(target, _)| component.contains(target))
            else {
                continue;
            };
            let mut names: Vec<String> = component
                .iter()
                .filter_map(|&index| tables.get(index).and_then(|table| table.path.clone()))
                .collect();
            names.sort();
            let message = match names.as_slice() {
                [only] => format!("module `{only}` imports itself"),
                _ => format!("import cycle between modules {}", quoted(&names)),
            };
            self.error(Code::U1003, span, message);
        }
    }

    /// U7005: trusted headers and `[trusted] modules` agree (`spec/07 §6`); U7002 for an
    /// `[run] entry` that is not a module of the package.
    fn trusted(&mut self, tables: &[Table<'a>], paths: &BTreeMap<String, usize>) {
        let settings = self.package.settings;
        // How the messages name the list of trusted modules of each manifest.
        let (listing, list_in, quoted_list) = match settings.format {
            una_project::Format::Toml => (
                "una.toml [trusted] modules",
                "under `[trusted] modules` in `una.toml`",
                "`[trusted] modules`",
            ),
            una_project::Format::Ita => (
                "the `trusted` section of ita.una",
                "under `trusted` in `ita.una`",
                "`trusted` in `ita.una`",
            ),
        };
        let list_name = match settings.format {
            una_project::Format::Toml => "[trusted] modules",
            una_project::Format::Ita => "the `trusted` section",
        };
        for table in tables {
            let (Some(header), Some(path)) = (&table.module.tree.module, &table.path) else {
                continue;
            };
            let listed = settings.trusted.iter().any(|entry| entry.value == *path);
            if header.trusted && !listed {
                let keyword = Span::new(
                    header.span.file,
                    header.span.start,
                    header.span.start + "trusted".len(),
                );
                // A single file has no una.toml to list the module in.
                let message = if self.package.single {
                    format!(
                        "module `{path}` is `trusted`, but a file checked on its own has no una.toml to list it in [trusted] modules"
                    )
                } else {
                    format!(
                        "module `{path}` is `trusted` in its header but not listed in {listing}"
                    )
                };
                // ADR 0095: removing `trusted` keeps the trusted base, since a policy outside
                // a trusted module is U4004; listing the module widens it, so it comes last.
                let mut diagnostic = self.make(Code::U7005, keyword, message);
                diagnostic.repairs.push(Self::advice(format!(
                    "remove `trusted` from the header of `{path}`{}",
                    then_u4004(table)
                )));
                if !self.package.single {
                    diagnostic.repairs.push(Self::escape(
                        Escape::Trusted,
                        format!("list `{path}` {list_in}"),
                    ));
                }
                self.diagnostics.push(diagnostic);
            }
        }
        let Some(manifest) = self.package.manifest else {
            return;
        };
        for entry in &settings.trusted {
            let span = Span::new(manifest, entry.range.start, entry.range.end);
            match paths.get(&entry.value).and_then(|&index| tables.get(index)) {
                Some(table) => {
                    let trusted = table
                        .module
                        .tree
                        .module
                        .as_ref()
                        .is_some_and(|header| header.trusted);
                    if !trusted {
                        // ADR 0095: removing the entry keeps the trusted base; adding
                        // `trusted` to the header widens it, so it comes last.
                        let mut diagnostic = self.make(
                            Code::U7005,
                            span,
                            format!("{list_name} lists `{}`, whose header does not say `trusted module`", entry.value),
                        );
                        diagnostic.repairs.push(Self::advice(format!(
                            "remove `{}` from {quoted_list}{}",
                            entry.value,
                            then_u4004(table)
                        )));
                        diagnostic.repairs.push(Self::escape(
                            Escape::Trusted,
                            format!(
                                "write `trusted module {}` in the header of `{}`",
                                entry.value, entry.value
                            ),
                        ));
                        self.diagnostics.push(diagnostic);
                    }
                }
                None => {
                    let mut diagnostic = self.make(
                        Code::U7005,
                        span,
                        format!(
                            "{list_name} lists `{}`, which is not a module of this package",
                            entry.value
                        ),
                    );
                    diagnostic.repairs.push(Self::advice(format!(
                        "remove `{}` from {quoted_list}",
                        entry.value
                    )));
                    self.diagnostics.push(diagnostic);
                }
            }
        }
        // #743: when the entry's own file is there but its module header did not parse, as with
        // `module when`, the syntax error at the header says why; a second error here would
        // only repeat it.
        let unparsed_entry = |entry: &str| {
            let file = format!("src/{}.una", entry.replace('.', "/"));
            self.package.modules.iter().any(|module| {
                module.tree.module.is_none() && self.package.sources.file(module.id).path() == file
            })
        };
        if let Some(entry) = &settings.entry
            && !paths.contains_key(&entry.value)
            && !unparsed_entry(&entry.value)
        {
            self.error(
                settings.format.code(),
                Span::new(manifest, entry.range.start, entry.range.end),
                format!(
                    "{} `{}` is not a module of this package",
                    settings.format.section("[run] entry"),
                    entry.value
                ),
            );
        }
        // ADR 0116 rule 1: the `drive` section's module is a module of the package, as `run
        // entry` is.
        if let Some(drive) = &settings.drive
            && !paths.contains_key(&drive.module.value)
            && !unparsed_entry(&drive.module.value)
        {
            self.error(
                settings.format.code(),
                Span::new(manifest, drive.module.range.start, drive.module.range.end),
                format!(
                    "{} `{}` is not a module of this package",
                    settings.format.section("[drive] module"),
                    drive.module.value
                ),
            );
        }
        // ADR 0123 rule 16: the `serve` section's module is a module of the package, as `run
        // entry` is.
        if let Some(serve) = &settings.serve
            && !paths.contains_key(&serve.module.value)
            && !unparsed_entry(&serve.module.value)
        {
            self.error(
                settings.format.code(),
                Span::new(manifest, serve.module.range.start, serve.module.range.end),
                format!(
                    "{} `{}` is not a module of this package",
                    settings.format.section("[serve] module"),
                    serve.module.value
                ),
            );
        }
    }
}

/// The text of a literal without interpolations, for example names.
fn text_of(text: &una_syntax::ast::TextLit) -> String {
    text.parts
        .iter()
        .map(|part| match part {
            una_syntax::ast::TextPart::Text(chunk) => chunk.clone(),
            una_syntax::ast::TextPart::Interpolation(_) => "{…}".to_owned(),
        })
        .collect()
}

fn quoted(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The strongly connected components of the import graph (Tarjan), each in module order.
fn strongly_connected(tables: &[Table<'_>]) -> Vec<Vec<usize>> {
    struct State {
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        components: Vec<Vec<usize>>,
    }
    fn visit(tables: &[Table<'_>], node: usize, state: &mut State) {
        let set = |values: &mut Vec<usize>, at: usize, value: usize| {
            if let Some(slot) = values.get_mut(at) {
                *slot = value;
            }
        };
        if let Some(slot) = state.index.get_mut(node) {
            *slot = Some(state.next);
        }
        set(&mut state.low, node, state.next);
        state.next += 1;
        state.stack.push(node);
        if let Some(slot) = state.on_stack.get_mut(node) {
            *slot = true;
        }
        let targets: Vec<usize> = tables
            .get(node)
            .map(|table| table.edges.iter().map(|(target, _)| *target).collect())
            .unwrap_or_default();
        for target in targets {
            match state.index.get(target).copied().flatten() {
                None => {
                    visit(tables, target, state);
                    let low = state.low.get(target).copied().unwrap_or(0);
                    let mine = state.low.get(node).copied().unwrap_or(0);
                    set(&mut state.low, node, mine.min(low));
                }
                Some(target_index) if state.on_stack.get(target).copied().unwrap_or(false) => {
                    let mine = state.low.get(node).copied().unwrap_or(0);
                    set(&mut state.low, node, mine.min(target_index));
                }
                Some(_) => {}
            }
        }
        if state.low.get(node) == state.index.get(node).copied().flatten().as_ref() {
            let mut component = Vec::new();
            while let Some(top) = state.stack.pop() {
                if let Some(slot) = state.on_stack.get_mut(top) {
                    *slot = false;
                }
                component.push(top);
                if top == node {
                    break;
                }
            }
            component.sort_unstable();
            state.components.push(component);
        }
    }
    let count = tables.len();
    let mut state = State {
        index: vec![None; count],
        low: vec![0; count],
        on_stack: vec![false; count],
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for node in 0..count {
        if state.index.get(node).copied().flatten().is_none() {
            visit(tables, node, &mut state);
        }
    }
    state.components.sort();
    state.components
}

/// What narrowing trust leaves to do when the module of `table` declares a policy: the policy is
/// then U4004, so the repair does not close the problem alone (ADR 0095, rule 6).
fn then_u4004(table: &Table<'_>) -> &'static str {
    let policies = table
        .module
        .tree
        .decls
        .iter()
        .any(|decl| matches!(decl.kind, DeclKind::Policy(_)));
    if policies {
        "; its policies are then U4004: move them into a trusted module"
    } else {
        ""
    }
}
