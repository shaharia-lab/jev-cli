//! `jev update` and the install facts `jev version` reports, run on copies of the built `jev`.
//!
//! Every test copies `jev` into a scratch directory and runs the copy, so the binary replaced is
//! the one running: on Windows that is the rename-aside case. Releases come from a local server
//! that stands in for GitHub, signed with a key made for the test; the test hooks point `jev` at
//! both, so nothing here can reach GitHub. Without the hooks only the paths that never touch the
//! network are tested.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::Value;

const EXE: &str = std::env::consts::EXE_SUFFIX;

/// A scratch directory, removed afterwards.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "update-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Copies the built `jev` to `relative` inside the sandbox, and returns where it is.
    fn install(&self, relative: &str) -> PathBuf {
        let exe = self.0.join(format!("{relative}{EXE}"));
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(assert_cmd::cargo::cargo_bin("jev"), &exe).unwrap();
        exe
    }

    fn touch(&self, relative: &str) {
        self.write(relative, "");
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    /// A copy of `jev` as the install script installs it, with its receipt: the only kind of
    /// install that updates itself.
    fn install_with_script(&self) -> PathBuf {
        self.touch("local/bin/.jev-update/receipt.json");
        self.install("local/bin/jev")
    }

    /// The configuration directory of this sandbox's `jev`.
    fn config(&self) -> PathBuf {
        self.0.join("config")
    }
}

/// The copy of `jev` at `exe` with the sandbox's own configuration directory.
fn jev_in(sandbox: &Sandbox, exe: &Path) -> Command {
    let mut command = jev(exe);
    command.env("JEV_CONFIG_DIR", sandbox.config());
    command
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The copy of `jev` at `exe`, isolated from the environment of whoever runs the tests.
fn jev(exe: &Path) -> Command {
    let mut command = Command::new(exe);
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
        "JEV_AUTO_UPDATE",
        "JEV_TEST_VERSION",
        "JEV_TEST_UPDATE_URL",
        "JEV_TEST_UPDATE_KEY",
        "TYPESAFE_API_KEY",
        "TYPESAFE_DEFAULT_MODEL",
        "TYPESAFE_LOG_LEVEL",
    ] {
        command.env_remove(variable);
    }
    command.env(
        "JEV_CONFIG_DIR",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
    );
    command.env("TYPESAFE_BASE_URL", "http://127.0.0.1:9");
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(command: &mut Command) -> Run {
    command.timeout(Duration::from_secs(120));
    // A copy of `jev` made by one test can be briefly held open for writing by a process another
    // test is starting at that moment, which inherits the descriptor until it execs; Linux then
    // refuses to run the copy ("Text file busy"). That clears within milliseconds.
    let mut attempts = 0;
    let output = loop {
        match command.output() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 50 =>
            {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            outcome => break outcome.unwrap(),
        }
    };
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

#[test]
fn a_homebrew_install_is_told_to_use_brew_and_nothing_changes() {
    let sandbox = Sandbox::new("brew");
    let exe = sandbox.install("homebrew/Cellar/jev/0.1.0/bin/jev");
    let before = fs::read(&exe).unwrap();

    let update = run(jev(&exe).args(["update", "-o", "json"]));
    let pinned = run(jev(&exe).args(["update", "--version", "0.3.1", "-o", "json"]));
    let rollback = run(jev(&exe).args(["update", "--rollback", "-o", "json"]));

    assert_eq!((update.code, update.stderr.as_str()), (0, ""));
    let report = json(&update.stdout);
    assert_eq!(report["status"], "managed");
    assert_eq!(report["install_method"], "homebrew");
    assert_eq!(report["command"], "brew upgrade shaharia-lab/tap/jev");
    assert_eq!(
        json(&pinned.stdout)["command"],
        "brew install shaharia-lab/tap/jev@0.3.1"
    );
    assert_eq!((rollback.code, rollback.stdout.as_str()), (2, ""));
    assert!(
        json(&rollback.stderr)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("installed with Homebrew")
    );

    assert_eq!(fs::read(&exe).unwrap(), before);
    assert!(!exe.parent().unwrap().join(".jev-update").exists());
}

#[test]
fn a_cargo_install_is_told_to_use_cargo_and_nothing_changes() {
    let sandbox = Sandbox::new("cargo");
    sandbox.touch("cargo-home/.crates.toml");
    let exe = sandbox.install("cargo-home/bin/jev");

    let update = run(jev(&exe).args(["update", "-o", "table"]));

    assert_eq!(update.code, 0, "{}", update.stderr);
    assert_eq!(
        update.stdout,
        "jev was installed with cargo, which updates it\n  run: cargo install jev-cli --locked\n"
    );
    assert!(!exe.parent().unwrap().join(".jev-update").exists());
}

#[test]
fn jev_version_reports_the_install_method_and_why_auto_update_is_off() {
    let sandbox = Sandbox::new("version");
    let brew = sandbox.install("homebrew/Cellar/jev/0.1.0/bin/jev");
    let script = sandbox.install("local/bin/jev");
    sandbox.touch("local/bin/.jev-update/receipt.json");

    let brew = json(&run(jev(&brew).args(["version", "-o", "json"])).stdout);
    let script = json(
        &run(jev(&script)
            .args(["version", "-o", "json"])
            .env("CI", "true"))
        .stdout,
    );

    assert_eq!(brew["install_method"], "homebrew");
    assert_eq!(brew["update_channel"], "stable");
    assert_eq!(brew["auto_update"]["enabled"], false);
    assert!(
        brew["auto_update"]["reason"]
            .as_str()
            .unwrap()
            .contains("brew upgrade"),
        "{brew}"
    );
    assert_eq!(script["install_method"], "self_managed");
    assert_eq!(script["auto_update"]["reason"], "CI=true");
}

#[test]
fn jev_version_names_the_guard_that_turns_automatic_updates_off() {
    let sandbox = Sandbox::new("guards");
    let exe = sandbox.install_with_script();
    let unknown = sandbox.install("elsewhere/jev");
    let auto_update = |command: &mut Command| {
        let report = json(&run(command.args(["version", "-o", "json"])).stdout);
        (
            report["auto_update"]["enabled"].as_bool().unwrap(),
            report["auto_update"]["reason"].as_str().map(str::to_owned),
        )
    };

    assert_eq!(auto_update(&mut jev_in(&sandbox, &exe)), (true, None));
    assert_eq!(
        auto_update(jev_in(&sandbox, &exe).env("JEV_AUTO_UPDATE", "0")),
        (false, Some("turned off by JEV_AUTO_UPDATE".to_owned()))
    );
    assert_eq!(
        auto_update(jev_in(&sandbox, &exe).env("CI", "true")),
        (false, Some("CI=true".to_owned()))
    );
    let (enabled, reason) = auto_update(&mut jev_in(&sandbox, &unknown));
    assert!(!enabled);
    assert!(
        reason
            .unwrap()
            .starts_with("not installed by the install script"),
        "no receipt"
    );

    for (config, expected) in [
        (
            "[update]\nauto = false\n",
            "turned off by `update.auto = false` in config.toml",
        ),
        (
            "[update]\npin_version = \"0.3.1\"\n",
            "update.pin_version is set to 0.3.1",
        ),
    ] {
        sandbox.write("config/config.toml", config);
        assert_eq!(
            auto_update(&mut jev_in(&sandbox, &exe)),
            (false, Some(expected.to_owned())),
            "{config}"
        );
    }
    sandbox.write("config/config.toml", "[update]\nchannel = \"prerelease\"\n");
    let report = json(&run(jev_in(&sandbox, &exe).args(["version", "-o", "json"])).stdout);
    assert_eq!(report["update_channel"], "prerelease");
    assert_eq!(report["auto_update"]["enabled"], true);
}

#[cfg(unix)]
#[test]
fn a_binary_in_a_directory_jev_cannot_write_is_not_updated_automatically() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("read-only");
    let exe = sandbox.install_with_script();
    let dir = exe.parent().unwrap().to_owned();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let report = run(jev_in(&sandbox, &exe).args(["version", "-o", "json"]));
    let listed = run(jev_in(&sandbox, &exe).args(["config", "path", "-o", "json"]));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

    // Root can write anywhere; the guard only means something for everyone else.
    let report = json(&report.stdout);
    if report["auto_update"]["enabled"] == false {
        assert!(
            report["auto_update"]["reason"]
                .as_str()
                .unwrap()
                .contains("never asks for elevated rights"),
            "{report}"
        );
        assert_eq!(
            (listed.code, listed.stderr.as_str()),
            (0, ""),
            "no notice either"
        );
        assert!(!sandbox.config().join("auto-update.json").exists());
    }
}

#[test]
fn rollback_without_a_previous_version_is_a_usage_error() {
    let sandbox = Sandbox::new("no-previous");
    let exe = sandbox.install("bin/jev");

    let rollback = run(jev(&exe).args(["update", "--rollback", "-o", "json"]));

    assert_eq!((rollback.code, rollback.stdout.as_str()), (2, ""));
    assert_eq!(
        json(&rollback.stderr)["error"]["code"],
        "no_previous_version"
    );
}

#[test]
fn a_version_below_the_minimum_is_refused_before_anything_is_fetched() {
    let sandbox = Sandbox::new("minimum");
    let exe = sandbox.install("bin/jev");

    let old = run(jev(&exe).args(["update", "--version", "0.0.9", "-o", "json"]));

    assert_eq!((old.code, old.stdout.as_str()), (2, ""));
    assert!(
        json(&old.stderr)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("older than 0.1.0")
    );
}

/// Releases served by a local stand-in for GitHub, which the test hooks point `jev` at.
#[cfg(feature = "internal-test-hooks")]
mod with_releases {
    use std::fmt::Write as _;
    use std::fs;
    use std::io::{Cursor, Write};
    use std::path::Path;

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use std::time::{Duration, Instant};

    use super::{EXE, Run, Sandbox, jev, jev_in, json, run};

    const REPOSITORY: &str = "shaharia-lab/jev-cli";

    struct Signer(minisign::KeyPair);

    impl Signer {
        fn new() -> Self {
            Self(minisign::KeyPair::generate_unencrypted_keypair().unwrap())
        }

        fn public(&self) -> String {
            self.0.pk.to_base64()
        }

        fn sign(&self, data: &[u8], file: &str, version: &str) -> Vec<u8> {
            minisign::sign(
                None,
                &self.0.sk,
                Cursor::new(data),
                Some(&format!("file:{file}\tversion:{version}")),
                None,
            )
            .unwrap()
            .into_string()
            .into_bytes()
        }
    }

    /// The target whose archive `jev` asks for.
    fn target() -> String {
        let reported =
            run(jev(&assert_cmd::cargo::cargo_bin("jev")).args(["version", "--field", "target"]));
        reported
            .stdout
            .trim()
            .replace("-unknown-linux-gnu", "-unknown-linux-musl")
    }

    /// The release archive of `version` holding `binary`, packed as package.sh packs it.
    fn archive(version: &str, binary: &[u8]) -> (String, Vec<u8>) {
        let target = target();
        let name = format!("jev-{version}-{target}");
        if target.contains("-windows-") {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer
                .start_file(
                    format!("{name}/jev.exe"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(binary).unwrap();
            (format!("{name}.zip"), writer.finish().unwrap().into_inner())
        } else {
            let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::fast(),
            ));
            let mut header = tar::Header::new_gnu();
            header.set_size(binary.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{name}/jev"), binary)
                .unwrap();
            (
                format!("{name}.tar.gz"),
                builder.into_inner().unwrap().finish().unwrap(),
            )
        }
    }

    /// Publishes `version` on `server`, with `binary` inside, signed by `signer`.
    async fn publish(
        server: &MockServer,
        version: &str,
        binary: &[u8],
        signer: &Signer,
        latest: bool,
    ) {
        let (name, bytes) = archive(version, binary);
        let mut sums = String::new();
        for byte in ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref() {
            write!(sums, "{byte:02x}").unwrap();
        }
        writeln!(sums, "  {name}").unwrap();
        let files = [
            (name.clone(), bytes.clone()),
            (
                format!("{name}.minisig"),
                signer.sign(&bytes, &name, version),
            ),
            ("SHA256SUMS".to_owned(), sums.clone().into_bytes()),
            (
                "SHA256SUMS.minisig".to_owned(),
                signer.sign(sums.as_bytes(), "SHA256SUMS", version),
            ),
        ];
        let release = json!({
            "tag_name": format!("v{version}"),
            "html_url": format!("https://github.com/{REPOSITORY}/releases/tag/v{version}"),
            "draft": false,
            "prerelease": false,
            "assets": files.iter().map(|(name, _)| json!({ "name": name })).collect::<Vec<_>>(),
        });
        let mut paths = vec![format!("/repos/{REPOSITORY}/releases/tags/v{version}")];
        if latest {
            paths.push(format!("/repos/{REPOSITORY}/releases/latest"));
        }
        for path_ in paths {
            Mock::given(method("GET"))
                .and(path(path_))
                .respond_with(ResponseTemplate::new(200).set_body_json(&release))
                .mount(server)
                .await;
        }
        for (file, bytes) in files {
            Mock::given(method("GET"))
                .and(path(format!(
                    "/{REPOSITORY}/releases/download/v{version}/{file}"
                )))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
                .mount(server)
                .await;
        }
    }

    /// Runs the copy of `jev` at `exe` as version `current`, against releases on `server` signed
    /// by `key`.
    async fn update(
        exe: &Path,
        current: &str,
        server: &MockServer,
        key: &str,
        args: &[&str],
    ) -> Run {
        let mut command = jev(exe);
        command
            .args(args)
            .env("JEV_TEST_VERSION", current)
            .env("JEV_TEST_UPDATE_URL", server.uri())
            .env("JEV_TEST_UPDATE_KEY", key);
        tokio::task::spawn_blocking(move || run(&mut command))
            .await
            .unwrap()
    }

    fn jev_bytes() -> Vec<u8> {
        fs::read(assert_cmd::cargo::cargo_bin("jev")).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn check_exits_20_when_a_newer_version_exists_and_0_when_not() {
        let sandbox = Sandbox::new("check");
        let exe = sandbox.install("bin/jev");
        let server = MockServer::start().await;
        let signer = Signer::new();
        publish(&server, "0.2.0", b"unused", &signer, true).await;

        let older = update(
            &exe,
            "0.1.0",
            &server,
            &signer.public(),
            &["update", "--check", "-o", "json"],
        )
        .await;
        let same = update(
            &exe,
            "0.2.0",
            &server,
            &signer.public(),
            &["update", "--check", "-o", "json"],
        )
        .await;

        assert_eq!((older.code, older.stderr.as_str()), (20, ""));
        let report = json(&older.stdout);
        assert_eq!(report["status"], "update_available");
        assert_eq!(report["current_version"], "0.1.0");
        assert_eq!(report["version"], "0.2.0");
        assert_eq!(report["command"], "jev update");
        assert_eq!(
            report["release_url"],
            format!("https://github.com/{REPOSITORY}/releases/tag/v0.2.0")
        );
        assert_eq!((same.code, same.stderr.as_str()), (0, ""));
        assert_eq!(json(&same.stdout)["status"], "up_to_date");
        assert!(
            !exe.parent().unwrap().join(".jev-update").exists(),
            "a check writes nothing"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_verified_release_replaces_the_running_binary_and_rollback_restores_it() {
        let sandbox = Sandbox::new("apply");
        let exe = sandbox.install("bin/jev");
        let server = MockServer::start().await;
        let signer = Signer::new();
        publish(&server, "0.2.0", &jev_bytes(), &signer, true).await;

        let updated = update(
            &exe,
            "0.1.0",
            &server,
            &signer.public(),
            &["update", "-o", "json"],
        )
        .await;

        assert_eq!(
            (updated.code, updated.stderr.as_str()),
            (0, ""),
            "{}",
            updated.stdout
        );
        let report = json(&updated.stdout);
        assert_eq!(report["status"], "updated");
        assert_eq!(report["current_version"], "0.1.0");
        assert_eq!(report["version"], "0.2.0");
        let state = exe.parent().unwrap().join(".jev-update");
        assert!(state.join(format!("previous{EXE}")).is_file());
        assert!(!state.join(format!("ready{EXE}")).exists());

        let rolled_back = update(
            &exe,
            "0.2.0",
            &server,
            &signer.public(),
            &["update", "--rollback", "-o", "json"],
        )
        .await;

        assert_eq!(
            (rolled_back.code, rolled_back.stderr.as_str()),
            (0, ""),
            "{}",
            rolled_back.stdout
        );
        let report = json(&rolled_back.stdout);
        assert_eq!(report["status"], "rolled_back");
        assert_eq!(report["current_version"], "0.2.0");
        assert!(
            state.join(format!("previous{EXE}")).is_file(),
            "and can be undone"
        );
        assert_eq!(
            run(jev(&exe).arg("version")).code,
            0,
            "the binary still runs"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_signed_by_an_unknown_key_changes_nothing() {
        let sandbox = Sandbox::new("bad-signature");
        let exe = sandbox.install("bin/jev");
        let before = fs::read(&exe).unwrap();
        let server = MockServer::start().await;
        publish(&server, "0.2.0", b"malicious", &Signer::new(), true).await;

        let trusted = Signer::new().public();
        let refused = update(&exe, "0.1.0", &server, &trusted, &["update", "-o", "json"]).await;

        assert_eq!((refused.code, refused.stdout.as_str()), (1, ""));
        let error = &json(&refused.stderr)["error"];
        assert_eq!(error["code"], "update_verification_failed");
        assert!(
            error["hint"]
                .as_str()
                .unwrap()
                .contains("nothing was changed")
        );
        assert_eq!(fs::read(&exe).unwrap(), before);
        let state = exe.parent().unwrap().join(".jev-update");
        assert!(!state.join(format!("ready{EXE}")).exists());
        assert!(!state.join(format!("previous{EXE}")).exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_binary_that_fails_its_self_test_is_rolled_back() {
        let sandbox = Sandbox::new("self-test");
        let exe = sandbox.install("bin/jev");
        let before = fs::read(&exe).unwrap();
        let server = MockServer::start().await;
        let signer = Signer::new();
        publish(&server, "0.2.0", b"not a program", &signer, true).await;

        let failed = update(
            &exe,
            "0.1.0",
            &server,
            &signer.public(),
            &["update", "-o", "json"],
        )
        .await;

        assert_eq!((failed.code, failed.stdout.as_str()), (1, ""));
        let error = &json(&failed.stderr)["error"];
        assert_eq!(error["code"], "update_self_test_failed");
        assert!(
            error["hint"]
                .as_str()
                .unwrap()
                .contains("previous binary was put back")
        );
        assert_eq!(fs::read(&exe).unwrap(), before);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_newer_jev_is_never_downgraded_unless_a_version_is_named() {
        let sandbox = Sandbox::new("downgrade");
        let exe = sandbox.install("bin/jev");
        let before = fs::read(&exe).unwrap();
        let server = MockServer::start().await;
        let signer = Signer::new();
        publish(&server, "0.2.0", &jev_bytes(), &signer, true).await;

        let automatic = update(
            &exe,
            "0.3.0",
            &server,
            &signer.public(),
            &["update", "-o", "json"],
        )
        .await;

        assert_eq!(automatic.code, 0);
        assert_eq!(json(&automatic.stdout)["status"], "up_to_date");
        let warning = &json(&automatic.stderr)["warning"];
        assert_eq!(warning["code"], "newer_than_latest");
        assert!(
            warning["hint"]
                .as_str()
                .unwrap()
                .contains("jev update --version 0.2.0")
        );
        assert_eq!(fs::read(&exe).unwrap(), before);

        let named = update(
            &exe,
            "0.3.0",
            &server,
            &signer.public(),
            &["update", "--version", "0.2.0", "-o", "json"],
        )
        .await;

        assert_eq!(
            (named.code, named.stderr.as_str()),
            (0, ""),
            "{}",
            named.stdout
        );
        assert_eq!(json(&named.stdout)["status"], "updated");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_release_no_such_version_and_no_github_are_each_reported() {
        let sandbox = Sandbox::new("unreachable");
        let exe = sandbox.install("bin/jev");
        let signer = Signer::new();
        // Nothing mounted: GitHub's 404 when nothing has been published.
        let empty = MockServer::start().await;

        let nothing = update(
            &exe,
            "0.1.0",
            &empty,
            &signer.public(),
            &["update", "-o", "json"],
        )
        .await;
        let missing = update(
            &exe,
            "0.1.0",
            &empty,
            &signer.public(),
            &["update", "--version", "0.9.0", "-o", "json"],
        )
        .await;
        let mut offline = jev(&exe);
        offline
            .args(["update", "--check", "-o", "json"])
            .env("JEV_TEST_UPDATE_URL", "http://127.0.0.1:9")
            .env("JEV_TEST_UPDATE_KEY", signer.public());
        let offline = tokio::task::spawn_blocking(move || run(&mut offline))
            .await
            .unwrap();

        assert_eq!((nothing.code, nothing.stderr.as_str()), (0, ""));
        assert_eq!(json(&nothing.stdout)["status"], "up_to_date");
        assert_eq!(json(&nothing.stdout)["version"], serde_json::Value::Null);
        assert_eq!(missing.code, 2);
        assert_eq!(
            json(&missing.stderr)["error"]["message"],
            "there is no published release of jev 0.9.0"
        );
        assert_eq!((offline.code, offline.stdout.as_str()), (6, ""));
        assert_eq!(json(&offline.stderr)["error"]["code"], "connection");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_that_fails_changes_nothing() {
        let sandbox = Sandbox::new("download-fails");
        let exe = sandbox.install("bin/jev");
        let before = fs::read(&exe).unwrap();
        let server = MockServer::start().await;
        let signer = Signer::new();
        // Mounted first, so it wins over the working download.
        let (archive, _) = archive("0.2.0", b"");
        Mock::given(method("GET"))
            .and(path(format!(
                "/{REPOSITORY}/releases/download/v0.2.0/{archive}"
            )))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;
        publish(&server, "0.2.0", b"unused", &signer, true).await;

        let failed = update(
            &exe,
            "0.1.0",
            &server,
            &signer.public(),
            &["update", "-o", "json"],
        )
        .await;

        assert_eq!((failed.code, failed.stdout.as_str()), (6, ""));
        let error = &json(&failed.stderr)["error"];
        assert_eq!(error["code"], "server_error");
        assert_eq!(error["http_status"], 502);
        assert_eq!(fs::read(&exe).unwrap(), before);
        assert!(
            !exe.parent()
                .unwrap()
                .join(".jev-update")
                .join(format!("ready{EXE}"))
                .exists()
        );
    }

    /// A command that needs no network, from the copy of `jev` at `exe` in `sandbox`, as version
    /// `current`, its automatic updates pointed at `server` and trusting `key`.
    fn command(
        sandbox: &Sandbox,
        exe: &Path,
        current: &str,
        server: &MockServer,
        key: &str,
    ) -> assert_cmd::Command {
        let mut command = jev_in(sandbox, exe);
        command
            .args(["config", "path", "-o", "json"])
            .env("JEV_TEST_VERSION", current)
            .env("JEV_TEST_UPDATE_URL", server.uri())
            .env("JEV_TEST_UPDATE_KEY", key)
            .env("TYPESAFE_API_KEY", SENTINEL_KEY);
        command
    }

    /// A key that must never appear in the updater's output or files. It is not a real credential.
    const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-6d02";

    /// Runs [`command`], changed by `configure`.
    async fn automatic(
        sandbox: &Sandbox,
        exe: &Path,
        current: &str,
        server: &MockServer,
        key: &str,
        configure: impl FnOnce(&mut assert_cmd::Command),
    ) -> Run {
        let mut command = command(sandbox, exe, current, server, key);
        configure(&mut command);
        tokio::task::spawn_blocking(move || run(&mut command))
            .await
            .unwrap()
    }

    /// How many times `jev` asked `server` for the latest release.
    async fn checks(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path().ends_with("/releases/latest"))
            .count()
    }

    /// Waits for the background check to leave `what` behind in the sandbox.
    async fn eventually(what: &str, done: impl Fn() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(
                started.elapsed() < Duration::from_secs(90),
                "the background check never {what}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn state(sandbox: &Sandbox) -> Option<serde_json::Value> {
        let text = fs::read_to_string(sandbox.config().join("auto-update.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Whether the last check has recorded what it found.
    fn recorded(sandbox: &Sandbox) -> bool {
        state(sandbox).is_some_and(|state| {
            state
                .get("last_outcome")
                .is_some_and(serde_json::Value::is_string)
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_newer_release_is_staged_after_one_command_and_swapped_in_before_the_next() {
        let sandbox = Sandbox::new("automatic");
        let exe = sandbox.install_with_script();
        let before = fs::read(&exe).unwrap();
        let server = MockServer::start().await;
        let signer = Signer::new();
        let key = signer.public();
        publish(&server, "0.2.0", &jev_bytes(), &signer, true).await;

        let opted_out = automatic(&sandbox, &exe, "0.1.0", &server, &key, |command| {
            command.env("JEV_AUTO_UPDATE", "false");
        })
        .await;
        let first = automatic(&sandbox, &exe, "0.1.0", &server, &key, |_| {}).await;

        // The command's own result is the same with or without automatic updates.
        assert_eq!((opted_out.code, opted_out.stderr.as_str()), (0, ""));
        assert_eq!((first.code, &first.stdout), (0, &opted_out.stdout));
        let notice = &json(&first.stderr)["info"];
        assert_eq!(notice["code"], "auto_update_enabled");
        assert!(
            notice["hint"]
                .as_str()
                .unwrap()
                .contains("JEV_AUTO_UPDATE=false"),
            "{notice}"
        );
        let staged = exe.parent().unwrap().join(".jev-update");
        eventually("staged the release", || recorded(&sandbox)).await;
        assert_eq!(state(&sandbox).unwrap()["last_outcome"], "staged jev 0.2.0");
        assert!(staged.join(format!("ready{EXE}")).is_file());
        assert_eq!(fs::read(&exe).unwrap(), before, "not swapped yet");

        let second = automatic(&sandbox, &exe, "0.1.0", &server, &key, |_| {}).await;

        assert_eq!((second.code, &second.stdout), (0, &opted_out.stdout));
        let lines: Vec<&str> = second.stderr.lines().collect();
        assert_eq!(lines.len(), 1, "exactly one line: {lines:?}");
        let notice = &json(lines[0])["info"];
        assert_eq!(notice["code"], "updated");
        let message = notice["message"].as_str().unwrap();
        assert!(message.starts_with("jev updated 0.1.0 "), "{message}");
        assert!(
            message.ends_with(&format!(
                "0.2.0 {} changelog: https://github.com/{REPOSITORY}/releases/tag/v0.2.0",
                if cfg!(windows) { "\u{2014}" } else { "-" }
            )),
            "{message}"
        );
        assert!(staged.join(format!("previous{EXE}")).is_file());
        assert!(!staged.join(format!("ready{EXE}")).exists());

        // The swap started no second check: that waits for the next day.
        let third = automatic(&sandbox, &exe, "0.2.0", &server, &key, |_| {}).await;
        assert_eq!((third.code, third.stderr.as_str()), (0, ""));
        assert_eq!(checks(&server).await, 1);

        let recorded = fs::read_to_string(sandbox.config().join("auto-update.json")).unwrap();
        for text in [&first.stderr, &second.stderr, &recorded] {
            assert!(!text.contains(SENTINEL_KEY), "{text}");
        }
        for request in server.received_requests().await.unwrap() {
            assert!(
                !request.headers.contains_key("authorization"),
                "GitHub is never sent a key"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_commands_start_one_check_a_day() {
        let sandbox = Sandbox::new("once-a-day");
        let exe = sandbox.install_with_script();
        let server = MockServer::start().await;
        let key = Signer::new().public();

        let started: Vec<_> = (0..6)
            .map(|_| {
                let mut command = command(&sandbox, &exe, "0.1.0", &server, &key);
                tokio::task::spawn_blocking(move || run(&mut command))
            })
            .collect();
        let mut runs = Vec::new();
        for run in started {
            runs.push(run.await.unwrap());
        }

        assert!(runs.iter().all(|run| run.code == 0));
        let notices = runs.iter().filter(|run| !run.stderr.is_empty()).count();
        assert_eq!(notices, 1, "the first-run notice is shown once");
        eventually("recorded its outcome", || recorded(&sandbox)).await;
        assert_eq!(
            state(&sandbox).unwrap()["last_outcome"],
            "no release is published yet"
        );
        automatic(&sandbox, &exe, "0.1.0", &server, &key, |_| {}).await;
        assert_eq!(checks(&server).await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_guard_means_no_check_and_no_request() {
        let sandbox = Sandbox::new("no-request");
        let exe = sandbox.install_with_script();
        let server = MockServer::start().await;
        let key = Signer::new().public();

        for (variable, value) in [("JEV_AUTO_UPDATE", "off"), ("CI", "true")] {
            let run = automatic(&sandbox, &exe, "0.1.0", &server, &key, |command| {
                command.env(variable, value);
            })
            .await;
            assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{variable}");
        }
        sandbox.write("config/config.toml", "[update]\npin_version = \"0.1.0\"\n");
        let pinned = automatic(&sandbox, &exe, "0.1.0", &server, &key, |_| {}).await;
        assert_eq!((pinned.code, pinned.stderr.as_str()), (0, ""));

        // A check is claimed in the state file before it is started, so without the file none was.
        assert!(state(&sandbox).is_none());
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_check_never_holds_up_the_command() {
        let sandbox = Sandbox::new("slow");
        let exe = sandbox.install_with_script();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/repos/{REPOSITORY}/releases/latest")))
            .respond_with(ResponseTemplate::new(404).set_delay(Duration::from_secs(15)))
            .mount(&server)
            .await;
        let key = Signer::new().public();

        let started = Instant::now();
        let run = automatic(&sandbox, &exe, "0.1.0", &server, &key, |_| {}).await;
        let took = started.elapsed();

        assert_eq!(run.code, 0);
        assert!(took < Duration::from_secs(8), "took {took:?}");
        let started = Instant::now();
        while checks(&server).await == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(90),
                "no check started"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_staged_update_that_fails_its_self_test_is_rolled_back_silently() {
        let sandbox = Sandbox::new("automatic-self-test");
        let exe = sandbox.install_with_script();
        let before = fs::read(&exe).unwrap();
        let server = MockServer::start().await;
        let signer = Signer::new();
        let key = signer.public();
        publish(&server, "0.2.0", b"not a program", &signer, true).await;

        automatic(&sandbox, &exe, "0.1.0", &server, &key, |_| {}).await;
        eventually("staged the release", || recorded(&sandbox)).await;
        let next = automatic(&sandbox, &exe, "0.1.0", &server, &key, |command| {
            command.arg("-v");
        })
        .await;

        assert_eq!(next.code, 0);
        assert!(!next.stderr.contains("\"updated\""), "{}", next.stderr);
        assert!(
            next.stderr.contains("update_self_test_failed"),
            "logged at -v: {}",
            next.stderr
        );
        assert_eq!(fs::read(&exe).unwrap(), before);
        let quiet = automatic(&sandbox, &exe, "0.1.0", &server, &key, |_| {}).await;
        assert_eq!(
            (quiet.code, quiet.stderr.as_str()),
            (0, ""),
            "not tried again"
        );
    }
}
