//! `jev mcp serve`: the Model Context Protocol over stdio, against a local mock of the API.

// Clippy's `allow-unwrap-in-tests`, `allow-panic-in-tests` and `allow-indexing-slicing-in-tests`
// cover `#[test]` functions only, not the helper functions an integration test shares, where
// failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::path::Path;
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A key that must never appear in any output. It is not a real credential.
const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-7d02";

/// `jev mcp serve`, isolated from the environment of whoever runs the tests, pointed at `server`.
fn serve(server: Option<&MockServer>, extra: &[&str]) -> Command {
    let mut command = Command::cargo_bin("jev").unwrap();
    for variable in [
        "CI",
        "NO_COLOR",
        "TERM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "JEV_OUTPUT",
        "JEV_PROFILE",
        "JEV_NO_INPUT",
        "TYPESAFE_API_KEY",
        "TYPESAFE_BASE_URL",
        "TYPESAFE_DEFAULT_MODEL",
        "TYPESAFE_LOG_LEVEL",
    ] {
        command.env_remove(variable);
    }
    command.env(
        "JEV_CONFIG_DIR",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
    );
    command.env("TYPESAFE_API_KEY", SENTINEL_KEY);
    // An address nothing listens on, so a test that forgets its mock cannot reach the real API.
    command.env(
        "TYPESAFE_BASE_URL",
        server.map_or_else(|| "http://127.0.0.1:9".to_owned(), MockServer::uri),
    );
    command.args(["mcp", "serve"]).args(extra);
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Every line on stdout, each of which must be a JSON-RPC 2.0 message.
    fn messages(&self) -> Vec<Value> {
        self.stdout
            .lines()
            .map(|line| {
                let message: Value = serde_json::from_str(line)
                    .unwrap_or_else(|error| panic!("not JSON ({error}): {line:?}"));
                assert_eq!(message["jsonrpc"], "2.0", "{line}");
                assert!(
                    message.get("result").is_some() != message.get("error").is_some(),
                    "a response has a result or an error: {line}"
                );
                message
            })
            .collect()
    }

    /// The response to the request with `id`.
    fn response(&self, id: u64) -> Value {
        self.messages()
            .into_iter()
            .find(|message| message["id"] == id)
            .unwrap_or_else(|| panic!("no response to {id}: {}", self.stdout))
    }

    /// The result of the tool call with `id`, and whether it is a tool error.
    fn tool(&self, id: u64) -> (Value, bool) {
        let result = &self.response(id)["result"];
        let structured = result["structuredContent"].clone();
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            text, structured,
            "the text content is the structured content, for older clients"
        );
        (structured, result["isError"].as_bool().unwrap())
    }
}

/// Sends `messages`, one per line, then closes stdin, which ends the server.
async fn run(mut command: Command, messages: &[Value]) -> Run {
    let mut input = String::new();
    for message in messages {
        input.push_str(&message.to_string());
        input.push('\n');
    }
    tokio::task::spawn_blocking(move || {
        let output = command
            .write_stdin(input)
            .timeout(Duration::from_secs(60))
            .output()
            .unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    })
    .await
    .unwrap()
}

fn initialize() -> Value {
    json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": { "name": "test", "version": "1" }
    }})
}

fn initialized() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
}

fn call(id: u64, tool: &str, arguments: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": arguments } })
}

/// The API, answering every evaluation with `answers` and listing two models.
async fn api(answers: Value) -> MockServer {
    let server = MockServer::start().await;
    let body = json!({ "model": "jev-1.13.0", "answers": answers, "usage": { "input_tokens": 1000, "output_tokens": 20 } });
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "req_mcp")
                .set_body_json(body),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "models": [{ "name": "jev-latest" }, { "name": "jev-preview" }] }),
        ))
        .mount(&server)
        .await;
    server
}

async fn sent(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method.as_str() == "POST")
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

fn assert_no_key(run: &Run) {
    assert!(
        !run.stdout.contains(SENTINEL_KEY),
        "the key leaked to stdout: {}",
        run.stdout
    );
    assert!(
        !run.stderr.contains(SENTINEL_KEY),
        "the key leaked to stderr: {}",
        run.stderr
    );
}

fn triage() -> Value {
    json!({
        "state": "Help! My payouts have been failing for 3 days.",
        "questions": {
            "is_urgent": { "type": "noul", "instructions": "Does this convey urgency?" },
            "team": { "type": "choice", "instructions": "Which team should handle this?",
                      "criteria": { "billing": "Payments", "technical": "Bugs", "other": null } }
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_can_initialise_list_the_tools_and_ping() {
    let run = run(
        serve(None, &[]),
        &[
            initialize(),
            initialized(),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": "two", "method": "ping" }),
        ],
    )
    .await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stderr, "");
    assert_eq!(
        run.messages().len(),
        3,
        "a notification is never answered: {}",
        run.stdout
    );
    let init = &run.response(0)["result"];
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "jev");
    assert_eq!(init["capabilities"]["tools"]["listChanged"], false);
    assert!(
        init["instructions"]
            .as_str()
            .unwrap()
            .contains("arithmetic")
    );

    let tools = run.response(1)["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "evaluate",
            "noul",
            "choice",
            "score",
            "validate",
            "list_models"
        ]
    );
    for tool in tools.as_array().unwrap() {
        assert!(!tool["description"].as_str().unwrap().is_empty());
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
    let ping = run
        .messages()
        .into_iter()
        .find(|message| message["id"] == "two")
        .unwrap();
    assert_eq!(
        ping["result"],
        json!({}),
        "ids are echoed as they were sent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_protocol_version_is_offered_the_newest() {
    let mut hello = initialize();
    hello["params"]["protocolVersion"] = json!("1999-01-01");

    let run = run(serve(None, &[]), &[hello]).await;

    assert_eq!(run.response(0)["result"]["protocolVersion"], "2025-11-25");
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_mistakes_are_json_rpc_errors_and_the_server_keeps_going() {
    let run = run(
        serve(None, &[]),
        &[
            json!("not an object"),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/list" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "nope" } }),
            json!({ "jsonrpc": "2.0", "id": 3 }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" }),
        ],
    )
    .await;

    assert_eq!(run.code, 0);
    assert_eq!(run.messages()[0]["error"]["code"], -32600);
    assert_eq!(run.messages()[0]["id"], Value::Null);
    assert_eq!(run.response(1)["error"]["code"], -32601);
    assert_eq!(run.response(2)["error"]["code"], -32602);
    assert!(
        run.response(2)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nope")
    );
    assert_eq!(run.response(3)["error"]["code"], -32600);
    assert_eq!(run.response(4)["result"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_line_that_is_not_json_is_a_parse_error() {
    let mut command = serve(None, &[]);
    let output = tokio::task::spawn_blocking(move || {
        command
            .write_stdin("{not json\n\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(lines.len(), 2, "a blank line is skipped: {stdout}");
    assert_eq!(lines[0]["error"]["code"], -32700);
    assert_eq!(lines[1]["result"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn every_tool_answers_and_the_session_counts_the_spend() {
    let server = api(json!({
        "is_urgent": { "type": "noul", "noul": 0.9 },
        "team": { "type": "choice", "choice": "billing", "confidence": 0.8,
                  "probabilities": { "billing": 0.85, "technical": 0.1, "other": 0.05 } },
        "answer": { "type": "noul", "noul": 0.25 }
    }))
    .await;

    let run = run(
        serve(Some(&server), &[]),
        &[
            initialize(),
            initialized(),
            call(1, "evaluate", &triage()),
            call(
                2,
                "noul",
                &json!({ "state": "Where is my refund?", "instructions": "Is the customer angry?",
                         "criteria": { "true": "Angry", "false": "Calm" } }),
            ),
            call(
                3,
                "choice",
                &json!({ "state": "s", "instructions": "Which team?", "criteria": { "billing": null, "technical": null } }),
            ),
            call(
                4,
                "score",
                &json!({ "state": "s", "instructions": "How angry?", "criteria": ["Calm", "Angry"], "model": "jev-1.13.0" }),
            ),
            call(5, "validate", &json!({ "questions": triage()["questions"] })),
            call(6, "list_models", &json!({})),
        ],
    )
    .await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stderr, "", "warnings belong in the results");
    assert_no_key(&run);

    let (evaluate, failed) = run.tool(1);
    assert!(!failed, "{evaluate}");
    assert_eq!(evaluate["model"], "jev-1.13.0");
    assert_eq!(evaluate["requested_model"], "jev-latest");
    assert_eq!(evaluate["request_id"], "req_mcp");
    assert_eq!(evaluate["answers"]["is_urgent"]["noul"], 0.9);
    assert_eq!(evaluate["usage"]["input_tokens"], 1000);
    assert_eq!(evaluate["session"]["calls"], 1);
    assert!(evaluate.get("warnings").is_none(), "{evaluate}");

    let (noul, failed) = run.tool(2);
    assert!(!failed, "{noul}");
    assert_eq!(
        noul["noul"], 0.25,
        "a shortcut's answer is at the top level, as `jev noul -o json` prints it"
    );
    assert!(noul.get("answers").is_none(), "{noul}");
    assert_eq!(noul["session"]["calls"], 2);

    let (choice, failed) = run.tool(3);
    assert!(!failed, "{choice}");
    assert_eq!(
        choice["warnings"][0]["rule"], "choice-no-escape-option",
        "a lint reaches the agent in the result, not on stderr: {choice}"
    );

    let (score, failed) = run.tool(4);
    assert!(!failed, "{score}");
    assert_eq!(score["requested_model"], "jev-1.13.0");
    assert_eq!(score["session"]["calls"], 4);
    let spent = score["session"]["estimated_cost_usd"].as_f64().unwrap();
    assert!(
        (spent - 4.0 * 0.000_042).abs() < 1e-12,
        "four calls of 1000 tokens at $0.042 per million: {spent}"
    );
    assert_eq!(score["session"]["unpriced_calls"], 0);
    assert_eq!(score["session"]["max_cost_usd_per_call"], Value::Null);

    let (validate, failed) = run.tool(5);
    assert!(!failed, "{validate}");
    assert_eq!(validate["valid"], true);

    let (models, failed) = run.tool(6);
    assert!(!failed, "{models}");
    assert_eq!(models["models"][0]["name"], "jev-latest");

    let bodies = sent(&server).await;
    assert_eq!(bodies.len(), 4);
    assert_eq!(bodies[0], {
        let mut expected = triage();
        expected["model"] = json!("jev-latest");
        expected
    });
    assert_eq!(
        bodies[1],
        json!({ "state": "Where is my refund?", "model": "jev-latest", "questions": { "answer": {
            "type": "noul", "instructions": "Is the customer angry?", "criteria": { "true": "Angry", "false": "Calm" }
        }}})
    );
    assert_eq!(
        bodies[3]["questions"]["answer"]["criteria"],
        json!(["Calm", "Angry"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_request_is_a_tool_error_with_every_finding_and_nothing_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let levels: Vec<String> = (0..11).map(|level| format!("Level {level}")).collect();

    let run = run(
        serve(Some(&server), &[]),
        &[
            initialize(),
            call(
                1,
                "evaluate",
                &json!({ "questions": { "rating": { "type": "score", "instructions": "?", "criteria": levels } } }),
            ),
            call(2, "score", &json!({ "state": "s", "instructions": "?", "criteria": ["only one"] })),
            call(3, "noul", &json!({ "state": "s", "question": "Is it?" })),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "noul", "arguments": [1] } }),
        ],
    )
    .await;

    assert_eq!(run.code, 0, "a failed call does not end the server");
    assert_eq!(run.stderr, "");
    let (error, failed) = run.tool(1);
    assert!(failed);
    let error = &error["error"];
    assert_eq!(error["code"], "usage");
    assert_eq!(error["exit_code"], 2);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .ends_with("2 problems found, nothing was sent"),
        "{error}"
    );
    let rules: Vec<&str> = error["details"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["rule"].as_str().unwrap())
        .collect();
    assert_eq!(rules, ["state-missing", "score-too-many-levels"]);

    let (error, failed) = run.tool(2);
    assert!(failed);
    assert_eq!(
        error["error"]["details"]["findings"][0]["rule"],
        "score-too-few-levels"
    );

    let (error, failed) = run.tool(3);
    assert!(failed);
    assert_eq!(
        error["error"]["message"],
        "`noul` has no argument `question`"
    );

    let (error, failed) = run.tool(4);
    assert!(failed);
    assert_eq!(error["error"]["code"], "usage");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_estimated_above_the_limit_is_refused_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let state = "word ".repeat(2000);

    let run = run(
        serve(
            Some(&server),
            &[
                "--model",
                "jev-1.13.0",
                "--max-cost-usd-per-call",
                "0.00001",
            ],
        ),
        &[
            initialize(),
            call(
                1,
                "noul",
                &json!({ "state": state, "instructions": "Is it long?" }),
            ),
        ],
    )
    .await;

    assert_eq!(run.code, 0);
    let (error, failed) = run.tool(1);
    assert!(failed);
    let error = &error["error"];
    assert_eq!(error["code"], "cost_limit");
    assert_eq!(error["exit_code"], 2);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("above the limit of $0.000010 per call; nothing was sent"),
        "{error}"
    );
    let estimate = error["details"]["estimated_cost_usd"].as_f64().unwrap();
    assert!(estimate > 0.000_01, "{estimate}");
    assert_eq!(error["details"]["max_cost_usd_per_call"], 0.000_01);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_on_a_model_without_a_price_refuses_and_a_generous_one_lets_calls_through() {
    let server = api(json!({ "answer": { "type": "noul", "noul": 0.5 } })).await;

    let unpriced = run(
        serve(Some(&server), &["--max-cost-usd-per-call", "1"]),
        &[
            initialize(),
            call(
                1,
                "noul",
                &json!({ "state": "s", "instructions": "Is it?" }),
            ),
        ],
    )
    .await;
    let priced = run(
        serve(
            Some(&server),
            &["--max-cost-usd-per-call", "1", "--model", "jev-1.13.0"],
        ),
        &[
            initialize(),
            call(
                1,
                "noul",
                &json!({ "state": "s", "instructions": "Is it?" }),
            ),
        ],
    )
    .await;

    let (error, failed) = unpriced.tool(1);
    assert!(failed);
    assert_eq!(error["error"]["code"], "cost_limit");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("`jev-latest` has no known price"),
        "{error}"
    );
    let (result, failed) = priced.tool(1);
    assert!(!failed, "{result}");
    assert_eq!(result["session"]["max_cost_usd_per_call"], 1.0);
    assert_eq!(
        sent(&server).await.len(),
        1,
        "only the priced call was sent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn with_everything_logged_stdout_carries_only_protocol_messages_and_the_key_never_leaks() {
    let server = MockServer::start().await;
    // An error whose text quotes the key, as a careless server might.
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "detail": {
            "error_type": "authentication_error",
            "message": format!("Invalid API key: {SENTINEL_KEY}")
        }})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": { "answer": { "type": "noul", "noul": 0.7 } },
            "usage": { "input_tokens": 10, "output_tokens": 1 }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(500).set_body_string(SENTINEL_KEY))
        .mount(&server)
        .await;
    let mut command = serve(Some(&server), &["-vvv", "--debug-bodies"]);
    command.env("TYPESAFE_LOG_LEVEL", "trace");
    let noul = json!({ "state": "s", "instructions": "Is it?" });

    let run = run(
        command,
        &[
            initialize(),
            initialized(),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            call(2, "noul", &noul),
            call(3, "noul", &noul),
            call(4, "list_models", &json!({})),
            call(5, "validate", &noul),
            call(6, "evaluate", &json!({ "state": "s", "questions": {} })),
        ],
    )
    .await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.messages().len(),
        7,
        "every stdout line is a response: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("MCP request"),
        "the logging went to stderr: {}",
        run.stderr
    );
    assert_no_key(&run);
    let (error, failed) = run.tool(2);
    assert!(failed);
    assert_eq!(error["error"]["code"], "authentication");
    assert_eq!(error["error"]["exit_code"], 3);
    assert!(!run.tool(3).1);
    assert!(run.tool(4).1);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_key_the_server_starts_validates_and_says_what_is_missing() {
    let mut command = serve(None, &[]);
    command.env_remove("TYPESAFE_API_KEY");

    let run = run(
        command,
        &[
            initialize(),
            call(1, "validate", &triage()),
            call(2, "evaluate", &triage()),
            call(3, "evaluate", &json!({ "questions": {} })),
        ],
    )
    .await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(!run.tool(1).1);
    let (error, failed) = run.tool(2);
    assert!(failed);
    assert_eq!(error["error"]["exit_code"], 3);
    let (error, _) = run.tool(3);
    assert_eq!(
        error["error"]["exit_code"], 2,
        "an invalid request is reported before the missing key"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_negative_limit_is_a_usage_error() {
    let run = run(serve(None, &["--max-cost-usd-per-call=-1"]), &[]).await;

    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert!(
        run.stderr.contains("not an amount of US dollars"),
        "{}",
        run.stderr
    );
}
