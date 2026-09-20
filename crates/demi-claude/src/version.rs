//! CLI version strings: the accepted form, and their semantic order.

use std::cmp::Ordering;

/// Whether `version` matches `^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$`.
/// A version names its installation directory, so nothing else is accepted.
pub fn is_valid(version: &str) -> bool {
    let (core, prerelease) = split(version);
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        && prerelease.is_none_or(|prerelease| {
            !prerelease.is_empty()
                && prerelease
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        })
}

/// Semantic version precedence of two valid versions. A release is newer than
/// its prereleases; numeric identifiers compare as numbers and sort before
/// alphanumeric ones.
pub fn compare(left: &str, right: &str) -> Ordering {
    let (left_core, left_prerelease) = split(left);
    let (right_core, right_prerelease) = split(right);
    let core = left_core
        .split('.')
        .zip(right_core.split('.'))
        .map(|(left, right)| compare_numbers(left, right))
        .find(|order| order.is_ne())
        .unwrap_or(Ordering::Equal);
    core.then_with(|| match (left_prerelease, right_prerelease) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_prereleases(left, right),
    })
}

fn split(version: &str) -> (&str, Option<&str>) {
    match version.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (version, None),
    }
}

/// Digit strings of any length compare as numbers without being parsed.
fn compare_numbers(left: &str, right: &str) -> Ordering {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn compare_prereleases(left: &str, right: &str) -> Ordering {
    let mut left = left.split('.');
    let mut right = right.split('.');
    loop {
        let order = match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(left), Some(right)) => match (is_numeric(left), is_numeric(right)) {
                (true, true) => compare_numbers(left, right),
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                (false, false) => left.cmp(right),
            },
        };
        if order.is_ne() {
            return order;
        }
    }
}

fn is_numeric(identifier: &str) -> bool {
    !identifier.is_empty() && identifier.bytes().all(|byte| byte.is_ascii_digit())
}
