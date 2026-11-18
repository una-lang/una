//! The guarantees of the checker's shared types (ADR 0077 stage 2): no code can change a type
//! node that another type shares, and no checker type leaves the thread that checks.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::Path;

/// Compiles only if `T` may move to another thread.
fn assert_send<T: Send>() {}

#[test]
fn the_result_of_a_check_holds_no_checker_type() {
    // `Shared` is `Rc`, so a type of the checker cannot cross a thread. `una-cli` checks a
    // package on its own thread and moves the result back: the result must stay `Send`, so no
    // checker type can be put in it.
    assert_send::<una_check::TypeCheck>();
}

#[test]
fn no_code_of_the_types_can_reach_a_shared_node_mutably() {
    // `Shared` has no mutable access; this guards against widening it later. Nowhere in the
    // types: `Rc::get_mut`, `Rc::make_mut`, `DerefMut`, `UnsafeCell`. In `ty.rs`, which defines
    // every node type (`Ty`, `FnTy`, `FnRefinements`, `Shared`): no interior mutability at all.
    // Comments are not code, so they may name these.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/types");
    let everywhere = ["Rc::get_mut", "Rc::make_mut", "DerefMut", "UnsafeCell"];
    let in_nodes = ["Cell<", "RefCell", "OnceCell", "Mutex", "RwLock", "Atomic"];
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let nodes = path.file_name().is_some_and(|name| name == "ty.rs");
        let text = std::fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let words = everywhere
                .iter()
                .chain(if nodes { &in_nodes[..] } else { &[] });
            for word in words {
                if code.contains(word) {
                    found.push(format!("{}:{}: {word}", path.display(), number + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn the_node_module_gives_no_mutable_access() {
    // ADR 0077 stage 2: `types/shared.rs` alone holds the `Rc` of a node, in a private field,
    // so no other module can reach it. Within it: no `&mut` but the `Formatter` of `Debug`,
    // `Rc` imported under its own name only, named in a type only by the struct's field and the
    // node a kind's arguments are collected into, and no `Rc` function but those that make,
    // clone or compare a handle, or give the address a memo keeps its handle for.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/types/shared.rs");
    let text = std::fs::read_to_string(&path).unwrap();
    let allowed = [
        "Rc::new(",
        "Rc::clone(",
        "Rc::from(",
        "Rc::ptr_eq(",
        "Rc::as_ptr(",
    ];
    let mut found = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let at = |what: &str| format!("shared.rs:{}: {what}: {line}", number + 1);
        if code.replace("&mut fmt::Formatter", "").contains("&mut") {
            found.push(at("&mut"));
        }
        if code.contains("use ") && code.contains("rc") && code.trim() != "use std::rc::Rc;" {
            found.push(at("an import of rc other than `use std::rc::Rc;`"));
        }
        let declares = ["    node: Rc<T>,", "        let node: Rc<[Ty]> ="];
        if code.contains("Rc<") && !declares.iter().any(|d| code.starts_with(d)) {
            found.push(at("a type that names Rc"));
        }
        let mut rest = code;
        while let Some(index) = rest.find("Rc::") {
            let call = &rest[index..];
            if !allowed.iter().any(|allowed| call.starts_with(allowed)) {
                found.push(at("an Rc function"));
            }
            rest = &rest[index + 4..];
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}
