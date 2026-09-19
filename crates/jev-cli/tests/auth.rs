//! `jev auth login|status|logout`, always with the keychain switched off and a scratch directory.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A key that must never be shown beyond its last four characters. It is not a real credential.
const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-0042-wxyz";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "auth-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(unique);
        let _ = fs::remove_dir_all(&dir);
        Self(dir)
    }

    fn credentials(&self) -> PathBuf {
        self.0.join("credentials")
    }

    fn jev(&self, server: Option<&MockServer>) -> Command {
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
        command.env("JEV_CONFIG_DIR", &self.0);
        // Never the real keychain of whoever runs the tests.
        command.env("JEV_NO_KEYCHAIN", "1");
        command.env(
            "TYPESAFE_BASE_URL",
            server.map_or_else(|| "http://127.0.0.1:9".to_owned(), MockServer::uri),
        );
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

async fn run(mut command: Command) -> Run {
    tokio::task::spawn_blocking(move || {
        let output = command.timeout(Duration::from_secs(60)).output().unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    })
    .await
    .unwrap()
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

/// A mock API that accepts `SENTINEL_KEY` and rejects every other key.
async fn api() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header(
            "authorization",
            format!("Bearer {SENTINEL_KEY}").as_str(),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "models": [{ "name": "jev-latest" }] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "detail": { "error_type": "authentication_error", "message": "Invalid API key." } })))
        .mount(&server)
        .await;
    server
}

/// Nothing `jev` prints may hold more of the key than its last four characters.
fn assert_key_is_masked(run: &Run) {
    for (stream, text) in [("stdout", &run.stdout), ("stderr", &run.stderr)] {
        assert!(
            !text.contains(SENTINEL_KEY),
            "the key leaked to {stream}: {text}"
        );
        let five = &SENTINEL_KEY[SENTINEL_KEY.len() - 5..];
        assert!(
            !text.contains(five),
            "more than four characters of the key reached {stream}: {text}"
        );
        assert!(
            !text.contains(&SENTINEL_KEY[..8]),
            "the start of the key reached {stream}: {text}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn login_status_and_logout_work_per_profile_without_a_terminal() {
    let server = api().await;
    let sandbox = Sandbox::new("lifecycle");
    let mut create = sandbox.jev(None);
    create.args(["profile", "create", "work"]);
    assert_eq!(run(create).await.code, 0);

    // Login: the key arrives on stdin, is verified, and lands in the credentials file.
    let mut login = sandbox.jev(Some(&server));
    login
        .args([
            "auth",
            "login",
            "--with-token",
            "--insecure-storage",
            "--profile",
            "work",
        ])
        .write_stdin(format!("{SENTINEL_KEY}\n"));
    let login = run(login).await;
    assert_eq!(
        (login.code, login.stderr.as_str()),
        (0, ""),
        "{}",
        login.stdout
    );
    assert_eq!(
        json_of(&login.stdout),
        json!({ "profile": "work", "stored_in": "file", "verified": true, "fingerprint": "…wxyz" })
    );
    assert_key_is_masked(&login);
    assert!(
        fs::read_to_string(sandbox.credentials())
            .unwrap()
            .contains("[profiles.work]")
    );

    // Status: per profile.
    let mut work = sandbox.jev(Some(&server));
    work.args(["auth", "status", "--profile", "work"]);
    let work = run(work).await;
    assert_eq!(work.code, 0, "{}", work.stderr);
    let status = json_of(&work.stdout);
    assert_eq!(
        (
            status["authenticated"].as_bool(),
            status["source"].as_str(),
            status["fingerprint"].as_str()
        ),
        (Some(true), Some("file"), Some("…wxyz"))
    );
    assert_eq!(status["check"], json!({ "ok": true, "detail": null }));
    assert_eq!(status["base_url"], server.uri());
    assert_key_is_masked(&work);

    let mut other = sandbox.jev(Some(&server));
    other.args(["auth", "status"]);
    let other = run(other).await;
    assert_eq!(other.code, 3, "the default profile has no key");
    assert_eq!(json_of(&other.stdout)["authenticated"], false);

    // The stored key is what `jev` now authenticates with.
    let mut models = sandbox.jev(Some(&server));
    models.args(["models", "list", "--profile", "work"]);
    assert_eq!(run(models).await.code, 0);

    // Logout.
    let mut logout = sandbox.jev(Some(&server));
    logout.args(["auth", "logout", "--profile", "work"]);
    let logout = run(logout).await;
    assert_eq!(
        json_of(&logout.stdout),
        json!({ "removed": [{ "profile": "work", "from": "file" }] })
    );
    let mut after = sandbox.jev(Some(&server));
    after.args(["models", "list", "--profile", "work"]);
    let after = run(after).await;
    assert_eq!(after.code, 3);
    let hint = json_of(&after.stderr)["error"]["hint"]
        .as_str()
        .unwrap()
        .to_owned();
    for remedy in [
        "TYPESAFE_API_KEY",
        "jev auth login",
        "https://console.typesafe.ai/keys",
    ] {
        assert!(hint.contains(remedy), "{hint}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_environment_variable_always_wins_over_a_stored_key() {
    let server = api().await;
    let sandbox = Sandbox::new("env-wins");
    let mut login = sandbox.jev(Some(&server));
    login
        .args(["auth", "login", "--with-token", "--insecure-storage"])
        .write_stdin(SENTINEL_KEY);
    assert_eq!(run(login).await.code, 0);

    let mut status = sandbox.jev(Some(&server));
    status
        .args(["auth", "status"])
        .env("TYPESAFE_API_KEY", "another-key-from-the-environment-abcd");
    let status = run(status).await;

    let report = json_of(&status.stdout);
    assert_eq!(
        (report["source"].as_str(), report["fingerprint"].as_str()),
        (Some("env"), Some("…abcd"))
    );
    assert_eq!(
        report["authenticated"], false,
        "the mock rejects that key, and it is the one in use"
    );
    assert_eq!(status.code, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn login_without_a_terminal_and_without_with_token_fails_fast() {
    let sandbox = Sandbox::new("no-tty");
    let mut command = sandbox.jev(None);
    command.args(["auth", "login"]);

    let run = run(command).await;

    assert_eq!((run.code, run.stdout.as_str()), (2, ""));
    let error = &json_of(&run.stderr)["error"];
    assert_eq!(error["code"], "input_required");
    assert_eq!(
        error["message"],
        "cannot ask for the API key: stdin is not a terminal"
    );
    assert!(
        error["hint"]
            .as_str()
            .unwrap()
            .contains("jev auth login --with-token"),
        "{error}"
    );
    assert!(!sandbox.credentials().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_key_is_not_stored() {
    let server = api().await;
    let sandbox = Sandbox::new("rejected");
    let mut command = sandbox.jev(Some(&server));
    command
        .args(["auth", "login", "--with-token", "--insecure-storage"])
        .write_stdin("a-wrong-key-that-the-api-rejects");

    let run = run(command).await;

    assert_eq!((run.code, run.stdout.as_str()), (3, ""));
    let error = &json_of(&run.stderr)["error"];
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("the API rejected the key, so nothing was stored"),
        "{error}"
    );
    assert!(
        error["hint"]
            .as_str()
            .unwrap()
            .contains("https://console.typesafe.ai/keys"),
        "{error}"
    );
    assert!(!run.stderr.contains("a-wrong-key"), "{}", run.stderr);
    assert!(!sandbox.credentials().exists(), "nothing may be stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn skip_verify_stores_without_a_network_and_an_unreachable_api_stores_nothing() {
    let sandbox = Sandbox::new("skip-verify");
    let mut unreachable = sandbox.jev(None);
    unreachable
        .args([
            "auth",
            "login",
            "--with-token",
            "--insecure-storage",
            "--max-retries",
            "0",
            "--timeout",
            "2",
        ])
        .write_stdin(SENTINEL_KEY);
    let mut skipped = sandbox.jev(None);
    skipped
        .args([
            "auth",
            "login",
            "--with-token",
            "--insecure-storage",
            "--skip-verify",
        ])
        .write_stdin(SENTINEL_KEY);

    let unreachable = run(unreachable).await;
    assert_eq!(unreachable.code, 6);
    assert!(
        json_of(&unreachable.stderr)["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("--skip-verify")
    );
    assert!(!sandbox.credentials().exists());

    let skipped = run(skipped).await;
    assert_eq!(skipped.code, 0, "{}", skipped.stderr);
    assert_eq!(json_of(&skipped.stdout)["verified"], false);
    assert!(sandbox.credentials().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_keychain_and_without_a_person_the_file_is_used_and_said_so() {
    let server = api().await;
    let sandbox = Sandbox::new("fallback");
    let mut command = sandbox.jev(Some(&server));
    command
        .args(["auth", "login", "--with-token"])
        .write_stdin(SENTINEL_KEY);

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(json_of(&run.stdout)["stored_in"], "file");
    let warning = &json_of(&run.stderr)["warning"];
    assert_eq!(warning["code"], "insecure_storage");
    assert!(
        warning["message"].as_str().unwrap().contains("clear text"),
        "{warning}"
    );
    assert_key_is_masked(&run);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_credentials_file_others_can_read_is_refused_with_the_exact_chmod() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("too-open");
    let mut login = sandbox.jev(None);
    login
        .args([
            "auth",
            "login",
            "--with-token",
            "--insecure-storage",
            "--skip-verify",
        ])
        .write_stdin(SENTINEL_KEY);
    assert_eq!(run(login).await.code, 0);
    assert_eq!(
        fs::metadata(sandbox.credentials())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "created private"
    );
    fs::set_permissions(sandbox.credentials(), fs::Permissions::from_mode(0o644)).unwrap();

    let mut command = sandbox.jev(None);
    command.args(["models", "list"]);
    let run = run(command).await;

    assert_eq!((run.code, run.stdout.as_str()), (3, ""));
    let error = &json_of(&run.stderr)["error"];
    assert_eq!(error["code"], "credentials_too_open");
    assert_eq!(
        error["hint"],
        format!("run `chmod 600 {}`", sandbox.credentials().display())
    );
    assert_key_is_masked(&run);
}

#[tokio::test(flavor = "multi_thread")]
async fn status_never_shows_more_than_the_last_four_characters_in_any_format() {
    let server = api().await;
    let sandbox = Sandbox::new("masking");
    let mut login = sandbox.jev(Some(&server));
    login
        .args(["auth", "login", "--with-token", "--insecure-storage"])
        .write_stdin(SENTINEL_KEY);
    assert_eq!(run(login).await.code, 0);

    for format in ["table", "json", "yaml", "jsonl"] {
        for verbosity in [None, Some("-vvv")] {
            let mut command = sandbox.jev(Some(&server));
            command
                .args(["auth", "status", "-o", format])
                .args(verbosity);

            let run = run(command).await;

            assert_eq!(run.code, 0, "{format}: {}", run.stderr);
            assert!(run.stdout.contains("…wxyz"), "{format}: {}", run.stdout);
            assert_key_is_masked(&run);
        }
    }
    let mut person = sandbox.jev(Some(&server));
    person.args(["auth", "status", "-o", "table", "--offline"]);
    let person = run(person).await;
    assert!(
        person.stdout.contains("key source     file"),
        "{}",
        person.stdout
    );
    assert!(
        person.stdout.contains("live check     skipped (--offline)"),
        "{}",
        person.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_profile_forgets_its_key_and_logout_all_clears_everything() {
    let sandbox = Sandbox::new("forget");
    for profile in ["one", "two"] {
        let mut create = sandbox.jev(None);
        create.args(["profile", "create", profile]);
        assert_eq!(run(create).await.code, 0);
        let mut login = sandbox.jev(None);
        login
            .args([
                "auth",
                "login",
                "--with-token",
                "--insecure-storage",
                "--skip-verify",
                "--profile",
                profile,
            ])
            .write_stdin(SENTINEL_KEY);
        assert_eq!(run(login).await.code, 0);
    }

    let mut delete = sandbox.jev(None);
    delete.args(["profile", "delete", "one"]);
    assert_eq!(run(delete).await.code, 0);
    assert!(
        !fs::read_to_string(sandbox.credentials())
            .unwrap()
            .contains("profiles.one")
    );

    let mut logout = sandbox.jev(None);
    logout.args(["auth", "logout", "--all"]);
    let logout = run(logout).await;
    assert_eq!(
        json_of(&logout.stdout),
        json!({ "removed": [{ "profile": "two", "from": "file" }] })
    );
    assert!(
        !fs::read_to_string(sandbox.credentials())
            .unwrap()
            .contains(SENTINEL_KEY)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_key_is_never_a_flag_value() {
    let sandbox = Sandbox::new("no-flag");
    for arguments in [
        vec!["auth", "login", "--token", SENTINEL_KEY],
        vec!["auth", "login", SENTINEL_KEY],
        vec!["auth", "login", "--api-key", SENTINEL_KEY],
    ] {
        let mut command = sandbox.jev(None);
        command.args(&arguments);

        let run = run(command).await;

        assert_eq!(run.code, 2, "{arguments:?}");
    }
    let mut help = sandbox.jev(None);
    help.args(["auth", "login", "--help"]);
    assert!(
        run(help)
            .await
            .stdout
            .contains("never accepted as a flag value")
    );
}
