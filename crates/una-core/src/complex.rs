//! `Complex[F64]` and `Complex[F32]` (`spec/29 §19`, ADR 0120 rules 22 to 25): two built-in
//! types, each a record of two components, `re` and `im`, of its component type. The prelude
//! of edition `0.4` declares them as the record kinds [`COMPLEX_F64`] and [`COMPLEX_F32`], which
//! no program can name: a program writes `Complex[F64]`, and builds a value with
//! `complex(re: x, im: y)`. Their operations are elaborated into the operations of their
//! components, so both engines and the verifier run them as the scalar operations they are.

use crate::ir::{Ctor, PRELUDE_MODULE};

/// The prelude's record kind of `Complex[F64]`.
pub const COMPLEX_F64: &str = "Complex_F64";

/// The prelude's record kind of `Complex[F32]`.
pub const COMPLEX_F32: &str = "Complex_F32";

/// The function that builds a complex value, and by which its Una form writes it (ADR 0120
/// rule 24).
pub const CONSTRUCTOR: &str = "complex";

/// Whether the kind `name` of `module` is one of the two complex kinds.
pub fn is_kind(module: &str, name: &str) -> bool {
    module == PRELUDE_MODULE && (name == COMPLEX_F64 || name == COMPLEX_F32)
}

/// How a program writes the type of the complex kind `name`: `Complex[F64]` or
/// `Complex[F32]`; `None` for any other name.
pub fn written(name: &str) -> Option<&'static str> {
    match name {
        COMPLEX_F64 => Some("Complex[F64]"),
        COMPLEX_F32 => Some("Complex[F32]"),
        _ => None,
    }
}

/// The name by which the Una form writes a value built by `ctor`: [`CONSTRUCTOR`] for a
/// complex value, as in `complex(re: 1.0, im: -2.0)`, and the constructor's own name otherwise.
pub fn constructor_name(ctor: &Ctor) -> &str {
    if is_kind(&ctor.kind.module, &ctor.kind.name) {
        CONSTRUCTOR
    } else {
        &ctor.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Path;

    #[test]
    fn complex_kinds_are_written_as_their_types() {
        assert_eq!(written(COMPLEX_F64), Some("Complex[F64]"));
        assert_eq!(written(COMPLEX_F32), Some("Complex[F32]"));
        assert_eq!(written("Header"), None);
        assert!(is_kind(PRELUDE_MODULE, COMPLEX_F64));
        assert!(!is_kind("m", COMPLEX_F64));
        let ctor = |module: &str, name: &str| Ctor {
            kind: Path {
                module: module.to_owned(),
                name: name.to_owned(),
            },
            name: name.to_owned(),
            index: 0,
        };
        assert_eq!(
            constructor_name(&ctor(PRELUDE_MODULE, COMPLEX_F32)),
            "complex"
        );
        assert_eq!(constructor_name(&ctor("m", COMPLEX_F32)), COMPLEX_F32);
    }
}
