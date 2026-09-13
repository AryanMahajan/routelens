//! Deciding whether an assertion or condition holds.
//!
//! The values compared are always text — that is what a response yields and what a
//! `{{variable}}` carries. When both sides parse as numbers the comparison is numeric, so
//! `status equals 200` works without anyone thinking about types and `9 < 10` is true.

use rl_model::Operator;

/// Whether `actual op expected` holds. `actual` is `None` when the source named nothing.
pub fn holds(op: Operator, actual: Option<&str>, expected: &str) -> bool {
    let Some(actual) = actual else {
        // Nothing there: the negative forms are satisfied, every positive one is not.
        return matches!(
            op,
            Operator::NotExists | Operator::NotEquals | Operator::NotContains
        );
    };

    match op {
        Operator::Exists => true,
        Operator::NotExists => false,
        Operator::Equals => equal(actual, expected),
        Operator::NotEquals => !equal(actual, expected),
        Operator::Contains => actual.contains(expected),
        Operator::NotContains => !actual.contains(expected),
        Operator::GreaterThan => match numbers(actual, expected) {
            Some((a, b)) => a > b,
            None => actual > expected,
        },
        Operator::LessThan => match numbers(actual, expected) {
            Some((a, b)) => a < b,
            None => actual < expected,
        },
    }
}

fn equal(actual: &str, expected: &str) -> bool {
    match numbers(actual, expected) {
        Some((a, b)) => a == b,
        None => actual == expected,
    }
}

fn numbers(a: &str, b: &str) -> Option<(f64, f64)> {
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// The symbol shown for an operator on a card.
pub fn symbol(op: Operator) -> &'static str {
    match op {
        Operator::Equals => "==",
        Operator::NotEquals => "!=",
        Operator::Contains => "contains",
        Operator::NotContains => "not contains",
        Operator::Exists => "exists",
        Operator::NotExists => "not exists",
        Operator::GreaterThan => ">",
        Operator::LessThan => "<",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Operator::*;

    #[test]
    fn numbers_compare_numerically_when_both_sides_are_numbers() {
        assert!(holds(Equals, Some("200"), "200"));
        assert!(holds(Equals, Some("200"), " 200.0 "));
        assert!(holds(LessThan, Some("9"), "10"), "not a string comparison");
        assert!(holds(GreaterThan, Some("404"), "399"));
        assert!(!holds(GreaterThan, Some("200"), "200"));
    }

    #[test]
    fn strings_compare_as_strings_otherwise() {
        assert!(holds(Equals, Some("admin"), "admin"));
        assert!(!holds(Equals, Some("admin"), "Admin"));
        assert!(holds(Contains, Some("Bearer abc"), "abc"));
        assert!(holds(NotContains, Some("Bearer abc"), "xyz"));
        assert!(holds(LessThan, Some("a"), "b"));
        assert!(holds(NotEquals, Some("1"), "one"));
    }

    #[test]
    fn an_absent_value_satisfies_only_the_negative_forms() {
        assert!(holds(NotExists, None, ""));
        assert!(holds(NotEquals, None, "x"));
        assert!(holds(NotContains, None, "x"));
        assert!(!holds(Exists, None, ""));
        assert!(!holds(Equals, None, ""));
        assert!(!holds(Contains, None, ""));
        assert!(!holds(GreaterThan, None, "0"));
        assert!(!holds(LessThan, None, "0"));
    }

    #[test]
    fn exists_ignores_the_right_hand_side() {
        assert!(holds(Exists, Some(""), "anything"));
        assert!(!holds(NotExists, Some(""), ""));
    }
}
