//! Names (M3, ADR 0023): module paths (U1008), declarations and duplicates (U1002), imports
//! (U1001, U1003, U1004), every name of every body (U1001), naming conventions (U1005),
//! requirements (U1002, U1006, U1007) and trusted modules (U7005).

mod conventions;
mod prelude;
mod resolve;

use std::collections::{BTreeMap, BTreeSet};

use una_diag::{Code, Diagnostic, Edit, Related, Repair};
use una_project::{Manifest, is_module_path};
use una_syntax::ast::{DeclKind, File, Ident, KindBody, RequirementSource};
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
}

/// What `una run` adds to a check: the module whose `main` runs, and the grants of
/// `[run] grants` and `--grant` together, which are final (`spec/04 §8`, DECISION D-109).
#[derive(Clone, Copy, Debug)]
pub struct RunInput<'a> {
    /// The path of the entry module.
    pub entry: &'a str,
    /// The grant specs.
    pub grants: &'a [String],
}

/// The name diagnostics of `package`.
pub fn check_names(package: &PackageInput<'_>) -> Vec<Diagnostic> {
    let mut checker = Checker {
        package,
        diagnostics: Vec::new(),
    };
    let mut tables: Vec<Table<'_>> = package
        .modules
        .iter()
        .map(|module| checker.declarations(module))
        .collect();
    let paths = checker.module_paths(&tables);
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
    checker.diagnostics
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
}

impl<'p, 'a> Checker<'p, 'a> {
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
            rank: 1,
            escape_cost: 0,
            description,
            edit: None,
        }
    }

    /// A repair that replaces `span` with `replacement`.
    fn edit(&self, description: String, span: Span, replacement: String) -> Repair {
        Repair {
            rank: 1,
            escape_cost: 0,
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
                names.join(".")
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
                DeclKind::Example(example) => {
                    let text = text_of(&example.name);
                    match examples.get(&text) {
                        Some(&first) => self.duplicate(
                            example.name.span,
                            first,
                            format!("example \"{text}\" is defined twice in this module"),
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
        let upper = matches!(what, What::Kind | What::Variant | What::Capability);
        if upper {
            self.upper_convention(ident, what.word());
        } else {
            self.lower_convention(ident, what.word());
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
                // (DECISION D-158).
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
        let expected = file
            .strip_prefix("src/")
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
        for module in &self.package.modules {
            for decl in &module.tree.decls {
                let DeclKind::Requirement(requirement) = &decl.kind else {
                    continue;
                };
                let id = self.name(&requirement.id);
                if !conventions::requirement(id) {
                    self.convention(
                        requirement.id.span,
                        "requirement identifier",
                        id,
                        "must be upper snake case ending with `_` and a number",
                        "EVENTS_3",
                    );
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
            let target = paths.get(&path).copied();
            if let Some(target) = target
                && let Some(table) = tables.get_mut(index)
            {
                table.edges.push((target, import.module.span));
            }
            let is_std = path == prelude::STD_TESTING;
            if target.is_none() && !is_std {
                self.error(
                    Code::U1001,
                    import.module.span,
                    format!(
                        "module `{path}` is not a module of this package or of the standard library"
                    ),
                );
                continue;
            }
            for ident in &import.names {
                let name = self.name(ident);
                let found = match target.and_then(|target| tables.get(target)) {
                    // DECISION D-041: a module with syntax errors may lack the name.
                    Some(module) if module.module.broken => Found::Unknown,
                    Some(module) => match module.upper.get(name).or_else(|| module.lower.get(name))
                    {
                        Some(declared) => Found::Declared(declared.clone(), module.module.id),
                        None => Found::Missing,
                    },
                    None if prelude::STD_TESTING_NAMES.contains(&name) => Found::Standard,
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
                    Found::Standard | Found::Unknown => {}
                }
                self.import_name(tables, index, ident, name);
            }
        }
    }

    /// U1004 with an edit that makes the declaration public (DECISION D-040).
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
        diagnostic.repairs.push(self.edit(
            format!("declare `{name}` with `pub`"),
            Span::at(file, declared.start),
            "pub ".to_owned(),
        ));
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
    /// order (DECISION D-040).
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
                // DECISION D-045: a single file has no una.toml to list the module in.
                let message = if self.package.single {
                    format!(
                        "module `{path}` is `trusted`, but a file checked on its own has no una.toml to list it in [trusted] modules"
                    )
                } else {
                    format!(
                        "module `{path}` is `trusted` in its header but not listed in una.toml [trusted] modules"
                    )
                };
                self.error(Code::U7005, keyword, message);
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
                        self.error(
                            Code::U7005,
                            span,
                            format!("[trusted] modules lists `{}`, whose header does not say `trusted module`", entry.value),
                        );
                    }
                }
                None => {
                    self.error(
                        Code::U7005,
                        span,
                        format!(
                            "[trusted] modules lists `{}`, which is not a module of this package",
                            entry.value
                        ),
                    );
                }
            }
        }
        if let Some(entry) = &settings.entry
            && !paths.contains_key(&entry.value)
        {
            self.error(
                Code::U7002,
                Span::new(manifest, entry.range.start, entry.range.end),
                format!(
                    "[run] entry `{}` is not a module of this package",
                    entry.value
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
