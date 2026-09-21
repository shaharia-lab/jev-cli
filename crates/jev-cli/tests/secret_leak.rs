//! The cross-cutting secret-leak net (TEST-7, SEC-1, SEC-2, SEC-6, SEC-7).
//!
//! Every command and subcommand `jev spec` lists is run with a sentinel API key at the most
//! verbose level, with and without `--debug-bodies`, as text and as JSON, against a mock API that
//! echoes the `Authorization` header and the request (the 422 `FastAPI` shape included) in every
//! answer, in success and failure paths, with the key from the environment and from the
//! credentials file. The sentinel must never appear in stdout, stderr or any file `jev` writes,
//! other than the credentials file that exists to hold it; the `state` must never appear on stderr
//! unless bodies were asked for, nor in a file nobody named. Every request `jev` makes must go to
//! the configured API or to the stand-in for GitHub Releases, and nothing may reach any other
//! host: every proxy variable points at a trap that counts connections.
//!
//! A command `jev spec` lists always gets `--help`, a bare run and an unknown flag; beyond that
//! it needs at least one scenario in [`scenarios`], and the test fails naming any command that has
//! none. The per-feature sentinel tests stay where they are; this is the net under all of them.
//!
//! The updater is pointed at the local stand-in through the test hooks, so this file needs them.

#![cfg(feature = "internal-test-hooks")]
// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::fs;
use std::io::Read as _;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Map, Value, json};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A key that must never be shown. It is not a real credential.
const KEY: &str = "sentinel-key-do-not-leak-30ab-c7d1";

/// What a leak is looked for as: the key without its last four characters, which
/// `jev auth status` shows on purpose as the key's fingerprint.
const KEY_MARK: &str = "sentinel-key-do-not-leak-30ab";

/// The same key with a space in it, which cannot be sent: the path where the key is refused.
const UNUSABLE_KEY: &str = "sentinel-key-do-not-leak-30ab c7d1";

/// Customer data in `state`: logged only with `--debug-bodies`, written only where asked.
const STATE: &str = "sentinel-state-customer-data-30ab";

/// The MCP tools the sweep knows how to call. A new tool fails the test until it is added to
/// [`mcp_session`].
const MCP_TOOLS: [&str; 7] = [
    "evaluate",
    "noul",
    "choice",
    "score",
    "validate",
    "list_models",
    "batch_run",
];

/// Where the key comes from, or why it cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeySource {
    /// `TYPESAFE_API_KEY`.
    Env,
    /// The `credentials` file `jev auth login` writes.
    CredentialsFile,
    /// `TYPESAFE_API_KEY` holding a key that cannot be sent.
    UnusableEnv,
    /// A credentials file that others can read, which `jev` refuses.
    #[cfg(unix)]
    ExposedCredentialsFile,
}

impl KeySource {
    const fn usable(self) -> bool {
        matches!(self, Self::Env | Self::CredentialsFile)
    }

    const fn stored_in_a_file(self) -> bool {
        !matches!(self, Self::Env | Self::UnusableEnv)
    }

    /// What `TYPESAFE_API_KEY` holds for this source, if anything.
    const fn in_the_environment(self) -> Option<&'static str> {
        match self {
            Self::Env => Some(KEY),
            Self::UnusableEnv => Some(UNUSABLE_KEY),
            Self::CredentialsFile => None,
            #[cfg(unix)]
            Self::ExposedCredentialsFile => None,
        }
    }
}

/// How the mock API answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// 200, with every answer the request asked for.
    Answer,
    /// 401, with the `Authorization` header quoted in the message.
    Unauthorized,
    /// 422 in `FastAPI`'s list shape, echoing the request and the header.
    Rejected,
}

/// The mock API. Every response carries the `Authorization` header it was sent, and the request.
struct Api(Arc<Mutex<Mode>>);

impl Respond for Api {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let authorization = request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let reply = |status: u16, body: Value| {
            ResponseTemplate::new(status)
                .insert_header("x-typesafe-request-id", format!("req_{status}"))
                .set_body_json(body)
        };
        let mode = *self.0.lock().unwrap();
        if mode == Mode::Unauthorized || authorization != format!("Bearer {KEY}") {
            return reply(
                401,
                json!({ "detail": {
                    "error_type": format!("authentication_error for {authorization}"),
                    "message": format!("Invalid API key: {authorization}"),
                }}),
            );
        }
        if mode == Mode::Rejected {
            return reply(
                422,
                json!({ "detail": [{
                    "type": "value_error",
                    "loc": ["body", "state"],
                    "msg": format!("Value error, sent with {authorization}"),
                    "input": body,
                    "ctx": { "authorization": authorization },
                }]}),
            );
        }
        if request.url.path() == "/v1/models" {
            return reply(
                200,
                json!({
                    "models": [{ "name": "jev-latest", "description": authorization }],
                    "echo": authorization,
                }),
            );
        }
        reply(
            200,
            json!({
                "model": "jev-1.13.0",
                "answers": answers(&body, &authorization),
                "usage": { "input_tokens": 120, "output_tokens": 10 },
                "echo": { "authorization": authorization, "request": body },
            }),
        )
    }
}

/// A well-formed answer to every question in `request`, each echoing `echo`.
fn answers(request: &Value, echo: &str) -> Value {
    let mut answers = Map::new();
    let questions = request["questions"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (id, question) in questions {
        let answer = match question["type"].as_str() {
            Some("choice") => {
                let options: Vec<String> = question["criteria"]
                    .as_object()
                    .map(|criteria| criteria.keys().cloned().collect())
                    .unwrap_or_default();
                let share = 0.1
                    / f64::from(u32::try_from(options.len().saturating_sub(1).max(1)).unwrap_or(1));
                let probabilities: Map<String, Value> = options
                    .iter()
                    .enumerate()
                    .map(|(n, option)| (option.clone(), json!(if n == 0 { 0.9 } else { share })))
                    .collect();
                json!({
                    "type": "choice", "choice": options.first(), "confidence": 0.9,
                    "probabilities": probabilities, "echo": echo,
                })
            }
            Some("score") => {
                let levels: Vec<Value> = match &question["criteria"] {
                    Value::Array(levels) => levels.clone(),
                    Value::Object(levels) => levels.values().cloned().collect(),
                    _ => Vec::new(),
                };
                let share = 0.2
                    / f64::from(u32::try_from(levels.len().saturating_sub(1).max(1)).unwrap_or(1));
                let mut legend = Map::new();
                let mut probabilities = Map::new();
                for (n, level) in levels.into_iter().enumerate() {
                    legend.insert(n.to_string(), level);
                    probabilities.insert(n.to_string(), json!(if n == 0 { 0.8 } else { share }));
                }
                json!({
                    "type": "score", "score": 0.2, "confidence": 0.8, "legend": legend,
                    "probabilities": probabilities, "echo": echo,
                })
            }
            _ => json!({ "type": "noul", "noul": 0.9, "echo": echo }),
        };
        answers.insert(id, answer);
    }
    Value::Object(answers)
}

/// A proxy that is never meant to be used: it counts the connections it gets and closes them.
struct Trap {
    address: String,
    connections: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Trap {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (count, log) = (Arc::clone(&connections), Arc::clone(&seen));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                count.fetch_add(1, Ordering::SeqCst);
                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                let mut first = [0; 256];
                let read = stream.read(&mut first).unwrap_or(0);
                let line = String::from_utf8_lossy(first.get(..read).unwrap_or_default());
                log.lock()
                    .unwrap()
                    .push(line.lines().next().unwrap_or_default().to_owned());
            }
        });
        Self {
            address,
            connections,
            seen,
        }
    }
}

/// One `jev` command line, and what it should do when the key is good and the API answers.
struct Scenario {
    args: Vec<String>,
    stdin: String,
    /// Whether it calls the API, so is worth running against every mode of the mock.
    network: bool,
    /// The exit code with a usable key and an answering API, when it is certain.
    expect: Option<i32>,
}

impl Scenario {
    fn new(args: &[&str], network: bool) -> Self {
        Self {
            args: args.iter().map(ToString::to_string).collect(),
            stdin: String::new(),
            network,
            expect: None,
        }
    }

    fn expect(mut self, code: i32) -> Self {
        self.expect = Some(code);
        self
    }

    fn stdin(mut self, stdin: impl Into<String>) -> Self {
        self.stdin = stdin.into();
        self
    }
}

fn offline(args: &[&str]) -> Scenario {
    Scenario::new(args, false)
}

fn online(args: &[&str]) -> Scenario {
    Scenario::new(args, true)
}

/// A scratch directory holding the configuration and the files the commands read and write.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("secret-leak-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sandbox = Self { root };
        fs::create_dir_all(sandbox.work().join("data")).unwrap();
        let questions = questions();
        let request =
            json!({ "model": "jev-latest", "state": { "ticket": STATE }, "questions": questions });
        let rows = format!(
            "{}\n{}\n",
            json!({ "id": "r1", "body": format!("{STATE} one") }),
            json!({ "id": "r2", "body": format!("{STATE} two") })
        );
        for (path, text) in [
            ("request.json", request.to_string()),
            ("questions.json", json!({ "questions": questions }).to_string()),
            ("state.txt", STATE.to_owned()),
            ("rows.jsonl", rows.clone()),
            ("data/rows.jsonl", rows),
            ("data/questions.json", json!({ "questions": questions }).to_string()),
            (
                "invalid.json",
                json!({ "questions": { "q": { "type": "score", "instructions": "x", "criteria": ["only"] } } })
                    .to_string(),
            ),
        ] {
            fs::write(sandbox.work().join(path), text).unwrap();
        }
        sandbox
    }

    fn config(&self) -> PathBuf {
        self.root.join("config")
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn credentials(&self) -> PathBuf {
        self.config().join("credentials")
    }

    /// Removes what the batch scenarios write, so each pass starts them afresh.
    fn clear_outputs(&self) {
        for path in [
            "out.jsonl",
            "summary.json",
            "piped.jsonl",
            "data/mcp-out.jsonl",
        ] {
            let _ = fs::remove_file(self.work().join(path));
        }
    }

    /// Files that may hold `state`: the inputs this test wrote and the outputs a command named.
    fn may_hold_state(&self, path: &Path) -> bool {
        let relative = path.strip_prefix(self.work()).unwrap_or(path);
        [
            "request.json",
            "state.txt",
            "rows.jsonl",
            "out.jsonl",
            "piped.jsonl",
            "data/rows.jsonl",
            "data/mcp-out.jsonl",
        ]
        .iter()
        .any(|allowed| relative == Path::new(allowed))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn questions() -> Value {
    json!({
        "is_urgent": { "type": "noul", "instructions": "Is `ticket` urgent?" },
        "department": {
            "type": "choice", "instructions": "Which team?",
            "criteria": { "billing": "Payments", "technical": "Bugs", "other": null }
        },
        "frustration": {
            "type": "score", "instructions": "How frustrated is the customer?",
            "criteria": ["Calm", "Frustrated", "Very angry"]
        }
    })
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// A JSON-RPC session that calls every MCP tool, well and badly.
fn mcp_session(sandbox: &Sandbox) -> String {
    let data = sandbox.work().join("data");
    let path = |name: &str| data.join(name).to_str().unwrap().to_owned();
    let calls = [
        (
            "evaluate",
            json!({ "state": STATE, "questions": questions() }),
        ),
        (
            "noul",
            json!({ "state": STATE, "instructions": "Is it urgent?" }),
        ),
        (
            "choice",
            json!({ "state": STATE, "instructions": "Which team?", "criteria": { "billing": "Payments", "other": null } }),
        ),
        (
            "score",
            json!({ "state": STATE, "instructions": "How angry?", "criteria": ["Calm", "Angry"] }),
        ),
        (
            "validate",
            json!({ "state": STATE, "questions": questions() }),
        ),
        ("list_models", json!({})),
        (
            "batch_run",
            json!({
                "input": path("rows.jsonl"), "out": path("mcp-out.jsonl"),
                "questions_file": path("questions.json"), "state_field": "body", "id_field": "id",
            }),
        ),
        (
            "evaluate",
            json!({ "state": STATE, "questions": { "q": { "type": "score", "criteria": [] } } }),
        ),
        (
            "batch_run",
            json!({ "input": "/nowhere/rows.jsonl", "out": path("x.jsonl") }),
        ),
        ("no_such_tool", json!({ "state": STATE })),
    ];
    let mut lines = vec![
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "leak-test", "version": "1" } } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    ];
    for (n, (tool, arguments)) in calls.into_iter().enumerate() {
        lines.push(
            json!({ "jsonrpc": "2.0", "id": 10 + n, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments } }),
        );
    }
    lines.iter().fold(String::new(), |mut session, line| {
        session.push_str(&line.to_string());
        session.push('\n');
        session
    })
}

/// Every command line the sweep runs beyond the automatic `--help`, bare run and unknown flag.
fn scenarios(sandbox: &Sandbox, api: &str) -> Vec<Scenario> {
    let mut scenarios = evaluation_scenarios();
    scenarios.extend(batch_scenarios(sandbox));
    scenarios.extend(account_scenarios(api));
    scenarios.extend(tooling_scenarios(sandbox));
    scenarios
}

/// `jev eval` and the three shortcuts, which send `state` to the API.
fn evaluation_scenarios() -> Vec<Scenario> {
    vec![
        online(&["eval", "-f", "request.json"]).expect(0),
        online(&["eval", "-f", "request.json", "--raw"]).expect(0),
        online(&[
            "eval",
            "-f",
            "questions.json",
            "--state-file",
            "state.txt",
            "--assert",
            "is_urgent >= 0.5",
        ])
        .expect(0),
        offline(&["eval", "-f", "request.json", "--dry-run"]).expect(0),
        offline(&["eval", "-f", "invalid.json", "--state", STATE]).expect(2),
        offline(&["eval", "-f", "missing.json"]).expect(2),
        online(&["noul", "Is it urgent?", "--state-file", "state.txt"]).expect(0),
        online(&[
            "noul",
            "Is it urgent?",
            "--state",
            STATE,
            "--fail-under",
            "0.5",
            "--raw",
        ])
        .expect(0),
        online(&[
            "noul",
            "Is it urgent?",
            "--state",
            STATE,
            "--fail-under",
            "0.95",
        ])
        .expect(10),
        offline(&["noul", "Is it urgent?", "--state", STATE, "--dry-run"]).expect(0),
        online(&[
            "choice",
            "Which team?",
            "--state-file",
            "state.txt",
            "--option",
            "billing",
            "--option",
            "other",
        ])
        .expect(0),
        online(&[
            "choice",
            "Which team?",
            "--state",
            STATE,
            "--option",
            "billing",
            "--option",
            "other",
            "--expect",
            "billing",
        ])
        .expect(0),
        offline(&[
            "choice",
            "Which team?",
            "--state",
            STATE,
            "--option",
            "billing",
            "--dry-run",
        ]),
        online(&[
            "score",
            "How angry?",
            "--state-file",
            "state.txt",
            "--level",
            "Calm",
            "--level",
            "Angry",
        ])
        .expect(0),
        offline(&[
            "score",
            "How angry?",
            "--state",
            STATE,
            "--level",
            "Calm",
            "--level",
            "Angry",
            "--dry-run",
        ])
        .expect(0),
        offline(&["score", "How angry?", "--state", STATE, "--level", "Only"]).expect(2),
    ]
}

/// `jev validate` and `jev batch run`, which read files and write results.
fn batch_scenarios(sandbox: &Sandbox) -> Vec<Scenario> {
    let rows = fs::read_to_string(sandbox.work().join("rows.jsonl")).unwrap();
    vec![
        offline(&["validate", "-f", "request.json"]).expect(0),
        offline(&[
            "validate",
            "-f",
            "invalid.json",
            "--state",
            STATE,
            "--strict",
        ])
        .expect(2),
        online(&[
            "batch",
            "run",
            "-f",
            "questions.json",
            "--input",
            "rows.jsonl",
            "--state-field",
            "body",
            "--id-field",
            "id",
            "--out",
            "out.jsonl",
            "--summary-json",
            "summary.json",
        ])
        .expect(0),
        online(&[
            "batch",
            "run",
            "-f",
            "questions.json",
            "--input",
            "rows.jsonl",
            "--state-field",
            "body",
            "--id-field",
            "id",
            "--out",
            "out.jsonl",
            "--resume",
            "--summary-json",
            "summary.json",
        ])
        .expect(0),
        online(&[
            "batch",
            "run",
            "-f",
            "questions.json",
            "--state-field",
            "body",
            "--out",
            "piped.jsonl",
            "--fail-fast",
        ])
        .stdin(rows.clone()),
        online(&[
            "batch",
            "run",
            "-f",
            "questions.json",
            "--state-field",
            "body",
            "--ordered",
        ])
        .stdin(rows.clone()),
        offline(&[
            "batch",
            "run",
            "-f",
            "questions.json",
            "--input",
            "rows.jsonl",
            "--state-field",
            "body",
            "--dry-run",
        ])
        .expect(0),
        online(&["models", "list"]).expect(0),
    ]
}

/// The commands that hold or show a key, and the profiles and settings around them.
fn account_scenarios(api: &str) -> Vec<Scenario> {
    vec![
        offline(&["profile", "create", "staging", "--base-url", api]).expect(0),
        offline(&[
            "auth",
            "login",
            "--with-token",
            "--skip-verify",
            "--profile",
            "staging",
        ])
        .stdin(format!("{KEY}\n"))
        .expect(0),
        online(&["auth", "login", "--with-token"]).stdin(KEY),
        offline(&["auth", "login"]).expect(2),
        offline(&[
            "auth",
            "login",
            "--with-token",
            "--profile",
            "staging",
            "--skip-verify",
        ])
        .stdin(UNUSABLE_KEY),
        online(&["auth", "status"]),
        offline(&["auth", "status", "--offline"]),
        offline(&["auth", "status", "--offline", "--profile", "staging"]).expect(0),
        offline(&["config", "set", "timeout", "45s"]).expect(0),
        offline(&["config", "get", "timeout"]).expect(0),
        offline(&["config", "unset", "timeout"]).expect(0),
        offline(&["config", "set", "api_key", KEY]),
        offline(&["config", "list"]).expect(0),
        offline(&["config", "path"]).expect(0),
        offline(&["profile", "list"]).expect(0),
        offline(&["profile", "use", "staging"]).expect(0),
        online(&["models", "list"]).expect(0),
        offline(&["profile", "use", "default"]).expect(0),
        offline(&["auth", "logout", "--profile", "staging"]).expect(0),
        offline(&["auth", "logout", "--profile", "never-created"]),
        offline(&["profile", "delete", "staging"]).expect(0),
        offline(&["profile", "use", "never-created"]),
    ]
}

/// What agents and packagers use: the schemas, the spec, the MCP server, updates and completions.
fn tooling_scenarios(sandbox: &Sandbox) -> Vec<Scenario> {
    let allow = sandbox.work().join("data");
    let allow = allow.to_str().unwrap();
    vec![
        offline(&["schema", "request"]).expect(0),
        offline(&["schema", "questions"]).expect(0),
        offline(&["schema", "batch-record"]).expect(0),
        offline(&["schema", "output"]).expect(0),
        offline(&["schema", "error"]).expect(0),
        offline(&["spec"]).expect(0),
        online(&["mcp", "serve"])
            .stdin(mcp_session(sandbox))
            .expect(0),
        online(&[
            "mcp",
            "serve",
            "--allow-dir",
            allow,
            "--max-batch-rows",
            "10",
        ])
        .stdin(mcp_session(sandbox))
        .expect(0),
        offline(&["update", "--check"]),
        offline(&["update"]),
        offline(&["update", "--rollback"]),
        offline(&["update", "--version", "0.0.1"]),
        offline(&["update", "--background"]),
        offline(&["completion", "bash"]).expect(0),
        offline(&["completion", "zsh"]).expect(0),
        offline(&["version"]).expect(0),
    ]
}

/// The command paths `jev spec` lists, such as `["batch", "run"]`.
fn spec_paths() -> Vec<Vec<String>> {
    let output = Command::cargo_bin("jev")
        .unwrap()
        .args(["spec", "-o", "json"])
        .output()
        .unwrap();
    let spec: Value = serde_json::from_slice(&output.stdout).unwrap();
    spec["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| {
            command["path"]
                .as_str()
                .unwrap()
                .split(' ')
                .map(ToOwned::to_owned)
                .collect()
        })
        .collect()
}

/// The mock API, the stand-in for GitHub and the proxy trap one sweep shares.
struct Servers {
    api: MockServer,
    mode: Arc<Mutex<Mode>>,
    github: MockServer,
    trap: Trap,
}

impl Servers {
    async fn start() -> Self {
        let mode = Arc::new(Mutex::new(Mode::Answer));
        let api = MockServer::start().await;
        Mock::given(any())
            .respond_with(Api(Arc::clone(&mode)))
            .mount(&api)
            .await;
        // No release is published: nothing can be installed over the binary under test.
        let github = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })),
            )
            .mount(&github)
            .await;
        Self {
            api,
            mode,
            github,
            trap: Trap::start(),
        }
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// `jev` in the sandbox, isolated from the environment of whoever runs the tests.
fn jev(sandbox: &Sandbox, servers: &Servers, source: KeySource) -> Command {
    let mut command = Command::cargo_bin("jev").unwrap();
    command.env_clear();
    // Only what a process needs to run at all: the environment is otherwise this test's.
    for variable in [
        "PATH",
        "HOME",
        "TMPDIR",
        "COMSPEC",
        "PATHEXT",
        "SYSTEMDRIVE",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "WINDIR",
    ] {
        if let Some(value) = std::env::var_os(variable) {
            command.env(variable, value);
        }
    }
    for variable in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(variable, &servers.trap.address);
    }
    for variable in ["NO_PROXY", "no_proxy"] {
        command.env(variable, "127.0.0.1,localhost");
    }
    command
        .current_dir(sandbox.work())
        .env("JEV_CONFIG_DIR", sandbox.config())
        .env("TYPESAFE_BASE_URL", servers.api.uri())
        .env("JEV_TEST_UPDATE_URL", servers.github.uri())
        .env("JEV_AUTO_UPDATE", "false");
    if let Some(key) = source.in_the_environment() {
        command.env("TYPESAFE_API_KEY", key);
    }
    command
}

/// Runs `jev` off the async runtime's threads, since the mock servers live on them.
async fn run(mut command: Command, stdin: String) -> Run {
    tokio::task::spawn_blocking(move || {
        let output = command
            .write_stdin(stdin)
            .timeout(Duration::from_secs(60))
            .output()
            .unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    })
    .await
    .unwrap()
}

/// Stores the key in the credentials file with `jev auth login`, as a person would.
async fn log_in(sandbox: &Sandbox, servers: &Servers) {
    // `jev` will not write over a file others can read either, so start from none.
    let _ = fs::remove_file(sandbox.credentials());
    let mut command = jev(sandbox, servers, KeySource::CredentialsFile);
    command.args(["auth", "login", "--with-token", "--skip-verify"]);
    let login = run(command, format!("{KEY}\n")).await;
    assert_eq!(login.code, 0, "{}", login.stderr);
    assert!(
        fs::read_to_string(sandbox.credentials())
            .unwrap()
            .contains(KEY)
    );
}

/// Makes the credentials file readable by others, which `jev` must refuse without quoting it.
#[cfg(unix)]
fn expose(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt;
    if sandbox.credentials().exists() {
        fs::set_permissions(sandbox.credentials(), fs::Permissions::from_mode(0o644)).unwrap();
    }
}

/// Asserts that nothing `jev` showed or wrote holds the key, or `state` where it does not belong.
fn assert_clean(sandbox: &Sandbox, what: &str, run: &Run, bodies: bool) {
    let context = || {
        format!(
            "{what}\n--- stdout\n{}\n--- stderr\n{}",
            run.stdout, run.stderr
        )
    };
    assert!(
        !run.stdout.contains(KEY_MARK),
        "the key leaked to stdout: {}",
        context()
    );
    assert!(
        !run.stderr.contains(KEY_MARK),
        "the key leaked to stderr: {}",
        context()
    );
    if !bodies {
        assert!(
            !run.stderr.contains(STATE),
            "`state` reached stderr without --debug-bodies: {}",
            context()
        );
    }
    for file in files(&sandbox.root) {
        let text = String::from_utf8_lossy(&fs::read(&file).unwrap_or_default()).into_owned();
        if file != sandbox.credentials() {
            assert!(
                !text.contains(KEY_MARK),
                "the key was written to {}: {}",
                file.display(),
                context()
            );
        }
        assert!(
            !text.contains(STATE) || sandbox.may_hold_state(&file),
            "`state` was written to {}, which nobody named: {}",
            file.display(),
            context()
        );
    }
}

/// What one sweep saw, to check it tested what it meant to.
#[derive(Default)]
struct Tally {
    codes: Vec<(Mode, i32)>,
}

/// Runs every command with the key from `source`, and asserts that nothing leaked.
async fn sweep(source: KeySource) {
    let sandbox = Sandbox::new(&format!("{source:?}").to_lowercase());
    let servers = Servers::start().await;
    let scenarios = scenarios(&sandbox, &servers.api.uri());

    // Every command `jev spec` lists needs a scenario of its own, beyond the automatic ones.
    let paths = spec_paths();
    let uncovered: Vec<String> = paths
        .iter()
        .filter(|path| {
            !scenarios
                .iter()
                .any(|scenario| scenario.args.starts_with(path.as_slice()))
        })
        .map(|path| path.join(" "))
        .collect();
    assert!(
        uncovered.is_empty(),
        "add a scenario to `scenarios` in tests/secret_leak.rs for: {uncovered:?}"
    );
    let mut automatic = Vec::new();
    for path in &paths {
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        automatic.push(offline(&[path.as_slice(), &["--help"]].concat()));
        automatic.push(offline(&path));
        automatic.push(offline(
            &[path.as_slice(), &["--no-such-flag", KEY]].concat(),
        ));
    }

    let modes: &[Mode] = if source.usable() {
        &[Mode::Answer, Mode::Unauthorized, Mode::Rejected]
    } else {
        &[Mode::Answer]
    };
    let mut tally = Tally::default();
    for &mode in modes {
        *servers.mode.lock().unwrap() = mode;
        // Text with everything but bodies logged, then JSON with bodies logged as well.
        for (bodies, globals) in [
            (false, ["-vvv", "-o", "table"].as_slice()),
            (true, ["-vvv", "--debug-bodies", "-o", "json"].as_slice()),
        ] {
            sandbox.clear_outputs();
            // The bare `jev auth logout` of the previous pass removed the stored key.
            if source.stored_in_a_file() {
                log_in(&sandbox, &servers).await;
            }
            let extra = if mode == Mode::Answer && !bodies {
                automatic.as_slice()
            } else {
                &[]
            };
            for scenario in scenarios.iter().chain(extra) {
                if mode != Mode::Answer && !scenario.network {
                    continue;
                }
                #[cfg(unix)]
                if source == KeySource::ExposedCredentialsFile {
                    expose(&sandbox);
                }
                let mut command = jev(&sandbox, &servers, source);
                command.args(globals).args(&scenario.args);
                let result = run(command, scenario.stdin.clone()).await;
                let what = format!(
                    "{source:?} {mode:?} jev {} {}",
                    globals.join(" "),
                    scenario.args.join(" ")
                );
                assert_clean(&sandbox, &what, &result, bodies);
                if let (Mode::Answer, true, Some(expected)) =
                    (mode, source.usable(), scenario.expect)
                {
                    assert_eq!(result.code, expected, "{what}\n{}", result.stderr);
                }
                tally.codes.push((mode, result.code));
            }
        }
    }

    // The sweep reached the API and saw each kind of answer it meant to.
    if source.usable() {
        for (mode, code) in [
            (Mode::Answer, 0),
            (Mode::Unauthorized, 3),
            (Mode::Rejected, 4),
        ] {
            assert!(
                tally.codes.contains(&(mode, code)),
                "no command exited {code} against {mode:?}"
            );
        }
    }
    assert_network(&servers, source).await;
    assert_mcp_tools_are_covered(&sandbox, &servers, source).await;
}

/// Every request went to the configured API or the stand-in for GitHub, carried the key only to
/// the API, and nothing reached any other host.
async fn assert_network(servers: &Servers, source: KeySource) {
    let api = servers.api.received_requests().await.unwrap();
    // Without a usable key only `jev auth login`, given the key on stdin, reaches the API.
    assert!(
        !source.usable() || api.len() > 20,
        "{source:?}: {} API calls",
        api.len()
    );
    for request in &api {
        assert!(
            ["/v1/systemone", "/v1/models"].contains(&request.url.path()),
            "an unexpected API call: {} {}",
            request.method,
            request.url
        );
        let authorization = request.headers.get("authorization").unwrap();
        assert_eq!(authorization.to_str().unwrap(), format!("Bearer {KEY}"));
    }
    let github = servers.github.received_requests().await.unwrap();
    assert!(!github.is_empty(), "the updater was never exercised");
    for request in &github {
        assert!(
            request.headers.get("authorization").is_none(),
            "{}",
            request.url
        );
        let everything = format!(
            "{} {:?} {}",
            request.url,
            request.headers,
            String::from_utf8_lossy(&request.body)
        );
        assert!(
            !everything.contains(KEY_MARK),
            "the key was sent to GitHub: {everything}"
        );
    }
    assert_eq!(
        servers.trap.connections.load(Ordering::SeqCst),
        0,
        "jev tried to reach a host that is neither the API nor GitHub Releases: {:?}",
        servers.trap.seen.lock().unwrap()
    );
}

/// The MCP server lists no tool the sweep does not call.
async fn assert_mcp_tools_are_covered(sandbox: &Sandbox, servers: &Servers, source: KeySource) {
    let allow = sandbox.work().join("data");
    let mut command = jev(sandbox, servers, source);
    command.args(["mcp", "serve", "--allow-dir", allow.to_str().unwrap()]);
    let session = mcp_session(sandbox);
    let listing: String = session
        .lines()
        .take(3)
        .fold(String::new(), |mut lines, line| {
            lines.push_str(line);
            lines.push('\n');
            lines
        });
    let served = run(command, listing).await;
    let tools: Vec<String> = served
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| message["id"] == 2)
        .flat_map(|message| {
            message["result"]["tools"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|tool| tool["name"].as_str().map(ToOwned::to_owned))
        .collect();
    assert!(!tools.is_empty(), "{}", served.stderr);
    for tool in tools {
        assert!(
            MCP_TOOLS.contains(&tool.as_str()),
            "add the MCP tool `{tool}` to `mcp_session` in tests/secret_leak.rs"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn no_command_leaks_a_key_from_the_environment() {
    sweep(KeySource::Env).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_command_leaks_a_key_from_the_credentials_file() {
    sweep(KeySource::CredentialsFile).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_command_leaks_a_key_it_cannot_use() {
    sweep(KeySource::UnusableEnv).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn no_command_leaks_a_key_from_a_credentials_file_others_can_read() {
    sweep(KeySource::ExposedCredentialsFile).await;
}

/// The trap is only a net if `jev` goes through it: a host that is not loopback must reach it.
#[tokio::test(flavor = "multi_thread")]
async fn the_proxy_trap_catches_a_host_that_is_not_allowed() {
    let sandbox = Sandbox::new("trap");
    let servers = Servers::start().await;
    let mut command = jev(&sandbox, &servers, KeySource::Env);
    command.args([
        "--base-url",
        "https://api.typesafe.invalid",
        "--max-retries",
        "0",
        "models",
        "list",
    ]);

    let result = run(command, String::new()).await;

    assert_eq!(result.code, 6, "{}", result.stderr);
    assert!(servers.trap.connections.load(Ordering::SeqCst) >= 1);
    assert_clean(&sandbox, "trap", &result, false);
}

/// Only two places in the source can open a connection: the API transport, and the client for
/// this repository's GitHub Releases (SEC-6). A third needs a review of this test and of
/// SECURITY.md, not an entry in the allow-list.
#[test]
fn only_the_api_transport_and_the_release_client_can_reach_the_network() {
    const ALLOWED: [&str; 2] = ["jev-client/src/http.rs", "jev-cli/src/update/github.rs"];
    const NETWORK: [&str; 10] = [
        "Client::builder",
        "ClientBuilder",
        "reqwest::get",
        "reqwest::blocking",
        "TcpStream",
        "UdpSocket",
        "TcpListener",
        "tokio::net",
        "Command::new(\"curl",
        "Command::new(\"wget",
    ];
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sources = files(&crates.join("jev-client/src"));
    sources.extend(files(&crates.join("jev-cli/src")));
    let mut seen = Vec::new();
    for source in sources
        .iter()
        .filter(|path| path.extension().is_some_and(|x| x == "rs"))
    {
        let text = fs::read_to_string(source).unwrap();
        let relative = source
            .strip_prefix(crates)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        for pattern in NETWORK.iter().filter(|pattern| text.contains(*pattern)) {
            seen.push(relative.clone());
            assert!(
                ALLOWED.contains(&relative.as_str()),
                "{relative} uses `{pattern}`: only {ALLOWED:?} may reach the network"
            );
        }
    }
    for allowed in ALLOWED {
        assert!(
            seen.iter().any(|file| file == allowed),
            "{allowed} no longer builds a client; update this test"
        );
    }
    // And the hosts they name are the API's and GitHub's.
    let hosts: Vec<String> = ALLOWED
        .iter()
        .flat_map(|file| hosts_named(&fs::read_to_string(crates.join(file)).unwrap()))
        .collect();
    for host in &hosts {
        assert!(
            [
                "api.typesafe.ai",
                "api.github.com",
                "github.com",
                "objects.githubusercontent.com",
                "release-assets.githubusercontent.com",
            ]
            .contains(&host.as_str()),
            "{host} is named in {ALLOWED:?}"
        );
    }
    assert!(
        hosts.iter().any(|host| host == "api.github.com"),
        "{hosts:?}"
    );
}

/// The hosts in the string literals of `source`: in a URL, or on their own.
fn hosts_named(source: &str) -> Vec<String> {
    source
        .split('"')
        .skip(1)
        .step_by(2)
        .filter_map(|literal| {
            let host = literal.strip_prefix("https://").unwrap_or(literal);
            let host = host.split(['/', ':', ' ', '`']).next().unwrap_or_default();
            let is_host = host.contains('.')
                && host
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
                && host.rsplit('.').next().is_some_and(|tld| {
                    tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_lowercase())
                });
            (is_host && (literal.starts_with("https://") || literal == host))
                .then(|| host.to_owned())
        })
        .collect()
}

/// The detached `jev update --background` that automatic updates start never gets the key.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn the_background_update_check_is_started_without_the_key() {
    let sandbox = Sandbox::new("background");
    // A copy of `jev` as the install script installs it: the only kind that updates itself.
    let bin = sandbox.root.join("local/bin");
    fs::create_dir_all(bin.join(".jev-update")).unwrap();
    fs::write(bin.join(".jev-update/receipt.json"), "").unwrap();
    let exe = bin.join("jev");
    fs::copy(assert_cmd::cargo::cargo_bin("jev"), &exe).unwrap();
    wait_until_spawnable(&exe).await;
    // GitHub answers slowly, so the check is still running while its environment is read.
    let github = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(404).set_delay(Duration::from_secs(5)))
        .mount(&github)
        .await;
    let mut command = Command::new(&exe);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("JEV_CONFIG_DIR", sandbox.config())
        .env("JEV_TEST_UPDATE_URL", github.uri())
        .env("JEV_TEST_VERSION", "0.1.0")
        .env("TYPESAFE_API_KEY", KEY)
        .args(["config", "path"]);

    let result = run(command, String::new()).await;

    assert_eq!(result.code, 0, "{}", result.stderr);
    let started = std::time::Instant::now();
    while github.received_requests().await.unwrap().is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no background check started"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let check = background_check(&exe).expect("the background check is running");
    let environment = fs::read(format!("/proc/{check}/environ")).unwrap();
    let environment = String::from_utf8_lossy(&environment);
    let names: Vec<&str> = environment
        .split('\0')
        .filter_map(|variable| variable.split('=').next())
        .collect();
    assert!(
        names.contains(&"JEV_TEST_UPDATE_URL"),
        "read the wrong process: {names:?}"
    );
    assert!(!names.contains(&"TYPESAFE_API_KEY"), "{names:?}");
    assert!(
        !environment.contains(KEY_MARK),
        "the key is in the environment of {names:?}"
    );

    // Nor does it write the key anywhere, or send it to GitHub.
    while !fs::read_to_string(sandbox.config().join("auto-update.json"))
        .is_ok_and(|state| state.contains("last_outcome"))
    {
        assert!(
            started.elapsed() < Duration::from_secs(90),
            "the check never finished"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for file in files(&sandbox.config()) {
        let text = String::from_utf8_lossy(&fs::read(&file).unwrap()).into_owned();
        assert!(!text.contains(KEY_MARK), "{}", file.display());
    }
    for request in github.received_requests().await.unwrap() {
        assert!(request.headers.get("authorization").is_none());
        assert!(!format!("{request:?}").contains(KEY_MARK));
    }
}

/// `fs::copy` can return before a CI runner's virus scanner has released the executable it just
/// wrote, so the very next `exec` of a freshly copied binary can transiently fail with
/// `ETXTBSY` ("Text file busy"). Run a throwaway `--help` until the kernel is actually ready to
/// spawn it, instead of letting that race hit the real test command. Off the async runtime's
/// threads, like `run()`, since the retry loop blocks on the child process.
#[cfg(target_os = "linux")]
async fn wait_until_spawnable(exe: &Path) {
    let exe = exe.to_owned();
    tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match std::process::Command::new(&exe).arg("--help").output() {
                Ok(_) => return,
                Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{} stayed busy: {err}",
                        exe.display()
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(err) => panic!("failed to warm up {}: {err}", exe.display()),
            }
        }
    })
    .await
    .unwrap();
}

/// The process id of `exe update --background`.
#[cfg(target_os = "linux")]
fn background_check(exe: &Path) -> Option<u32> {
    let wanted = format!("{}\0update\0--background\0", exe.display());
    fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        let command_line = fs::read(entry.path().join("cmdline")).ok()?;
        (command_line == wanted.as_bytes()).then_some(pid)
    })
}
