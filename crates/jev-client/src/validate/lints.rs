//! Lints: advice that does not stop a request, but usually improves its answers.

use serde_json::Value;

use super::document::child_pointer;
use super::finding::{Finding, Rule};
use super::rules::{QuestionView, RequestView};

/// An array longer than this is one the model miscounts into. Measured against the live API: every
/// index into 20 items was read correctly, while 10 of 60 were wrong.
const LONG_ARRAY_ITEMS: usize = 20;

/// Option names that give the model a way out when nothing fits.
const ESCAPE_OPTIONS: [&str; 24] = [
    "other",
    "others",
    "else",
    "something_else",
    "none",
    "none_of_the_above",
    "none_of_these",
    "neither",
    "no_match",
    "not_stated",
    "not_specified",
    "not_mentioned",
    "not_applicable",
    "not_sure",
    "na",
    "n_a",
    "unknown",
    "unclear",
    "unsure",
    "uncertain",
    "ambiguous",
    "undetermined",
    "cannot_tell",
    "insufficient_information",
];

const YES_NO_PAIRS: [[&str; 2]; 3] = [["yes", "no"], ["true", "false"], ["y", "n"]];

pub(super) fn check(request: &RequestView<'_>, findings: &mut Vec<Finding>) {
    for question in &request.questions {
        let criteria_pointer = child_pointer(&question.pointer, "criteria");
        match (question.kind, question.fields.get("criteria")) {
            (Some("choice"), Some(Value::Object(options))) if !options.is_empty() => {
                let names: Vec<String> = options.keys().map(|name| normalise(name)).collect();
                check_escape_option(question, &criteria_pointer, &names, findings);
                check_yes_no(question, &criteria_pointer, &names, findings);
                let described = options
                    .iter()
                    .map(|(name, description)| (name.clone(), description));
                check_duplicate_descriptions(
                    question,
                    &criteria_pointer,
                    "options",
                    described,
                    findings,
                );
            }
            (Some("score"), Some(Value::Array(levels))) => {
                let described = levels
                    .iter()
                    .enumerate()
                    .map(|(index, level)| (index.to_string(), level));
                check_duplicate_descriptions(
                    question,
                    &criteria_pointer,
                    "levels",
                    described,
                    findings,
                );
            }
            _ => {}
        }
        check_indexed_paths(question, request.state, findings);
    }
}

/// Lower-case, with every run of other characters collapsed to one underscore.
fn normalise(name: &str) -> String {
    let mut normalised = String::with_capacity(name.len());
    for character in name.trim().chars() {
        if character.is_alphanumeric() {
            normalised.extend(character.to_lowercase());
        } else if !normalised.ends_with('_') {
            normalised.push('_');
        }
    }
    normalised.trim_matches('_').to_owned()
}

fn is_escape_option(normalised: &str) -> bool {
    ESCAPE_OPTIONS.contains(&normalised)
        || ["other_", "none_of_", "not_stated_", "unknown_"]
            .iter()
            .any(|prefix| normalised.starts_with(prefix))
}

fn check_escape_option(
    question: &QuestionView<'_>,
    pointer: &str,
    names: &[String],
    findings: &mut Vec<Finding>,
) {
    if names.iter().any(|name| is_escape_option(name)) {
        return;
    }
    findings.push(
        Finding::warning(
            Rule::ChoiceNoEscapeOption,
            pointer,
            format!(
                "choice `{}` has no way out; the model must pick one of its options even when none fits",
                question.id
            ),
        )
        .in_question(question.id)
        .suggest("add an option such as `other`, `none_of_the_above` or `not_stated`"),
    );
}

fn check_yes_no(
    question: &QuestionView<'_>,
    pointer: &str,
    names: &[String],
    findings: &mut Vec<Finding>,
) {
    let [first, second] = names else { return };
    let is_pair = YES_NO_PAIRS
        .iter()
        .any(|[yes, no]| (first == yes && second == no) || (first == no && second == yes));
    if !is_pair {
        return;
    }
    findings.push(
        Finding::warning(
            Rule::ChoiceYesNo,
            pointer,
            format!("choice `{}` is a yes/no question", question.id),
        )
        .in_question(question.id)
        .suggest(
            "use a noul: it returns the probability of yes, which a threshold can be tuned against",
        ),
    );
}

fn check_duplicate_descriptions<'a>(
    question: &QuestionView<'_>,
    pointer: &str,
    what: &str,
    described: impl Iterator<Item = (String, &'a Value)>,
    findings: &mut Vec<Finding>,
) {
    let mut seen: Vec<(String, &Value)> = Vec::new();
    for (name, description) in described {
        if description.is_null()
            || description
                .as_str()
                .is_some_and(|text| text.trim().is_empty())
        {
            continue;
        }
        if let Some((earlier, _)) = seen
            .iter()
            .find(|(_, other)| same_description(other, description))
        {
            findings.push(
                Finding::warning(
                    Rule::DuplicateDescription,
                    child_pointer(pointer, &name),
                    format!(
                        "{what} `{earlier}` and `{name}` of `{}` have the same description",
                        question.id
                    ),
                )
                .in_question(question.id)
                .suggest("say what tells them apart, or merge them"),
            );
        } else {
            seen.push((name, description));
        }
    }
}

fn same_description(left: &Value, right: &Value) -> bool {
    match (left.as_str(), right.as_str()) {
        (Some(left), Some(right)) => left.trim().eq_ignore_ascii_case(right.trim()),
        _ => left == right,
    }
}

/// Warns about backticked paths such as `` `items[47].name` `` that index into a long array.
fn check_indexed_paths(question: &QuestionView<'_>, state: &Value, findings: &mut Vec<Finding>) {
    let mut reported: Vec<String> = Vec::new();
    for field in ["instructions", "criteria"] {
        let Some(content) = question.fields.get(field) else {
            continue;
        };
        let field_pointer = child_pointer(&question.pointer, field);
        visit_strings(content, &field_pointer, &mut |text, pointer| {
            for path in backticked(text) {
                let Some(length) = longest_indexed_array(state, path) else {
                    continue;
                };
                if length <= LONG_ARRAY_ITEMS || reported.iter().any(|seen| seen == path) {
                    continue;
                }
                reported.push(path.to_owned());
                findings.push(
                    Finding::warning(
                        Rule::PathIndexLongArray,
                        pointer,
                        format!(
                            "`{path}` indexes into an array of {length} items; the model miscounts positions in arrays longer than {LONG_ARRAY_ITEMS}"
                        ),
                    )
                    .in_question(question.id)
                    .suggest("put the value itself in the instructions, or key the items by a short id instead of a position"),
                );
            }
        });
    }
}

/// Calls `visit` with every string inside `value`, and the JSON Pointer to it.
fn visit_strings(value: &Value, pointer: &str, visit: &mut impl FnMut(&str, &str)) {
    match value {
        Value::String(text) => visit(text, pointer),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                visit_strings(item, &child_pointer(pointer, &index.to_string()), visit);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                visit_strings(item, &child_pointer(pointer, key), visit);
            }
        }
        _ => {}
    }
}

/// The segments of `text` that are wrapped in single backticks.
fn backticked(text: &str) -> impl Iterator<Item = &str> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
}

/// One step of a path: a key, or an index into an array.
enum Step<'a> {
    Key(&'a str),
    Index(usize),
}

/// Parses `a.b[3].c` into steps. Returns `None` for text that is not a path with an index in it.
fn parse_path(path: &str) -> Option<Vec<Step<'_>>> {
    let mut steps = Vec::new();
    let mut has_index = false;
    for part in path.split('.') {
        let (key, mut rest) = part
            .split_once('[')
            .map_or((part, ""), |(key, rest)| (key, rest));
        if !key.is_empty() {
            if !key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                return None;
            }
            steps.push(Step::Key(key));
        } else if rest.is_empty() {
            return None;
        }
        while !rest.is_empty() {
            let (index, after) = rest.split_once(']')?;
            steps.push(Step::Index(index.trim().parse().ok()?));
            has_index = true;
            rest = match after.strip_prefix('[') {
                Some(next) => next,
                None if after.is_empty() => "",
                None => return None,
            };
        }
    }
    has_index.then_some(steps)
}

/// Follows `path` through `state` and returns the length of the longest array it indexes into.
/// A path that does not lead anywhere in `state` yields `None`: it may not be a path at all.
fn longest_indexed_array(state: &Value, path: &str) -> Option<usize> {
    let steps = parse_path(path)?;
    // People write both `items[3]` and `state.items[3]`.
    let skip = usize::from(
        matches!(steps.first(), Some(Step::Key("state"))) && state.get("state").is_none(),
    );

    let mut current = state;
    let mut longest = None;
    for step in steps.iter().skip(skip) {
        current = match step {
            Step::Key(key) => current.get(key)?,
            Step::Index(index) => {
                let items = current.as_array()?;
                longest = longest.max(Some(items.len()));
                // An index past the end still points into this array, so its length still counts.
                match items.get(*index) {
                    Some(item) => item,
                    None => return longest,
                }
            }
        };
    }
    longest
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{backticked, is_escape_option, longest_indexed_array, normalise};

    #[test]
    fn option_names_are_compared_without_case_or_punctuation() {
        assert_eq!(normalise(" None of the Above "), "none_of_the_above");
        assert_eq!(normalise("N/A"), "n_a");
        assert_eq!(normalise("not-stated"), "not_stated");
        assert_eq!(normalise("__Other__"), "other");
    }

    #[test]
    fn recognises_the_usual_ways_out() {
        for name in [
            "other",
            "none_of_the_above",
            "not_stated",
            "n_a",
            "unknown",
            "other_reason",
            "none_of_these_teams",
        ] {
            assert!(is_escape_option(name), "{name}");
        }
        for name in [
            "billing",
            "not_urgent",
            "another",
            "nonessential",
            "no",
            "otherwise",
        ] {
            assert!(!is_escape_option(name), "{name}");
        }
    }

    #[test]
    fn finds_backticked_segments() {
        let found: Vec<&str> =
            backticked("Compare `a[0]` with ` b.c[12] ` and `` and plain text").collect();

        assert_eq!(found, ["a[0]", "b.c[12]"]);
        assert_eq!(backticked("no code here").count(), 0);
        assert_eq!(
            backticked("an `unclosed segment").collect::<Vec<_>>(),
            ["unclosed segment"]
        );
    }

    #[test]
    fn follows_indexed_paths_through_the_state() {
        let state = json!({
            "items": (0..30).map(|n| json!({ "tags": ["a", "b"], "n": n })).collect::<Vec<_>>(),
            "few": [1, 2, 3],
            "grid": [[1, 2], [3, 4]]
        });

        assert_eq!(longest_indexed_array(&state, "items[3]"), Some(30));
        assert_eq!(longest_indexed_array(&state, "state.items[3].n"), Some(30));
        assert_eq!(
            longest_indexed_array(&state, "items[3].tags[1]"),
            Some(30),
            "the longest one counts"
        );
        assert_eq!(
            longest_indexed_array(&state, "items[99]"),
            Some(30),
            "past the end still indexes it"
        );
        assert_eq!(longest_indexed_array(&state, "few[0]"), Some(3));
        assert_eq!(longest_indexed_array(&state, "grid[1][0]"), Some(2));
    }

    #[test]
    fn ignores_backticks_that_are_not_indexed_paths_into_the_state() {
        let state = json!({ "items": [1, 2, 3], "name": "x" });

        for not_a_path in [
            "items",
            "name",
            "missing[0]",
            "items[x]",
            "items[0",
            "name[0]",
            "f(x)[0]",
            "a b[0]",
            "[",
            "",
        ] {
            assert_eq!(
                longest_indexed_array(&state, not_a_path),
                None,
                "{not_a_path:?}"
            );
        }
        assert_eq!(
            longest_indexed_array(&json!("plain text state"), "items[0]"),
            None
        );
        assert_eq!(
            longest_indexed_array(&json!([[1, 2, 3]]), "[0][1]"),
            Some(3),
            "a root array"
        );
    }
}
