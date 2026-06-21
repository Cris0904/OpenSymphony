//! Shared helper utilities for the OpenSymphony CLI.
//!
//! These helpers live outside the individual command modules so both
//! `init` ([`crate::init_repo`]) and `update` ([`crate::update_repo`]),
//! plus the project-set writer, can reuse the same canonical
//! implementations. Keeping a single source of truth here prevents
//! subtle drift in places like whitespace handling for the project-set
//! inventory (`LOC-19`).

/// Trims `value` and returns it as a `String` when the result is non-empty.
///
/// Returns `None` when the input is `None` or trims to an empty string.
/// Centralised here so the project-set writer and `init` share one
/// implementation — see `LOC-19` AI review feedback on `trimmed_non_empty`
/// duplication.
pub fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_non_empty_returns_none_for_none_input() {
        assert_eq!(trimmed_non_empty(None), None);
    }

    #[test]
    fn trimmed_non_empty_returns_none_for_empty_or_whitespace() {
        assert_eq!(trimmed_non_empty(Some("")), None);
        assert_eq!(trimmed_non_empty(Some("   ")), None);
        assert_eq!(trimmed_non_empty(Some("\t\n  ")), None);
    }

    #[test]
    fn trimmed_non_empty_trims_surrounding_whitespace() {
        assert_eq!(
            trimmed_non_empty(Some("  hello  ")),
            Some("hello".to_owned())
        );
    }
}
