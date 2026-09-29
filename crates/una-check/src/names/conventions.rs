//! Naming conventions (`spec/01 §4`, U1005). DECISION D-042: type parameters follow the
//! upper-case convention, and owners and module path segments the lower-case one; each name is
//! checked where it is defined.

/// Why a lower-case name breaks the convention, and a name that keeps it.
pub(crate) fn lower(name: &str) -> Option<(String, String)> {
    let problem = if name.contains("__") {
        "must not contain `__`"
    } else if name.ends_with('_') {
        "must not end with `_`"
    } else {
        return None;
    };
    let mut suggestion = String::new();
    for c in name.chars() {
        if !(c == '_' && (suggestion.is_empty() || suggestion.ends_with('_'))) {
            suggestion.push(c);
        }
    }
    let suggestion = suggestion.trim_end_matches('_').to_owned();
    Some((problem.to_owned(), suggestion))
}

/// Whether an upper-case name is UpperCamelCase, and a name that is.
pub(crate) fn upper(name: &str) -> Option<(String, String)> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric());
    if valid {
        return None;
    }
    let mut suggestion = String::new();
    let mut capital = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if capital {
                suggestion.extend(c.to_uppercase());
            } else {
                suggestion.push(c);
            }
            capital = false;
        } else {
            capital = true;
        }
    }
    Some(("must be UpperCamelCase".to_owned(), suggestion))
}

/// Whether a requirement identifier follows `[A-Z][A-Z0-9]*(_[A-Z0-9]+)*_[0-9]+`.
pub(crate) fn requirement(name: &str) -> bool {
    let Some((head, number)) = name.rsplit_once('_') else {
        return false;
    };
    let mut parts = head.split('_');
    let first = parts.next().unwrap_or("");
    let upper_digit = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit();
    !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
        && first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && first.chars().all(upper_digit)
        && parts.all(|part| !part.is_empty() && part.chars().all(upper_digit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_case_names() {
        assert_eq!(lower("total"), None);
        assert_eq!(lower("a_b1"), None);
        assert_eq!(
            lower("total_"),
            Some(("must not end with `_`".to_owned(), "total".to_owned()))
        );
        assert_eq!(
            lower("a__b"),
            Some(("must not contain `__`".to_owned(), "a_b".to_owned()))
        );
    }

    #[test]
    fn upper_case_names() {
        assert_eq!(upper("SeatCount"), None);
        assert_eq!(upper("HTTPServer"), None);
        assert_eq!(
            upper("Seat_Count"),
            Some(("must be UpperCamelCase".to_owned(), "SeatCount".to_owned()))
        );
    }

    #[test]
    fn requirement_identifiers() {
        for valid in ["EVENTS_3", "A_1", "AUDIT_LOG_3", "R2_D2_7"] {
            assert!(requirement(valid), "{valid}");
        }
        for invalid in [
            "EVENTS3",
            "EVENTS_",
            "Events_3",
            "EVENTS__3",
            "_12",
            "EVENTS_1A",
        ] {
            assert!(!requirement(invalid), "{invalid}");
        }
    }
}
