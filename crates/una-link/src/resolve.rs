//! Symbol resolution (ADR 0124 rule 23): the program's object first, then the members of the
//! runtime archive that undefined symbols demand, to a fixed point; duplicates, weak definitions,
//! the linker-defined `__mh_execute_header`, and what stays undefined.

use std::collections::{BTreeMap, VecDeque};

use crate::input::{Archive, Object, nlist, read_object};
use crate::problem::{Problem, Result, shown};

/// The one symbol the linker defines on Mach-O.
pub(crate) const HEADER_SYMBOL: &[u8] = b"__mh_execute_header";

/// What a global name resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Global {
    /// Defined by `symbol` of `object`.
    Defined {
        /// The object's index in [`Resolved::objects`].
        object: usize,
        /// The symbol's index in that object's symbol table.
        symbol: usize,
        /// Whether the definition is weak.
        weak: bool,
    },
    /// Defined by no input: bound to libSystem if its export list holds it.
    Undefined,
    /// `__mh_execute_header`, the address of the Mach-O header.
    Header,
}

/// The loaded objects and the global symbol table.
pub(crate) struct Resolved<'a> {
    /// The program's object, then the loaded members, in the order they were loaded.
    pub(crate) objects: Vec<Object<'a>>,
    /// The placement order of the objects: the program's first, then the members in archive
    /// order, so that the layout does not depend on the order of loading.
    pub(crate) order: Vec<usize>,
    /// Every global name and what it resolves to.
    pub(crate) globals: BTreeMap<&'a [u8], Global>,
}

/// Resolves the program's object against the archive.
///
/// # Errors
///
/// U8001 on a member that cannot be read, a duplicate strong definition, a definition of
/// `__mh_execute_header`, an absolute or common symbol, or an archive symbol table that names a
/// member that does not define the symbol.
pub(crate) fn resolve<'a>(program: Object<'a>, archive: &Archive<'a>) -> Result<Resolved<'a>> {
    let mut resolved = Resolved {
        objects: Vec::new(),
        order: Vec::new(),
        globals: BTreeMap::new(),
    };
    let mut pending: VecDeque<&'a [u8]> = VecDeque::new();
    let mut keys: Vec<u64> = vec![0];
    resolved.objects.push(program);
    add_symbols(&mut resolved, 0, &keys, &mut pending)?;
    let mut loaded: BTreeMap<u64, usize> = BTreeMap::new();
    while let Some(name) = pending.pop_front() {
        if resolved.globals.get(name) != Some(&Global::Undefined) {
            continue;
        }
        let Some(offset) = archive.member_of(name) else {
            continue;
        };
        if loaded.contains_key(&offset) {
            return Err(Problem::internal(format!(
                "the symbol table of `{}` names a member for `{}` that does not define it",
                archive.name,
                shown(name)
            )));
        }
        let (member_name, bytes) = archive.member(offset)?;
        let object = read_object(&member_name, bytes)?;
        let index = resolved.objects.len();
        resolved.objects.push(object);
        keys.push(offset.saturating_add(1));
        loaded.insert(offset, index);
        add_symbols(&mut resolved, index, &keys, &mut pending)?;
        if resolved.globals.get(name) == Some(&Global::Undefined) {
            return Err(Problem::internal(format!(
                "the symbol table of `{}` names a member for `{}` that does not define it",
                archive.name,
                shown(name)
            )));
        }
    }
    if let Some(header) = resolved.globals.get_mut(HEADER_SYMBOL)
        && *header == Global::Undefined
    {
        *header = Global::Header;
    }
    let mut order: Vec<usize> = (0..resolved.objects.len()).collect();
    order.sort_by_key(|index| keys.get(*index).copied().unwrap_or(u64::MAX));
    resolved.order = order;
    Ok(resolved)
}

fn add_symbols<'a>(
    resolved: &mut Resolved<'a>,
    index: usize,
    keys: &[u64],
    pending: &mut VecDeque<&'a [u8]>,
) -> Result<()> {
    let Some(object) = resolved.objects.get(index) else {
        return Ok(());
    };
    let name_of = |object: usize| {
        resolved
            .objects
            .get(object)
            .map_or_else(String::new, |o| o.name.clone())
    };
    let mut updates: BTreeMap<&'a [u8], Global> = BTreeMap::new();
    for (position, symbol) in object.symbols.iter().enumerate() {
        if !symbol.external() {
            continue;
        }
        if symbol.name == HEADER_SYMBOL && !symbol.undefined() {
            return Err(Problem::internal(format!(
                "`{}` defines `__mh_execute_header`, which only the linker defines",
                object.name
            )));
        }
        if symbol.in_section() {
            let weak = symbol.weak();
            let current = updates
                .get(symbol.name)
                .copied()
                .or_else(|| resolved.globals.get(symbol.name).copied());
            let new = Global::Defined {
                object: index,
                symbol: position,
                weak,
            };
            match current {
                None | Some(Global::Undefined) => {
                    updates.insert(symbol.name, new);
                }
                Some(Global::Defined {
                    object: other,
                    symbol: other_symbol,
                    weak: other_weak,
                }) => {
                    let other_private = resolved
                        .objects
                        .get(other)
                        .and_then(|o| o.symbols.get(other_symbol))
                        .is_some_and(|s| s.n_type & nlist::N_PEXT != 0);
                    let private = symbol.n_type & nlist::N_PEXT != 0;
                    if !other_weak && !weak && other_private && private {
                        // Two private-extern copies of one name: a generic instance that two
                        // codegen units of one crate both emit, identical by its mangled name.
                        // The first in input order is kept, as ld64 keeps it.
                        if keys.get(index) < keys.get(other) {
                            updates.insert(symbol.name, new);
                        }
                        continue;
                    }
                    if !other_weak && !weak {
                        return Err(Problem::internal(format!(
                            "the symbol `{}` is defined twice, in `{}` and in `{}`",
                            shown(symbol.name),
                            name_of(other),
                            object.name
                        )));
                    }
                    if other_weak && !weak {
                        updates.insert(symbol.name, new);
                    }
                }
                Some(Global::Header) => {}
            }
        } else if symbol.undefined() {
            if symbol.n_value != 0 {
                return Err(Problem::internal(format!(
                    "`{}` declares `{}` as a common symbol, which the internal linker does not support",
                    object.name,
                    shown(symbol.name)
                )));
            }
            if !resolved.globals.contains_key(symbol.name) && !updates.contains_key(symbol.name) {
                updates.insert(symbol.name, Global::Undefined);
                pending.push_back(symbol.name);
            }
        } else {
            return Err(Problem::internal(format!(
                "`{}` defines `{}` with a symbol type ({:#x}) that the internal linker does not support",
                object.name,
                shown(symbol.name),
                symbol.n_type
            )));
        }
    }
    for (name, global) in updates {
        resolved.globals.insert(name, global);
    }
    Ok(())
}
