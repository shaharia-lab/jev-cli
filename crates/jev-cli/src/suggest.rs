//! "Did you mean ...?" for names a person typed.

/// The candidate that `written` is most likely a misspelling of, if any is close enough.
pub(crate) fn closest<'a>(
    written: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let written = written.to_lowercase();
    candidates
        .into_iter()
        .map(|candidate| {
            (
                candidate,
                edit_distance(&written, &candidate.to_lowercase()),
            )
        })
        .filter(|(candidate, distance)| {
            *distance > 0 && *distance <= 2.max(candidate.chars().count() / 4)
        })
        .min_by_key(|(_, distance)| *distance)
        .map(|(candidate, _)| candidate)
}

/// Levenshtein distance between two short strings.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous.get(column).copied().unwrap_or(usize::MAX)
                + usize::from(left_char != *right_char);
            let deletion = previous
                .get(column + 1)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let insertion = current
                .last()
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            current.push(substitution.min(deletion).min(insertion));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::closest;

    #[test]
    fn suggests_only_a_close_spelling() {
        let keys = ["base_url", "model", "output", "timeout", "max_retries"];

        assert_eq!(closest("modle", keys), Some("model"));
        assert_eq!(closest("Base-Url", keys), Some("base_url"));
        assert_eq!(closest("max_retrys", keys), Some("max_retries"));
        assert_eq!(closest("api_key", keys), None);
        assert_eq!(
            closest("model", keys),
            None,
            "an exact match needs no suggestion"
        );
    }
}
