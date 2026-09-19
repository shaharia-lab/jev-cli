//! The install scripts at the root of the repository, `install.sh` (Linux, macOS) and
//! `install.ps1` (Windows), run end to end on the platform they are for.
//!
//! Releases come from a local server that stands in for GitHub, signed with a key made for the
//! test and holding the built `jev`. Each test runs a copy of the script whose block of release
//! constants (where releases come from, the trusted keys, the verifier) points at that server and
//! key; the rest of the script is what ships, and tests below check that the shipped constants
//! are GitHub over HTTPS and the committed release keys. Nothing here can reach GitHub.
//!
//! The signature checks need the minisign CLI, as the scripts do. CI installs it on every OS
//! (scripts/ci/install-minisign.sh); elsewhere those tests are skipped when it is missing.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

/// The repository root, where the scripts are.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn script(name: &str) -> String {
    fs::read_to_string(root().join(name)).unwrap()
}

/// The value assigned on the one line of `text` that starts with `prefix`.
fn constant<'a>(text: &'a str, prefix: &str) -> &'a str {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.trim_start().starts_with(prefix))
        .collect();
    assert_eq!(lines.len(), 1, "one line starts with {prefix:?}");
    lines
        .first()
        .unwrap()
        .trim_start()
        .strip_prefix(prefix)
        .unwrap()
        .trim()
}

#[test]
fn both_scripts_trust_exactly_the_committed_release_keys() {
    let keys: Vec<String> = ["release-primary.pub", "release-next.pub"]
        .iter()
        .map(|file| {
            let text = fs::read_to_string(root().join("crates/jev-cli/keys").join(file)).unwrap();
            text.lines().nth(1).unwrap().trim().to_owned()
        })
        .collect();

    let (shell, powershell) = (script("install.sh"), script("install.ps1"));
    let shell = constant(&shell, "PUBLIC_KEYS=");
    let powershell = constant(&powershell, "$PublicKeys = ");

    assert_eq!(shell, format!("'{} {}'", keys[0], keys[1]));
    assert_eq!(powershell, format!("@('{}', '{}')", keys[0], keys[1]));
}

#[test]
fn both_scripts_download_from_github_over_https_and_use_the_real_verifier() {
    let shell = script("install.sh");
    let powershell = script("install.ps1");

    for (text, prefix, expected) in [
        (
            &shell,
            "REPOSITORY_URL=",
            "'https://github.com/shaharia-lab/jev-cli'",
        ),
        (
            &shell,
            "API_URL=",
            "'https://api.github.com/repos/shaharia-lab/jev-cli'",
        ),
        (&shell, "ALLOW_HTTP=", "0"),
        (&shell, "MINISIGN=", "minisign"),
        (
            &powershell,
            "$RepositoryUrl = ",
            "'https://github.com/shaharia-lab/jev-cli'",
        ),
        (
            &powershell,
            "$ApiUrl = ",
            "'https://api.github.com/repos/shaharia-lab/jev-cli'",
        ),
        (&powershell, "$AllowHttp = ", "$false"),
        (&powershell, "$Minisign = ", "'minisign'"),
    ] {
        assert_eq!(constant(text, prefix), expected, "{prefix}");
    }
}

#[test]
fn install_ps1_is_ascii() {
    // Windows PowerShell 5.1 reads a script without a byte order mark in the ANSI code page.
    let text = script("install.ps1");
    let line = text.lines().position(|line| !line.is_ascii());
    assert_eq!(
        line.map(|index| index + 1),
        None,
        "a line that is not ASCII"
    );
}

#[test]
fn neither_script_elevates() {
    for name in ["install.sh", "install.ps1"] {
        for line in script(name).lines() {
            let code = line.trim_start();
            if code.starts_with('#') {
                continue;
            }
            // Messages may say that the script never uses sudo; a command may not.
            let command = code
                .split(['"', '\''])
                .step_by(2)
                .collect::<Vec<_>>()
                .join(" ");
            for word in ["sudo", "doas", "runas", "-verb"] {
                assert!(
                    !command
                        .to_lowercase()
                        .split(|c: char| c.is_whitespace() || ";|&(){}".contains(c))
                        .any(|token| token == word),
                    "{name}: {line}"
                );
            }
        }
    }
}

/// Releases served by a local stand-in for GitHub, and the scripts run against them.
mod end_to_end {
    use std::fmt::Write as _;
    use std::fs;
    use std::io::{Cursor, Write as _};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::script;

    const REPOSITORY: &str = "shaharia-lab/jev-cli";
    const EXE: &str = std::env::consts::EXE_SUFFIX;

    /// The version the fake releases have: with the test hooks, `JEV_TEST_VERSION` makes the
    /// built `jev` report a pre-release version, as the installer's self-test checks; without
    /// them, the version it was built as.
    fn version() -> String {
        if cfg!(feature = "internal-test-hooks") {
            "0.2.0-rc.1".to_owned()
        } else {
            reported("version")
        }
    }

    fn reported(field: &str) -> String {
        let output = Command::new(assert_cmd::cargo::cargo_bin("jev"))
            .args(["version", "--field", field])
            .env(
                "JEV_CONFIG_DIR",
                Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
            )
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// The target whose archive the scripts ask for.
    fn target() -> String {
        reported("target").replace("-unknown-linux-gnu", "-unknown-linux-musl")
    }

    /// Whether the minisign CLI is on PATH. In CI it must be, so that no signature test is
    /// skipped there.
    fn minisign() -> bool {
        let found = Command::new("minisign")
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok();
        assert!(
            found || std::env::var("CI").as_deref() != Ok("true"),
            "CI must install minisign (scripts/ci/install-minisign.sh)"
        );
        if !found {
            eprintln!("minisign is not installed; skipping the signature checks");
        }
        found
    }

    /// A scratch directory, removed afterwards.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "install-{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("home")).unwrap();
            Self(dir)
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

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

    /// The files of a release, as the release workflow publishes them for this target.
    struct Release {
        version: String,
        archive: String,
        files: Vec<(String, Vec<u8>)>,
    }

    impl Release {
        /// Release `version` holding `binary`, signed by `signer`.
        fn new(version: &str, binary: &[u8], signer: &Signer) -> Self {
            let (archive, bytes) = archive(version, binary);
            let mut sums = format!("{}  {archive}\n", digest(&bytes));
            writeln!(sums, "{}  jev-{version}.cdx.json", "0".repeat(64)).unwrap();
            let files = vec![
                (
                    format!("{archive}.minisig"),
                    signer.sign(&bytes, &archive, version),
                ),
                (archive.clone(), bytes),
                (
                    "SHA256SUMS.minisig".to_owned(),
                    signer.sign(sums.as_bytes(), "SHA256SUMS", version),
                ),
                ("SHA256SUMS".to_owned(), sums.into_bytes()),
            ];
            Self {
                version: version.to_owned(),
                archive,
                files,
            }
        }

        /// The release of the built `jev`.
        fn of_jev(signer: &Signer) -> Self {
            Self::new(&version(), jev_bytes(), signer)
        }

        fn file(&mut self, name: &str) -> &mut Vec<u8> {
            &mut self
                .files
                .iter_mut()
                .find(|(file, _)| file == name)
                .unwrap()
                .1
        }

        /// Serves the release on `server`, as GitHub's latest stable release when `latest`.
        async fn serve(&self, server: &MockServer, latest: bool) {
            if latest {
                Mock::given(method("GET"))
                    .and(path(format!("/repos/{REPOSITORY}/releases/latest")))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                        "tag_name": format!("v{}", self.version),
                        "draft": false,
                        "prerelease": false,
                    })))
                    .mount(server)
                    .await;
            }
            for (file, bytes) in &self.files {
                Mock::given(method("GET"))
                    .and(path(format!(
                        "/{REPOSITORY}/releases/download/v{}/{file}",
                        self.version
                    )))
                    .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
                    .mount(server)
                    .await;
            }
        }
    }

    /// The built `jev`, read once. A debug build on Linux carries over 100 MB of debug
    /// information, which would only slow every test down, so it is stripped when `strip` exists.
    fn jev_bytes() -> &'static [u8] {
        static BYTES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        BYTES.get_or_init(|| {
            let built = assert_cmd::cargo::cargo_bin("jev");
            let stripped = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("install-jev-stripped-{}", std::process::id()));
            let bytes = if cfg!(target_os = "linux")
                && Command::new("strip")
                    .arg("--strip-debug")
                    .arg("-o")
                    .arg(&stripped)
                    .arg(&built)
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            {
                fs::read(&stripped).unwrap()
            } else {
                fs::read(&built).unwrap()
            };
            let _ = fs::remove_file(stripped);
            bytes
        })
    }

    /// The release archive of `version` holding `binary`, packed as package.sh packs it. The
    /// archive of the built `jev` is packed once.
    fn archive(version: &str, binary: &[u8]) -> (String, Vec<u8>) {
        static JEV: std::sync::OnceLock<(String, Vec<u8>)> = std::sync::OnceLock::new();
        if std::ptr::eq(binary, jev_bytes()) {
            return JEV.get_or_init(|| pack(version, binary)).clone();
        }
        pack(version, binary)
    }

    fn pack(version: &str, binary: &[u8]) -> (String, Vec<u8>) {
        let name = format!("jev-{version}-{}", target());
        if cfg!(windows) {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default();
            writer
                .start_file(format!("{name}/jev.exe"), options)
                .unwrap();
            writer.write_all(binary).unwrap();
            writer
                .start_file(format!("{name}/README.md"), options)
                .unwrap();
            writer.write_all(b"jev").unwrap();
            (format!("{name}.zip"), writer.finish().unwrap().into_inner())
        } else {
            let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
                Vec::new(),
                // Stored, not compressed: the archive only has to be valid gzip, and fast.
                flate2::Compression::none(),
            ));
            for (entry, data, mode) in [("jev", binary, 0o755), ("README.md", &b"jev"[..], 0o644)] {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(mode);
                header.set_cksum();
                builder
                    .append_data(&mut header, format!("{name}/{entry}"), data)
                    .unwrap();
            }
            (
                format!("{name}.tar.gz"),
                builder.into_inner().unwrap().finish().unwrap(),
            )
        }
    }

    /// A program that runs an install script.
    #[derive(Clone, Copy, Debug)]
    enum Shell {
        Sh,
        PowerShell(&'static str),
    }

    /// The shells to run the script for this platform with: `sh` on Unix; on Windows,
    /// Windows PowerShell and, when installed, PowerShell 7.
    fn shells() -> Vec<Shell> {
        if cfg!(windows) {
            let mut shells = vec![Shell::PowerShell("powershell")];
            if Command::new("pwsh")
                .args(["-NoProfile", "-Command", "exit 0"])
                .status()
                .is_ok_and(|status| status.success())
            {
                shells.push(Shell::PowerShell("pwsh"));
            }
            shells
        } else {
            vec![Shell::Sh]
        }
    }

    /// How the script is run.
    #[derive(Clone, Copy)]
    enum Form {
        /// From a file, with options as arguments.
        File,
        /// The one-line form in the README: the script's text piped into the shell, options
        /// from the environment.
        OneLine,
    }

    struct Run {
        code: i32,
        stdout: String,
        stderr: String,
    }

    impl Run {
        fn assert_success(&self) {
            assert_eq!(
                (self.code, self.stdout.as_str()),
                (0, ""),
                "stderr: {}",
                self.stderr
            );
        }

        fn assert_failure(&self, code: i32, message: &str) {
            assert_eq!(
                (self.code, self.stdout.as_str()),
                (code, ""),
                "stderr: {}",
                self.stderr
            );
            assert!(
                self.stderr.contains(message),
                "stderr does not contain {message:?}: {}",
                self.stderr
            );
        }
    }

    /// A run of the install script for `shell`, pointed at `server` and trusting `key`.
    struct Installer<'a> {
        shell: Shell,
        sandbox: &'a Sandbox,
        script: PathBuf,
        options: Vec<(&'static str, String)>,
        env: Vec<(&'static str, String)>,
    }

    impl<'a> Installer<'a> {
        /// `minisign` is the verifier the script looks for; a name that does not exist stands
        /// for a machine without one.
        fn new(
            shell: Shell,
            sandbox: &'a Sandbox,
            server: &MockServer,
            key: &str,
            minisign: &str,
        ) -> Self {
            let uri = server.uri();
            let (name, replacements) = match shell {
                Shell::Sh => (
                    "install.sh",
                    [
                        ("REPOSITORY_URL=", format!("'{uri}/{REPOSITORY}'")),
                        ("API_URL=", format!("'{uri}/repos/{REPOSITORY}'")),
                        ("ALLOW_HTTP=", "1".to_owned()),
                        ("MINISIGN=", minisign.to_owned()),
                        ("PUBLIC_KEYS=", format!("'{key}'")),
                    ],
                ),
                Shell::PowerShell(_) => (
                    "install.ps1",
                    [
                        ("$RepositoryUrl = ", format!("'{uri}/{REPOSITORY}'")),
                        ("$ApiUrl = ", format!("'{uri}/repos/{REPOSITORY}'")),
                        ("$AllowHttp = ", "$true".to_owned()),
                        ("$Minisign = ", format!("'{minisign}'")),
                        ("$PublicKeys = ", format!("@('{key}')")),
                    ],
                ),
            };
            let mut text = script(name);
            for (prefix, value) in replacements {
                let line = text
                    .lines()
                    .find(|line| line.trim_start().starts_with(prefix))
                    .unwrap()
                    .to_owned();
                let indent = &line[..line.len() - line.trim_start().len()];
                text = text.replacen(&line, &format!("{indent}{prefix}{value}"), 1);
            }
            let script = sandbox.path(name);
            fs::write(&script, text).unwrap();
            Self {
                shell,
                sandbox,
                script,
                options: Vec::new(),
                env: Vec::new(),
            }
        }

        /// Sets an option, by its install.sh name (`version`, `install-dir`, `no-modify-path`,
        /// `require-signature`); a switch has an empty value.
        fn option(mut self, name: &'static str, value: impl Into<String>) -> Self {
            self.options.push((name, value.into()));
            self
        }

        fn env(mut self, name: &'static str, value: impl Into<String>) -> Self {
            self.env.push((name, value.into()));
            self
        }

        fn arguments(&self) -> Vec<String> {
            let mut arguments = Vec::new();
            for (name, value) in &self.options {
                match self.shell {
                    Shell::Sh => arguments.push(format!("--{name}")),
                    Shell::PowerShell(_) => arguments.push(
                        match *name {
                            "version" => "-Version",
                            "install-dir" => "-InstallDir",
                            "no-modify-path" => "-NoModifyPath",
                            "require-signature" => "-RequireSignature",
                            other => panic!("no PowerShell name for --{other}"),
                        }
                        .to_owned(),
                    ),
                }
                if !value.is_empty() {
                    arguments.push(value.clone());
                }
            }
            arguments
        }

        fn run(&self, form: Form) -> Run {
            let mut command = match (self.shell, form) {
                (Shell::Sh, Form::File) => {
                    let mut command = Command::new("sh");
                    command.arg(&self.script).args(self.arguments());
                    command
                }
                (Shell::Sh, Form::OneLine) => {
                    let mut command = Command::new("sh");
                    command.args(["-s", "--"]).args(self.arguments());
                    command
                }
                (Shell::PowerShell(program), Form::File) => {
                    let mut command = Command::new(program);
                    command
                        .args([
                            "-NoProfile",
                            "-NonInteractive",
                            "-ExecutionPolicy",
                            "Bypass",
                        ])
                        .arg("-File")
                        .arg(&self.script)
                        .args(self.arguments());
                    command
                }
                (Shell::PowerShell(program), Form::OneLine) => {
                    let mut command = Command::new(program);
                    command.args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-Command",
                        "$input | Out-String | Invoke-Expression",
                    ]);
                    command
                }
            };
            if cfg!(unix) {
                // Only what a login shell would have, so a developer's settings cannot leak in.
                command.env_clear();
                command.env("PATH", std::env::var_os("PATH").unwrap());
                command.env("HOME", self.sandbox.path("home"));
                command.env("SHELL", "/bin/sh");
            } else {
                for (name, _) in std::env::vars() {
                    if name.starts_with("JEV_") || name.starts_with("TYPESAFE_") || name == "CI" {
                        command.env_remove(name);
                    }
                }
            }
            if cfg!(feature = "internal-test-hooks") {
                command.env("JEV_TEST_VERSION", version());
            }
            for (name, value) in &self.env {
                command.env(name, value);
            }
            let stdin = match form {
                Form::File => Stdio::null(),
                Form::OneLine => Stdio::piped(),
            };
            let mut child = command
                .stdin(stdin)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&fs::read(&self.script).unwrap()).unwrap();
            }
            let output = wait(child);
            Run {
                code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8(output.stdout).unwrap(),
                stderr: String::from_utf8(output.stderr).unwrap(),
            }
        }
    }

    /// Waits for `child`, failing the test when it takes more than two minutes.
    fn wait(child: std::process::Child) -> std::process::Output {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(child.wait_with_output()));
        receiver
            .recv_timeout(Duration::from_secs(120))
            .unwrap_or_else(|_| panic!("the install script did not finish in two minutes"))
            .unwrap()
    }

    /// Runs the installer on this thread, while the runtime's other threads keep the mock server
    /// answering.
    fn run(installer: &Installer<'_>, form: Form) -> Run {
        tokio::task::block_in_place(|| installer.run(form))
    }

    /// The SHA-256 of `data`, to compare binaries without printing them.
    fn digest(data: &[u8]) -> String {
        let mut hex = String::new();
        for byte in ring::digest::digest(&ring::digest::SHA256, data).as_ref() {
            write!(hex, "{byte:02x}").unwrap();
        }
        hex
    }

    fn installed(dir: &Path) -> PathBuf {
        dir.join(format!("jev{EXE}"))
    }

    fn receipt(dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(dir.join(".jev-update/receipt.json")).unwrap()).unwrap()
    }

    /// The names in `dir`, except the updater's directory.
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|name| name != ".jev-update")
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn installer_name(shell: Shell) -> &'static str {
        match shell {
            Shell::Sh => "install.sh",
            Shell::PowerShell(_) => "install.ps1",
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn installs_the_latest_release_verified_with_a_receipt_for_jev_update() {
        let signer = Signer::new();
        let server = MockServer::start().await;
        let release = Release::of_jev(&signer);
        release.serve(&server, true).await;
        let verifier = minisign();

        for shell in shells() {
            let sandbox = Sandbox::new("latest");
            let dir = sandbox.path("bin");
            let result = run(
                &Installer::new(shell, &sandbox, &server, &signer.public(), "minisign")
                    .option("install-dir", dir.to_str().unwrap())
                    .option("no-modify-path", ""),
                Form::File,
            );

            result.assert_success();
            let version = version();
            assert!(
                result.stderr.contains(&format!(
                    "Installed jev {version} ({}) to {}",
                    target(),
                    installed(&dir).display()
                )),
                "{shell:?}: {}",
                result.stderr
            );
            if verifier {
                assert!(
                    result.stderr.contains(
                        "Verified: the SHA-256 checksum, and the minisign signatures of the archive and SHA256SUMS"
                    ),
                    "{shell:?}: {}",
                    result.stderr
                );
            }
            assert!(
                result.stderr.contains("is not on your PATH"),
                "{shell:?}: {}",
                result.stderr
            );
            assert_eq!(
                receipt(&dir),
                json!({ "installer": installer_name(shell), "version": version, "target": target() })
            );
            assert_eq!(entries(&dir), [format!("jev{EXE}")], "nothing else is left");

            let mut command = Command::new(installed(&dir));
            command
                .args(["version", "-o", "json"])
                .env("JEV_CONFIG_DIR", sandbox.path("config"))
                .env("CI", "true");
            if cfg!(feature = "internal-test-hooks") {
                command.env("JEV_TEST_VERSION", &version);
            }
            let output = command.output().unwrap();
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["version"], version.as_str());
            assert_eq!(
                report["install_method"], "self_managed",
                "jev update recognises the install"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_one_line_form_installs_a_named_version_from_the_environment() {
        let signer = Signer::new();
        let server = MockServer::start().await;
        let release = Release::of_jev(&signer);
        // Not the latest: a pre-release is only installed when it is named.
        release.serve(&server, false).await;

        for shell in shells() {
            let sandbox = Sandbox::new("one-line");
            let dir = sandbox.path("bin");
            let result = run(
                &Installer::new(shell, &sandbox, &server, &signer.public(), "minisign")
                    .env("JEV_INSTALL_VERSION", format!("v{}", version()))
                    .env("JEV_INSTALL_DIR", dir.to_str().unwrap())
                    .env("JEV_INSTALL_NO_MODIFY_PATH", "1"),
                Form::OneLine,
            );

            result.assert_success();
            assert_eq!(receipt(&dir)["version"], version());
            assert!(installed(&dir).is_file());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_minisign_the_checksum_is_verified_and_the_signature_reported_unchecked() {
        let signer = Signer::new();
        let server = MockServer::start().await;
        let release = Release::of_jev(&signer);
        release.serve(&server, true).await;

        for shell in shells() {
            let sandbox = Sandbox::new("no-minisign");
            let installer = |dir: &Path| {
                Installer::new(
                    shell,
                    &sandbox,
                    &server,
                    &signer.public(),
                    "jev-test-no-minisign",
                )
                .option("install-dir", dir.to_str().unwrap())
                .option("no-modify-path", "")
            };

            let checksum_only = run(&installer(&sandbox.path("bin")), Form::File);
            let required = run(
                &installer(&sandbox.path("required")).option("require-signature", ""),
                Form::File,
            );

            checksum_only.assert_success();
            assert!(
                checksum_only.stderr.contains(
                    "Verified: the SHA-256 checksum against SHA256SUMS.\nNot verified: the signature, because minisign is not installed."
                ),
                "{shell:?}: {}",
                checksum_only.stderr
            );
            required.assert_failure(1, "but minisign is not installed");
            assert!(!sandbox.path("required").exists(), "nothing was installed");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_corrupted_download_installs_nothing() {
        let signer = Signer::new();
        let server = MockServer::start().await;
        let mut release = Release::of_jev(&signer);
        let archive = release.archive.clone();
        release.file(&archive).push(0);
        release.serve(&server, true).await;
        let verifier = minisign();

        for shell in shells() {
            let sandbox = Sandbox::new("corrupt");
            let installer = |minisign: &str| {
                Installer::new(shell, &sandbox, &server, &signer.public(), minisign)
                    .option("install-dir", sandbox.path("bin").to_str().unwrap())
                    .option("no-modify-path", "")
            };

            run(&installer("jev-test-no-minisign"), Form::File).assert_failure(
                1,
                &format!("the SHA-256 of {archive} does not match SHA256SUMS"),
            );
            if verifier {
                run(&installer("minisign"), Form::File).assert_failure(
                    1,
                    &format!("{archive}.minisig is not a valid signature of {archive}"),
                );
            }
            assert!(!sandbox.path("bin").exists(), "nothing was installed");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_signed_by_another_key_or_for_another_release_installs_nothing() {
        if !minisign() {
            return;
        }
        let signer = Signer::new();
        let server = MockServer::start().await;
        let mut release = Release::of_jev(&signer);
        let sums = release.file("SHA256SUMS").clone();
        // A genuine signature of SHA256SUMS, but made for another release.
        *release.file("SHA256SUMS.minisig") = signer.sign(&sums, "SHA256SUMS", "0.0.1");
        release.serve(&server, true).await;
        let other = Signer::new();

        for shell in shells() {
            let sandbox = Sandbox::new("signature");
            let installer = |key: &str| {
                Installer::new(shell, &sandbox, &server, key, "minisign")
                    .option("install-dir", sandbox.path("bin").to_str().unwrap())
                    .option("no-modify-path", "")
            };

            run(&installer(&other.public()), Form::File).assert_failure(
                1,
                "SHA256SUMS.minisig is not a valid signature of SHA256SUMS by a jev release key",
            );
            run(&installer(&signer.public()), Form::File).assert_failure(
                1,
                "SHA256SUMS.minisig is a valid signature, but for another file or release",
            );
            assert!(!sandbox.path("bin").exists(), "nothing was installed");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_binary_that_does_not_run_is_not_installed() {
        let signer = Signer::new();
        let server = MockServer::start().await;
        Release::new(&version(), b"not a program", &signer)
            .serve(&server, true)
            .await;

        for shell in shells() {
            let sandbox = Sandbox::new("not-a-program");
            let dir = sandbox.path("bin");
            let result = run(
                &Installer::new(
                    shell,
                    &sandbox,
                    &server,
                    &signer.public(),
                    "jev-test-no-minisign",
                )
                .option("install-dir", dir.to_str().unwrap())
                .option("no-modify-path", ""),
                Form::File,
            );

            result.assert_failure(1, "does not run on this system");
            assert_eq!(entries(&dir), Vec::<String>::new(), "nothing was installed");
            assert!(!dir.join(".jev-update/receipt.json").exists());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn installing_again_replaces_the_binary() {
        let signer = Signer::new();
        let server = MockServer::start().await;
        Release::of_jev(&signer).serve(&server, true).await;

        for shell in shells() {
            let sandbox = Sandbox::new("again");
            let dir = sandbox.path("bin");
            fs::create_dir_all(&dir).unwrap();
            fs::write(installed(&dir), "an older jev").unwrap();
            let installer = || {
                Installer::new(
                    shell,
                    &sandbox,
                    &server,
                    &signer.public(),
                    "jev-test-no-minisign",
                )
                .option("install-dir", dir.to_str().unwrap())
                .option("no-modify-path", "")
            };

            run(&installer(), Form::File).assert_success();
            run(&installer(), Form::File).assert_success();

            assert_eq!(
                digest(&fs::read(installed(&dir)).unwrap()),
                digest(jev_bytes())
            );
            assert_eq!(entries(&dir), [format!("jev{EXE}")]);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_a_stable_release_the_error_says_to_name_a_version() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/repos/{REPOSITORY}/releases/latest")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        for shell in shells() {
            let sandbox = Sandbox::new("no-stable");
            let result = run(
                &Installer::new(
                    shell,
                    &sandbox,
                    &server,
                    &Signer::new().public(),
                    "minisign",
                )
                .option("install-dir", sandbox.path("bin").to_str().unwrap()),
                Form::File,
            );

            result.assert_failure(1, "could not find the latest stable release of jev");
            assert!(
                result.stderr.contains("install a pre-release with"),
                "{}",
                result.stderr
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_malformed_version_is_a_usage_error() {
        let server = MockServer::start().await;

        for shell in shells() {
            let sandbox = Sandbox::new("usage");
            run(
                &Installer::new(
                    shell,
                    &sandbox,
                    &server,
                    &Signer::new().public(),
                    "minisign",
                )
                .option("version", "latest; rm -rf /")
                .option("install-dir", sandbox.path("bin").to_str().unwrap()),
                Form::File,
            )
            .assert_failure(2, "is not a version such as 0.1.0");
            assert!(server.received_requests().await.unwrap().is_empty());
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn by_default_jev_goes_to_local_bin_and_on_path_through_the_shell_profile_once() {
        let signer = Signer::new();
        let server = MockServer::start().await;
        Release::of_jev(&signer).serve(&server, true).await;
        let sandbox = Sandbox::new("path");
        let profile = if cfg!(target_os = "macos") {
            ".bash_profile"
        } else {
            ".bashrc"
        };
        let installer = || {
            Installer::new(Shell::Sh, &sandbox, &server, &signer.public(), "minisign")
                .env("SHELL", "/bin/bash")
        };

        let first = run(&installer(), Form::File);
        let second = run(&installer(), Form::File);

        first.assert_success();
        second.assert_success();
        assert!(installed(&sandbox.path("home/.local/bin")).is_file());
        let line = "export PATH=\"$HOME/.local/bin:$PATH\"";
        assert_eq!(
            fs::read_to_string(sandbox.path("home").join(profile)).unwrap(),
            format!("\n# Added by the jev installer\n{line}\n")
        );
        assert!(
            first.stderr.contains("Added $HOME/.local/bin to PATH in "),
            "{}",
            first.stderr
        );
        assert!(
            second
                .stderr
                .contains("already adds $HOME/.local/bin to PATH"),
            "{}",
            second.stderr
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unsupported_platform_is_pointed_at_cargo_install() {
        use std::os::unix::fs::PermissionsExt as _;

        let server = MockServer::start().await;
        for (system, machine, message) in [
            (
                "Linux",
                "riscv64",
                "no prebuilt jev for the riscv64 architecture",
            ),
            ("FreeBSD", "x86_64", "no prebuilt jev for FreeBSD"),
        ] {
            let sandbox = Sandbox::new("unsupported");
            let fake = sandbox.path("fake-bin");
            fs::create_dir_all(&fake).unwrap();
            let uname = fake.join("uname");
            fs::write(
                &uname,
                format!("#!/bin/sh\ncase $1 in -s) echo {system} ;; -m) echo {machine} ;; esac\n"),
            )
            .unwrap();
            fs::set_permissions(&uname, fs::Permissions::from_mode(0o755)).unwrap();
            let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap());

            let result = run(
                &Installer::new(
                    Shell::Sh,
                    &sandbox,
                    &server,
                    &Signer::new().public(),
                    "minisign",
                )
                .option("install-dir", sandbox.path("bin").to_str().unwrap())
                .env("PATH", path),
                Form::File,
            );

            result.assert_failure(1, message);
            assert!(
                result.stderr.contains("cargo install jev-cli --locked"),
                "{}",
                result.stderr
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
