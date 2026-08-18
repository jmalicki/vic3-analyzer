//! Resolve Clausewitz localization markup into display text.
//!
//! Vic3 pop-type names are stored as `@academics! $academics_no_icon$` with the
//! readable word on the `_no_icon` key. `@academics!` is a texticon for
//! `gfx/interface/icons/pops_icons/academics.dds`, stored as `pop:academics`;
//! labels keep the substituted name and drop the marker.

use std::collections::{BTreeMap, HashSet};

/// Expand `$key$` substitutions and strip `@icon!` markers in place.
pub(crate) fn polish_labels(labels: &mut BTreeMap<String, String>) {
    if labels.is_empty() {
        return;
    }
    let raw = labels.clone();
    let mut memo = BTreeMap::new();
    let mut visiting = HashSet::new();
    for key in raw.keys() {
        visiting.clear();
        let value = resolve_key(&raw, key, &mut visiting, &mut memo);
        labels.insert(key.clone(), value);
    }
}

fn resolve_key(
    labels: &BTreeMap<String, String>,
    key: &str,
    visiting: &mut HashSet<String>,
    memo: &mut BTreeMap<String, String>,
) -> String {
    if let Some(done) = memo.get(key) {
        return done.clone();
    }
    let Some(raw) = labels.get(key) else {
        return String::new();
    };
    if !visiting.insert(key.to_string()) {
        return raw.clone();
    }
    let text = strip_icons(&expand_dollars(raw, labels, visiting, memo));
    visiting.remove(key);
    memo.insert(key.to_string(), text.clone());
    text
}

fn expand_dollars(
    text: &str,
    labels: &BTreeMap<String, String>,
    visiting: &mut HashSet<String>,
    memo: &mut BTreeMap<String, String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let Some(end) = rest.find('$') else {
            out.push('$');
            out.push_str(rest);
            return out;
        };
        let inner = &rest[..end];
        rest = &rest[end + 1..];
        let key = inner.split_once('|').map(|(key, _)| key).unwrap_or(inner);
        if is_loc_key(key) && labels.contains_key(key) {
            out.push_str(&resolve_key(labels, key, visiting, memo));
        } else {
            out.push('$');
            out.push_str(inner);
            out.push('$');
        }
    }
    out.push_str(rest);
    out
}

fn is_loc_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Drop `@icon_name!` markers and collapse leftover whitespace.
fn strip_icons(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('@') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let ident_len = after
            .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
            .unwrap_or(after.len());
        if ident_len > 0 && after[ident_len..].starts_with('!') {
            rest = &after[ident_len + 1..];
            continue;
        }
        out.push('@');
        rest = after;
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn pop_type_icon_and_no_icon_key_become_the_name() {
        let mut map = labels(&[
            ("academics", "@academics! $academics_no_icon$"),
            ("academics_no_icon", "Academics"),
            ("academics_only_icon", "@academics!"),
        ]);
        polish_labels(&mut map);
        assert_eq!(map.get("academics").map(String::as_str), Some("Academics"));
        assert_eq!(
            map.get("academics_no_icon").map(String::as_str),
            Some("Academics")
        );
        assert_eq!(map.get("academics_only_icon").map(String::as_str), Some(""));
    }

    #[test]
    fn nested_name_keys_expand() {
        let mut map = labels(&[
            ("building_rye_farm", "Rye Farms"),
            (
                "building_rye_farm_lens_option",
                "Expand $building_rye_farm$",
            ),
        ]);
        polish_labels(&mut map);
        assert_eq!(
            map.get("building_rye_farm_lens_option").map(String::as_str),
            Some("Expand Rye Farms")
        );
    }

    #[test]
    fn missing_and_formatter_keys_stay_or_resolve() {
        let mut map = labels(&[
            ("clerks", "Clerks"),
            ("hint", "Hire #v $clerks|v$#! not $ghost$"),
        ]);
        polish_labels(&mut map);
        assert_eq!(
            map.get("hint").map(String::as_str),
            Some("Hire #v Clerks#! not $ghost$")
        );
    }

    #[test]
    fn cycles_do_not_loop() {
        let mut map = labels(&[("a", "$b$"), ("b", "$a$")]);
        polish_labels(&mut map);
        assert!(map.contains_key("a"));
        assert!(map.contains_key("b"));
    }

    #[test]
    fn plain_labels_are_unchanged() {
        let mut map = labels(&[("grain", "Grain")]);
        polish_labels(&mut map);
        assert_eq!(map.get("grain").map(String::as_str), Some("Grain"));
    }
}
