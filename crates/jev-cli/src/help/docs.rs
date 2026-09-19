//! The help of every command. See [`super::Doc`] for what each part is for.
//!
//! Write for a reader who has only this text, person or agent: say when to pick the command over
//! its siblings, and make every example something that runs as written.

use super::{Doc, Example, ExitCode, code, code_as};
use crate::exit::Exit;

/// The examples of `jev --help`.
pub(crate) const ROOT_EXAMPLES: &[Example] = &[
    Example {
        description: "Ask a yes/no question about a file; exit 10 when the answer is probably no",
        command: "jev noul \"Is the customer asking for a refund?\" --state-file ticket.txt --fail-under 0.7",
    },
    Example {
        description: "Every command, flag and exit code as JSON, for agents and tooling",
        command: "jev spec | jq -r '.commands[].path'",
    },
];

/// What every `jev schema` command reads.
const SCHEMA_INPUT: &str =
    "Nothing. It needs no API key and no configuration, and never uses the network.";

/// What every `jev schema` command can exit with.
const SCHEMA_EXIT_CODES: &[ExitCode] = &[code(Exit::Success), code(Exit::Usage)];

/// Every command's help, in the order of the command tree.
pub(crate) const DOCS: &[Doc] = &[
    Doc {
        path: "eval",
        when: "Use `jev eval` to ask several questions about the same state in one call: the answers \
come back together, and extra questions cost tokens, not time. Use `jev noul`, `jev choice` or \
`jev score` for one question written on the command line, `jev validate` to check a request file \
without sending it, and `jev batch run` to apply the same questions to many states.",
        input: "A request file (-f), JSON or YAML, shaped like the API body: `questions`, a map of \
question id to question, and optionally `state` and `model`. The state is --state or --state-file \
(`-` for stdin), else the file's `state`, else a pipe. The model is --model, else the file's \
`model`, else TYPESAFE_DEFAULT_MODEL, else the profile, else `jev-latest`.",
        output: "The result envelope: `model` (the versioned id that answered), `requested_model`, \
`answers` (one per question id, exactly as the API returned them), `usage`, `cost_usd` (an \
estimate), `request_id` and `latency_ms`, plus `gate` when --assert was used. --dry-run prints the \
request body and a size estimate instead, and sends nothing; --raw prints the API's response body \
as received.",
        exit_codes: &[
            code_as(Exit::Success, "success, and every --assert condition holds"),
            code(Exit::Usage),
            code(Exit::Auth),
            code(Exit::ApiRejected),
            code(Exit::RateLimited),
            code(Exit::Network),
            code_as(
                Exit::GateFalse,
                "evaluated, and an --assert condition is false (never an error)",
            ),
        ],
        examples: &[
            Example {
                description: "Several questions about one ticket, in a single call",
                command: "jev eval -f triage.yaml --state-file ticket.txt",
            },
            Example {
                description: "Only one answer's probability, for a script",
                command: "jev eval -f triage.yaml --state-file ticket.txt --field answers.is_urgent.noul",
            },
            Example {
                description: "Gate a CI step: exit 10 unless the change is confidently low risk",
                command: "jev eval -f review.yaml --state-file diff.txt \\\n    --assert \"risk == low\" --assert \"risk.confidence >= 0.8\"",
            },
            Example {
                description: "Check the request and see its size, without sending it or needing a key",
                command: "jev eval -f triage.yaml --state-file ticket.txt --dry-run -o json",
            },
        ],
    },
    Doc {
        path: "noul",
        when: "Use `jev noul` when the answer is yes or no about one property: does this hold? The \
probability is absolute: it can be low even when nothing fits better, and several nouls can all be \
high, so ask one noul per label when several labels can apply at once. Use `jev choice` when \
exactly one of several options must be picked (which one?), and `jev score` for a position on a \
described scale (how much?). A value near 0.5 means the model cannot tell, not \"medium\".",
        input: "QUESTION in plain words, or --instructions-file with structured instructions. \
--true and --false (or --criteria-file) say what yes and no mean. The state is --state or \
--state-file (`-` for stdin), else a pipe. Jev reads literally and cannot count, do arithmetic or \
compare dates: keep those in code.",
        output: "`noul`, the probability of yes from 0 to 1, then `model`, `requested_model`, \
`usage`, `cost_usd` (an estimate), `request_id`, `latency_ms`, and `gate` when a gating flag was \
used. `--field noul` prints just the number.",
        exit_codes: &[
            code_as(
                Exit::Success,
                "success, and the condition holds (or no gate was given)",
            ),
            code(Exit::Usage),
            code(Exit::Auth),
            code(Exit::ApiRejected),
            code(Exit::RateLimited),
            code(Exit::Network),
            code_as(
                Exit::GateFalse,
                "evaluated, and P(yes) is below --fail-under or above --fail-over (never an error)",
            ),
            code_as(
                Exit::Abstain,
                "evaluated, and P(yes) is inside --abstain-band: the model cannot tell",
            ),
        ],
        examples: &[
            Example {
                description: "Branch in a shell script: exit 0 when P(yes) >= 0.7, exit 10 when it is lower",
                command: "if git log -1 --pretty=%B | jev noul \"Does this commit describe a user-facing change?\" \\\n     --fail-under 0.7; then\n  echo \"needs a changelog entry\"\nfi",
            },
            Example {
                description: "Just the probability",
                command: "jev noul \"Is the customer angry?\" --state-file ticket.txt --field noul",
            },
            Example {
                description: "Route uncertain answers to a person: exit 11 when the model cannot tell",
                command: "jev noul \"Is this spam?\" --state \"$MESSAGE\" --fail-under 0.6 --abstain-band 0.4,0.6",
            },
        ],
    },
    Doc {
        path: "choice",
        when: "Use `jev choice` when exactly one of several options is the answer: which one? The \
probabilities are relative and add up to 1, so always offer a way out such as `other` or \
`not_stated`: the model cannot pick an option you left out. Use `jev noul` when the answer is yes \
or no, or when several labels can apply at once (one noul per label), and `jev score` when the \
options are ordered levels of one scale. Two options that mean yes and no are a noul.",
        input: "QUESTION in plain words, or --instructions-file. The options are repeated --option \
`name` or `name=description` (up to 255), or --criteria-file with a map of name to description; \
describe options that are easy to confuse. The state is --state or --state-file (`-` for stdin), \
else a pipe.",
        output: "`choice` (the winning option), `confidence` (how peaked the distribution is, not \
how likely it is to be right) and `probabilities` per option, then `model`, `requested_model`, \
`usage`, `cost_usd` (an estimate), `request_id`, `latency_ms`, and `gate` when a gating flag was \
used. `--field choice` prints just the winner.",
        exit_codes: &[
            code_as(
                Exit::Success,
                "success, and the condition holds (or no gate was given)",
            ),
            code(Exit::Usage),
            code(Exit::Auth),
            code(Exit::ApiRejected),
            code(Exit::RateLimited),
            code(Exit::Network),
            code_as(
                Exit::GateFalse,
                "evaluated, and the winner is not --expect or is below --min-confidence (never an \
error)",
            ),
        ],
        examples: &[
            Example {
                description: "Pick a team, always offering a way out, and print only the winner",
                command: "jev choice \"Which team should handle this?\" --state-file ticket.txt \\\n    --option billing=\"Payments, invoices, refunds\" --option technical=\"Bugs, outages\" \\\n    --option other --field choice",
            },
            Example {
                description: "Branch in a shell script: exit 0 when either option wins with enough confidence, 10 otherwise",
                command: "if jev choice \"Which team?\" --state-file ticket.txt \\\n     --option billing --option sales --option other \\\n     --expect billing --expect sales --min-confidence 0.8; then\n  echo \"route to revenue\"\nfi",
            },
        ],
    },
    Doc {
        path: "score",
        when: "Use `jev score` to place the state on one ordered scale: how much, how severe, how \
good? Each level is judged on its own, so describe a situation per level rather than \"worse than \
the previous one\", and rate one dimension per score. Use `jev noul` for yes or no, and \
`jev choice` for options with no order.",
        input: "QUESTION in plain words, or --instructions-file. The levels are repeated --level, \
lowest first (2 to 10), or --criteria-file with an ordered list; they are numbered from 0. The \
state is --state or --state-file (`-` for stdin), else a pipe.",
        output: "`score` (the expected level, from 0 to the highest level), `confidence`, \
`probabilities` and `legend` per level, then `model`, `requested_model`, `usage`, `cost_usd` (an \
estimate), `request_id`, `latency_ms`, and `gate` when a gating flag was used. `--field score` \
prints just the number.",
        exit_codes: &[
            code_as(
                Exit::Success,
                "success, and the condition holds (or no gate was given)",
            ),
            code(Exit::Usage),
            code(Exit::Auth),
            code(Exit::ApiRejected),
            code(Exit::RateLimited),
            code(Exit::Network),
            code_as(
                Exit::GateFalse,
                "evaluated, and --fail-under, --fail-over or --min-confidence does not hold (never \
an error)",
            ),
        ],
        examples: &[
            Example {
                description: "Rate on a rubric; levels are numbered from 0 in the order given",
                command: "jev score \"How frustrated is the customer?\" --state-file ticket.txt \\\n    --level Calm --level Frustrated --level \"Very angry\"",
            },
            Example {
                description: "Branch in a shell script: exit 10 when the score is above 1.5",
                command: "if ! jev score \"How risky is this change?\" --state-file diff.txt \\\n     --level Safe --level Risky --level Dangerous --fail-over 1.5 --field score; then\n  echo \"needs a second reviewer\"\nfi",
            },
        ],
    },
    Doc {
        path: "validate",
        when: "Use `jev validate` while writing a request file, in CI, or before a large batch: it \
runs every check `jev eval` runs before sending, offline, with no API key and no cost. Use \
`jev eval --dry-run` to also see the exact request body that would be sent.",
        input: "A request file (-f), JSON or YAML; `-` reads stdin. `state` and `model` may be \
left out. --state or --state-file adds a state, so that the size of the request can be estimated. \
Nothing is read from a pipe unless asked for with `-`.",
        output: "`file`, `valid`, `strict`, `summary` (counts of `errors` and `warnings`), \
`findings` (each with `severity`, `rule`, `question`, `path` as a JSON Pointer, `message` and \
`suggestion`) and `size`, the estimated input tokens when there is a state.",
        exit_codes: &[
            code_as(
                Exit::Success,
                "the request is valid; warnings allowed unless --strict",
            ),
            code_as(
                Exit::Usage,
                "the request is not valid, or the command line is wrong",
            ),
        ],
        examples: &[
            Example {
                description: "Check a request file while writing it",
                command: "jev validate -f triage.yaml",
            },
            Example {
                description: "Fail CI on any finding, warnings included, with a state for the size check",
                command: "jev validate -f triage.yaml --state-file ticket.txt --strict -o json",
            },
            Example {
                description: "List the rules a file breaks",
                command: "jev validate -f triage.yaml -o json | jq -r '.findings[].rule'",
            },
        ],
    },
    Doc {
        path: "batch run",
        when: "Use `jev batch run` to ask the same questions about many states: tickets, messages, \
records. It sends one request per row, a few at a time, and writes one result record per row. Use \
`jev eval` for one state and several questions, `jev noul`, `jev choice` or `jev score` for one \
state and one question, and `jev validate` to check the question set without sending anything.",
        input: "A request file (-f), JSON or YAML, whose `questions` (and `model`, if present) are \
used for every row; any `state` in it is replaced by each row's. The rows are --input, a JSONL file \
(one JSON value per line) or a CSV file with a header row, or JSONL piped on stdin. A row's state is \
the whole row, one field of it (--state-field) or an object of some fields (--state-fields); its id \
is --id-field, else its line number. Before anything is sent the question set is validated and \
every row of a file is checked: a malformed row, a missing field or a repeated id is exit 2. Piped \
rows can be read only once, so they are checked as they arrive, and such a row stops sending there.",
        output: "JSON Lines on stdout, or appended to --out: one record per row, in the order rows \
finish, whatever --output says. An answered row is `{id, status: \"ok\", model, answers, usage, \
cost_usd, request_id, latency_ms}`; a failed one is `{id, status: \"error\", error}`, where \
`error` has the shape of every JSON error (`code`, `exit_code`, `error_type`, `message`, `hint`, \
`request_id`, ...). A failed row does not stop the run unless --fail-fast or --max-errors says so. \
The summary goes to stderr (one `{\"summary\": ...}` JSON line when the output is for a program) \
and to --summary-json: `rows_total`, `ok`, `failed`, `skipped`, `input_tokens`, `cost_usd` (an \
estimate), `wall_time_ms`, `rows_per_second`, `retries`, `models` and `stopped_by`.",
        exit_codes: &[
            code_as(Exit::Success, "every row was answered"),
            code_as(
                Exit::Usage,
                "usage error, or a bad question set or input row; nothing was sent for a file",
            ),
            code(Exit::Auth),
            code_as(
                Exit::BatchPartial,
                "the run finished, but some rows failed: see their error records",
            ),
        ],
        examples: &[
            Example {
                description: "Label tickets: the `body` field is the state, records keyed by ticket_id",
                command: "jev batch run -f triage.yaml --input tickets.jsonl --state-field body \\\n    --id-field ticket_id --out results.jsonl",
            },
            Example {
                description: "Send two columns of a CSV export, and keep a machine-readable summary",
                command: "jev batch run -f triage.yaml --input export.csv --state-fields subject,body \\\n    --out results.jsonl --summary-json summary.json",
            },
            Example {
                description: "Rows from a pipeline, stopping at the first failure; list the failed ids",
                command: "jq -c '.items[]' dump.json | jev batch run -f triage.yaml --fail-fast \\\n    | jq -r 'select(.status == \"error\") | .id'",
            },
        ],
    },
    Doc {
        path: "models list",
        when: "Use `jev models list` to see the model names and aliases the account can use. A \
versioned id such as `jev-1.13.0` is accepted by --model even when it is not listed here. Aliases \
such as `jev-latest` move to newer versions without notice, so pin a versioned id once thresholds \
have been tuned against it. Use `jev auth status` to check the key alone, and `jev version` for \
the version of jev itself.",
        input: "Nothing but an API key (TYPESAFE_API_KEY or `jev auth login`). --profile and \
--base-url choose the account and the endpoint.",
        output: "`models`: one entry per model or alias, with `name`, `description` and \
`release_date`.",
        exit_codes: &[
            code(Exit::Success),
            code(Exit::Usage),
            code(Exit::Auth),
            code(Exit::ApiRejected),
            code(Exit::RateLimited),
            code(Exit::Network),
        ],
        examples: &[
            Example {
                description: "What the account can use",
                command: "jev models list",
            },
            Example {
                description: "Only the names, for a script",
                command: "jev models list -o json | jq -r '.models[].name'",
            },
        ],
    },
    Doc {
        path: "auth login",
        when: "Use `jev auth login` once per machine and profile, to store an API key in jev's \
private credentials file. Where TYPESAFE_API_KEY is set (CI, containers, agents) there is nothing \
to log in to: the variable always wins. Use `jev auth status` to see which key is in use, and \
`jev auth logout` to remove a stored one.",
        input: "The key, typed at a prompt that does not echo, or on stdin with --with-token. The \
key is never accepted as a flag value, so that it cannot end up in shell history or a process \
list. --profile chooses the profile it is stored for; --skip-verify stores it without checking it \
against the API. jev looks for a key in TYPESAFE_API_KEY first, then in the credentials file this \
command writes (next to config.toml, readable only by you).",
        output: "`profile`, `stored_in` (the credentials file), `verified` and `fingerprint`, the \
last four characters of the key. Nothing more of the key is ever shown.",
        exit_codes: &[
            code_as(Exit::Success, "the key is stored"),
            code_as(
                Exit::Usage,
                "no key was given, or a prompt was needed but not allowed: use --with-token",
            ),
            code_as(Exit::Auth, "the API refused the key; nothing was stored"),
            code(Exit::RateLimited),
            code(Exit::Network),
        ],
        examples: &[
            Example {
                description: "Interactive: type or paste the key at a prompt that does not echo",
                command: "jev auth login",
            },
            Example {
                description: "Automation: read the key from stdin",
                command: "printf %s \"$TYPESAFE_KEY\" | jev auth login --with-token --profile ci -o json",
            },
        ],
    },
    Doc {
        path: "auth status",
        when: "Use `jev auth status` to find out whether jev has an API key, where it comes from \
(the environment or the credentials file) and whether the API accepts it. Use `jev auth login` to \
store a key, and `jev models list` to see what the key gives access to.",
        input: "Nothing. --profile chooses the profile; --offline skips the check against the API.",
        output: "`profile`, `authenticated`, `source` (`env` or `file`), `fingerprint` (the last \
four characters of the key), `base_url` and `check` (`ok`, and `detail` when it failed). The key \
itself is never shown.",
        exit_codes: &[
            code_as(
                Exit::Success,
                "a key is configured, and the API accepts it (or --offline)",
            ),
            code_as(Exit::Usage, "the configuration cannot be read"),
            code_as(Exit::Auth, "no key is configured, or the API refused it"),
        ],
        examples: &[
            Example {
                description: "Is there a key, and does it work?",
                command: "jev auth status",
            },
            Example {
                description: "Check without the network, for a script",
                command: "jev auth status --offline --field authenticated",
            },
        ],
    },
    Doc {
        path: "auth logout",
        when: "Use `jev auth logout` to delete a key stored by `jev auth login`, for the selected \
profile or, with --all, for every profile. It cannot unset TYPESAFE_API_KEY: unset the variable \
yourself. Use `jev auth status` to check what is left.",
        input: "Nothing. --profile chooses the profile; --all removes the keys of every profile.",
        output: "`removed`: the profiles whose stored key was deleted, empty when there was none.",
        exit_codes: &[code(Exit::Success), code(Exit::Usage)],
        examples: &[
            Example {
                description: "Forget the key of one profile",
                command: "jev auth logout --profile staging",
            },
            Example {
                description: "Forget every stored key",
                command: "jev auth logout --all -o json",
            },
        ],
    },
    Doc {
        path: "config get",
        when: "Use `jev config get` to see the value one setting will have, and where it comes \
from: a flag, the environment, the profile or the default. Use `jev config list` for every \
setting at once, and `jev config set` to change one.",
        input: "KEY, one of `base_url`, `model`, `output`, `timeout`, `max_retries`, \
`concurrency` and `warn_unpinned`. --profile reads another profile, and flags on the same command \
line count, as they would on a real run.",
        output: "`key`, `value`, `source` (`flag`, `env`, `profile` or `default`), `origin` (the \
flag, variable or profile) and `description`. `--field value` prints just the value.",
        exit_codes: &[
            code(Exit::Success),
            code_as(
                Exit::Usage,
                "not a setting, or the configuration cannot be read",
            ),
        ],
        examples: &[
            Example {
                description: "Which model would a run use, and why?",
                command: "jev config get model",
            },
            Example {
                description: "Just the value, for a script",
                command: "jev config get timeout --field value",
            },
        ],
    },
    Doc {
        path: "config set",
        when: "Use `jev config set` to store a default in the selected profile, so that you do not \
repeat a flag. For one run, pass the flag or set the environment variable instead; \
`jev config unset` restores the default. The API key is not a setting: use `jev auth login`.",
        input: "KEY and VALUE. The value is checked before anything is written: `timeout` takes \
`30s` or `500ms`, `output` takes table, json, yaml or jsonl, and so on. --profile chooses the \
profile to change.",
        output: "`profile`, `key`, `value` and `changed`.",
        exit_codes: &[
            code_as(Exit::Success, "the setting is stored"),
            code_as(
                Exit::Usage,
                "not a setting, a value it does not accept, or the configuration cannot be \
written",
            ),
        ],
        examples: &[
            Example {
                description: "Pin a versioned model once thresholds are tuned against it",
                command: "jev config set model jev-1.13.0",
            },
            Example {
                description: "A longer timeout for the ci profile",
                command: "jev config set timeout 45s --profile ci -o json",
            },
        ],
    },
    Doc {
        path: "config unset",
        when: "Use `jev config unset` to remove a setting from the selected profile, so that the \
environment or the built-in default applies again. Use `jev config set` to change it instead, and \
`jev config get` to see the value that now applies.",
        input: "KEY, as for `jev config get`. --profile chooses the profile to change.",
        output: "`profile`, `key`, `value` (null) and `changed`, false when the profile did not \
set it.",
        exit_codes: &[
            code(Exit::Success),
            code_as(
                Exit::Usage,
                "not a setting, or the configuration cannot be written",
            ),
        ],
        examples: &[
            Example {
                description: "Back to the default timeout",
                command: "jev config unset timeout",
            },
            Example {
                description: "Stop pinning a model in the ci profile",
                command: "jev config unset model --profile ci -o json",
            },
        ],
    },
    Doc {
        path: "config list",
        when: "Use `jev config list` to see every effective setting and where each value comes \
from, e.g. to find out why a run used an unexpected model. Use `jev config get` for one setting, \
and `jev profile list` for the profiles themselves.",
        input: "Nothing. --profile lists another profile, and flags on the same command line \
count.",
        output: "`profile` (the selected profile, and how it was selected), `settings` (one row \
per setting with `key`, `value`, `source`, `origin` and `description`) and `config_file`.",
        exit_codes: &[
            code(Exit::Success),
            code_as(Exit::Usage, "the configuration cannot be read"),
        ],
        examples: &[
            Example {
                description: "Every setting and where it comes from",
                command: "jev config list",
            },
            Example {
                description: "Only the settings that are not defaults",
                command: "jev config list -o json | jq '.settings[] | select(.source != \"default\")'",
            },
        ],
    },
    Doc {
        path: "config path",
        when: "Use `jev config path` to find the configuration file, to read or edit it by hand. \
It works even when the file is broken, so it is the way to find a file that needs fixing. Use \
`jev config list` to see the values in effect.",
        input: "Nothing. JEV_CONFIG_DIR moves the configuration directory.",
        output: "`config_dir`, `config_file` and `exists`. `--field config_file` prints just the \
path.",
        exit_codes: &[
            code(Exit::Success),
            code_as(
                Exit::Usage,
                "the configuration directory cannot be worked out",
            ),
        ],
        examples: &[
            Example {
                description: "Where the configuration lives",
                command: "jev config path",
            },
            Example {
                description: "Only the file's path, for a script",
                command: "jev config path --field config_file",
            },
        ],
    },
    Doc {
        path: "profile list",
        when: "Use `jev profile list` to see the profiles, which one is active and what each one \
sets. Use `jev config list` for the values a run would actually use, after flags and environment \
variables.",
        input: "Nothing.",
        output: "`profiles`: one entry per profile, with `name`, `active` (used when nothing \
selects another), `selected` (used by this run) and `settings`.",
        exit_codes: &[
            code(Exit::Success),
            code_as(Exit::Usage, "the configuration cannot be read"),
        ],
        examples: &[
            Example {
                description: "The profiles, marking the active one",
                command: "jev profile list",
            },
            Example {
                description: "Only the names, for a script",
                command: "jev profile list -o json | jq -r '.profiles[].name'",
            },
        ],
    },
    Doc {
        path: "profile use",
        when: "Use `jev profile use` to change the profile that applies when neither --profile nor \
JEV_PROFILE selects one. For a single run, pass --profile instead. Use `jev profile create` first \
when the profile does not exist.",
        input: "NAME, an existing profile.",
        output: "`profile` and `action`, which is `active`.",
        exit_codes: &[
            code(Exit::Success),
            code_as(
                Exit::Usage,
                "no such profile, or the configuration cannot be written",
            ),
        ],
        examples: &[
            Example {
                description: "Work against staging from now on",
                command: "jev profile use staging",
            },
            Example {
                description: "Back to the default profile",
                command: "jev profile use default -o json",
            },
        ],
    },
    Doc {
        path: "profile create",
        when: "Use `jev profile create` to keep a separate set of defaults, e.g. for a staging \
endpoint or for CI. Use `jev config set --profile NAME` to change one setting of an existing \
profile, and `jev auth login --profile NAME` to give it a key.",
        input: "NAME: letters, digits, `-` and `_`. --base-url, --model, --output, --timeout and \
--max-retries given on the same command line become its settings.",
        output: "`profile` and `action`, which is `created`.",
        exit_codes: &[
            code(Exit::Success),
            code_as(
                Exit::Usage,
                "the name is taken or not valid, or the configuration cannot be written",
            ),
        ],
        examples: &[
            Example {
                description: "A profile for a staging endpoint with a pinned model",
                command: "jev profile create staging --base-url https://staging.example.com --model jev-1.13.0",
            },
            Example {
                description: "A profile with a longer timeout",
                command: "jev profile create ci --timeout 60s -o json",
            },
        ],
    },
    Doc {
        path: "profile delete",
        when: "Use `jev profile delete` to remove a profile, its settings and its stored API key. \
Use `jev config unset` to remove one setting instead, and `jev auth logout` to remove only the \
key.",
        input: "NAME, an existing profile other than `default`.",
        output: "`profile` and `action`, which is `deleted`.",
        exit_codes: &[
            code(Exit::Success),
            code_as(
                Exit::Usage,
                "no such profile, or the configuration cannot be written",
            ),
        ],
        examples: &[
            Example {
                description: "Remove a profile that is no longer needed",
                command: "jev profile delete staging",
            },
            Example {
                description: "The same, for a script",
                command: "jev profile delete staging -o json",
            },
        ],
    },
    Doc {
        path: "schema request",
        when: "Use `jev schema request` to author or check a file that carries everything an \
evaluation needs: `state`, `model` and `questions`, exactly the body the API takes. Use \
`jev schema questions` for a file whose state comes from elsewhere, and `jev validate` to check a \
file offline with jev's own rules, which explain each problem and suggest the fix.",
        input: SCHEMA_INPUT,
        output: "A JSON Schema (draft 2020-12) with every definition inlined, generated from the \
types jev reads. It holds the limits: 1 to 255 choice options, 2 to 10 score levels, at least one \
question, and no unknown fields. JSON on a terminal too; -o yaml gives YAML.",
        exit_codes: SCHEMA_EXIT_CODES,
        examples: &[
            Example {
                description: "Save it for an editor or a JSON Schema validator",
                command: "jev schema request -o json > jev-request.schema.json",
            },
            Example {
                description: "The fields a question may have",
                command: "jev schema request | jq '.properties.questions.additionalProperties'",
            },
        ],
    },
    Doc {
        path: "schema questions",
        when: "Use `jev schema questions` to author or check a question set: a request file in \
which only `questions` is required, as given to `jev eval -f` with --state-file, to \
`jev validate` and to `jev batch run -f`. Use `jev schema request` for a file that must carry its \
own `state` and `model`.",
        input: SCHEMA_INPUT,
        output: "A JSON Schema (draft 2020-12) with every definition inlined: the request schema \
with only `questions` required. JSON on a terminal too; -o yaml gives YAML.",
        exit_codes: SCHEMA_EXIT_CODES,
        examples: &[
            Example {
                description: "Save it for an editor, then check a file offline",
                command: "jev schema questions -o json > questions.schema.json\njev validate -f triage.yaml",
            },
            Example {
                description: "The question types",
                command: "jev schema questions \\\n    | jq '.properties.questions.additionalProperties.oneOf[].properties.type'",
            },
        ],
    },
    Doc {
        path: "schema batch-record",
        when: "Use `jev schema batch-record` before writing code that reads the JSONL of \
`jev batch run`: every line is one record. Use `jev schema output` for the result of a single \
evaluation, and `jev schema questions` for the question set a batch reads.",
        input: SCHEMA_INPUT,
        output: "A JSON Schema (draft 2020-12) of one record: `anyOf` an `ok` record (`id`, \
`status`, `model`, `answers`, `usage`, `cost_usd`, `request_id`, `latency_ms`) and an `error` \
record (`id`, `status`, and an `error` object shaped like the JSON error on stderr). JSON on a \
terminal too; -o yaml gives YAML.",
        exit_codes: SCHEMA_EXIT_CODES,
        examples: &[
            Example {
                description: "Save it to check a results file",
                command: "jev schema batch-record -o json > jev-batch-record.schema.json",
            },
            Example {
                description: "The fields of a failed row",
                command: "jev schema batch-record | jq '.anyOf[1].properties | keys'",
            },
        ],
    },
    Doc {
        path: "schema output",
        when: "Use `jev schema output` before writing code that reads the JSON of `jev eval`, \
`jev noul`, `jev choice` or `jev score`. Use `jev schema error` for what they print on stderr \
when they fail.",
        input: SCHEMA_INPUT,
        output: "A JSON Schema (draft 2020-12) of the result: `anyOf` the `jev eval` form, with \
every answer under `answers`, and the one-question form, with the answer's fields at the top \
level. Both have `model`, `requested_model`, `usage`, `cost_usd`, `request_id` and `latency_ms`, \
and `gate` when a gating flag was used. JSON on a terminal too; -o yaml gives YAML.",
        exit_codes: SCHEMA_EXIT_CODES,
        examples: &[
            Example {
                description: "The fields every result has",
                command: "jev schema output | jq '.anyOf[0].required'",
            },
            Example {
                description: "Save it to check results in a test suite",
                command: "jev schema output -o json > jev-result.schema.json",
            },
        ],
    },
    Doc {
        path: "schema error",
        when: "Use `jev schema error` before writing code that handles jev's failures: branch on \
`code` or `exit_code`, show `message` and follow `hint`. Use `jev schema output` for what a \
successful command prints on stdout.",
        input: SCHEMA_INPUT,
        output: "A JSON Schema (draft 2020-12) of the one JSON object a failing command prints on \
stderr when its output format is machine-readable, and that an MCP tool error carries: \
`{error: {code, exit_code, error_type, message, hint, request_id, http_status, retryable, \
details}}`, every field always present. JSON on a terminal too; -o yaml gives YAML.",
        exit_codes: SCHEMA_EXIT_CODES,
        examples: &[
            Example {
                description: "The error fields",
                command: "jev schema error | jq '.properties.error.required'",
            },
            Example {
                description: "As YAML",
                command: "jev schema error -o yaml",
            },
        ],
    },
    Doc {
        path: "spec",
        when: "Use `jev spec` when a program or an AI agent needs to know what jev can do: every \
command, argument and flag (with its type, default, allowed values and environment variable), \
exit code and example, in one JSON document generated from the same definitions as the help. Use \
`jev --help` or `jev <command> --help` to read about commands as a person.",
        input: "Nothing. It needs no API key and no configuration, and never uses the network.",
        output: "One JSON document: `spec_version`, `name`, `version`, `about`, `global_flags`, \
`exit_codes` (the whole contract), `examples` and `commands`. Each command has `path`, `usage`, \
`about`, `implemented`, `when_to_use`, `input`, `output`, `arguments`, `flags`, `exit_codes` and \
`examples`. It is JSON on a terminal too; -o yaml gives YAML.",
        exit_codes: &[code(Exit::Success)],
        examples: &[
            Example {
                description: "Every command",
                command: "jev spec | jq -r '.commands[].path'",
            },
            Example {
                description: "The flags of one command",
                command: "jev spec | jq '.commands[] | select(.path == \"noul\") | .flags'",
            },
        ],
    },
    Doc {
        path: "mcp serve",
        when: "Use `jev mcp serve` when an AI agent should call Jev as a tool, through an MCP client \
such as Claude Code, Claude Desktop, Cursor or VS Code. From a shell script or CI, call \
`jev eval`, `jev noul`, `jev choice` or `jev score` directly instead.",
        input: "The Model Context Protocol on stdin, one JSON-RPC message per line, until the \
client closes stdin. Tools: evaluate (a state and many questions), noul, choice, score (one \
question each), validate (offline, needs no key) and list_models. The API key, base URL and model \
come from the usual places (--profile, the environment, the profile), read once at start-up; the \
key is never part of a tool's input, result or error. Every call is validated offline before \
anything is sent.",
        output: "Protocol messages only on stdout; logs, notices and warnings go to stderr. A tool \
result is the same JSON envelope as `jev eval -o json`, plus `session` (calls made and estimated \
spend so far). A failure is a tool error carrying the JSON error object `jev` prints on stderr, \
and an invalid request is a tool error listing every finding. API failures never end the server; \
if stdout cannot be written, it exits 1.",
        exit_codes: &[
            code_as(Exit::Success, "the client closed stdin"),
            code_as(Exit::Usage, "usage, or a broken configuration"),
        ],
        examples: &[
            Example {
                description: "Register with Claude Code",
                command: "claude mcp add jev -- jev mcp serve",
            },
            Example {
                description: "Refuse any call estimated above a tenth of a cent (pin a versioned model so it has a price)",
                command: "jev mcp serve --model jev-1.13.0 --max-cost-usd-per-call 0.001",
            },
            Example {
                description: "Use a profile's key and settings; logs go to stderr, never stdout",
                command: "jev mcp serve --profile work -v",
            },
            Example {
                description: "Check it from a shell: list the tools' names",
                command: "echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}' | jev mcp serve \\\n  | jq -r '.result.tools[].name'",
            },
        ],
    },
    Doc {
        path: "version",
        when: "Use `jev version` to report the jev you run, e.g. in a bug report. Use \
`jev --version` for the bare version, and `jev models list` for the model versions the API offers.",
        input: "Nothing.",
        output: "`version`, `commit`, `build_date`, `target` and `client_version`. \
`--field version` prints just the version.",
        exit_codes: &[code(Exit::Success)],
        examples: &[
            Example {
                description: "Everything about this build",
                command: "jev version",
            },
            Example {
                description: "Just the version, for a script",
                command: "jev version --field version",
            },
        ],
    },
];
