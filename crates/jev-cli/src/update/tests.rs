//! The updater against a fake release source: what gets installed, and what never does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use semver::Version;

use super::{
    Installation, Newest, Release, TrustedKeys, UpdateSource, archive, is_upgrade, newest,
    release_target, stage, verify,
};
use crate::error::CliError;
use crate::exit::Exit;

/// A new, empty directory for one test.
pub(crate) fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jev-update-unit-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A throwaway release-signing key.
pub(crate) struct Signer(minisign::KeyPair);

impl Signer {
    pub(crate) fn new() -> Self {
        Self(minisign::KeyPair::generate_unencrypted_keypair().unwrap())
    }

    /// Trusts this key alone.
    pub(crate) fn keys(&self) -> TrustedKeys {
        TrustedKeys(vec![
            minisign_verify::PublicKey::from_base64(&self.0.pk.to_base64()).unwrap(),
        ])
    }

    /// Signs as the release workflow does: the trusted comment names the file and the version.
    pub(crate) fn sign(&self, data: &[u8], file: &str, version: &Version) -> Vec<u8> {
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

/// A `.tar.gz` holding `files`.
pub(crate) fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for (path, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, path, *contents).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

/// A `.zip` holding `files`.
pub(crate) fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, contents) in files {
        writer
            .start_file(*path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Releases served from memory, recording what was downloaded.
#[derive(Default)]
struct FakeSource {
    releases: Vec<Release>,
    files: HashMap<(Version, String), Vec<u8>>,
    /// Downloads of these files are cut short.
    interrupted: Vec<String>,
    downloads: RefCell<Vec<String>>,
}

impl FakeSource {
    /// Publishes `version` with `binary` inside, signed by `signer`, exactly as the release
    /// workflow lays a release out.
    fn publish(&mut self, version: &Version, binary: &[u8], signer: &Signer) {
        let target = release_target();
        let name = archive::name(version, &target);
        let entry = archive::binary_entry(version, &target);
        let files: &[(&str, &[u8])] = &[(&entry, binary)];
        let bytes = if target.contains("-windows-") {
            zip(files)
        } else {
            tar_gz(files)
        };
        let sums = format!("{}  {name}\n", verify::sha256_hex(&bytes));
        self.put(version, "SHA256SUMS", sums.as_bytes(), signer);
        self.put(version, &name, &bytes, signer);
        self.releases.push(Release {
            version: version.clone(),
            url: format!("https://example.test/v{version}"),
            assets: self
                .files
                .keys()
                .filter(|(of, _)| of == version)
                .map(|(_, asset)| asset.clone())
                .collect(),
        });
    }

    /// Adds a file and its signature.
    fn put(&mut self, version: &Version, name: &str, bytes: &[u8], signer: &Signer) {
        self.files.insert(
            (version.clone(), format!("{name}.minisig")),
            signer.sign(bytes, name, version),
        );
        self.files
            .insert((version.clone(), name.to_owned()), bytes.to_vec());
    }

    fn release_of(&self, version: &Version) -> Release {
        self.release(version).unwrap().unwrap()
    }

    fn archive(version: &Version) -> String {
        archive::name(version, &release_target())
    }
}

impl UpdateSource for FakeSource {
    fn latest(&self) -> Result<Option<Release>, CliError> {
        Ok(self
            .releases
            .iter()
            .max_by(|a, b| a.version.cmp(&b.version))
            .cloned())
    }

    fn release(&self, version: &Version) -> Result<Option<Release>, CliError> {
        Ok(self
            .releases
            .iter()
            .find(|release| release.version == *version)
            .cloned())
    }

    fn download(&self, release: &Release, asset: &str, limit: u64) -> Result<Vec<u8>, CliError> {
        self.downloads.borrow_mut().push(asset.to_owned());
        if self.interrupted.iter().any(|name| name == asset) {
            return Err(CliError::update(
                "connection",
                Exit::Network,
                format!("the download of {asset} was interrupted"),
            ));
        }
        let bytes = self
            .files
            .get(&(release.version.clone(), asset.to_owned()))
            .cloned()
            .unwrap();
        assert!(bytes.len() as u64 <= limit, "{asset} is over its limit");
        Ok(bytes)
    }
}

/// A pretend installed binary.
fn installed() -> Installation {
    let exe = scratch().join(format!("jev{}", std::env::consts::EXE_SUFFIX));
    fs::write(&exe, "old binary").unwrap();
    Installation::new(exe)
}

fn contents(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// Stages `version` from `source` and, when that works, swaps it in.
fn update(
    source: &FakeSource,
    keys: &TrustedKeys,
    installation: &Installation,
    version: &Version,
) -> Result<(), CliError> {
    stage(source, &source.release_of(version), keys, installation)?;
    installation.apply(version, &|_| Ok(()))
}

/// The installation is exactly as it was: same binary, nothing staged, nothing kept.
fn untouched(installation: &Installation) {
    assert_eq!(contents(installation.exe()), "old binary");
    assert_eq!(installation.staged(), None);
    assert!(!installation.previous().exists());
}

#[test]
fn a_valid_release_is_verified_staged_and_swapped_in() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let version = Version::new(0, 2, 0);
    source.publish(&version, b"new binary", &signer);
    let installation = installed();

    assert_eq!(
        newest(&source, &Version::new(0, 1, 0)).unwrap(),
        Newest::Available(source.release_of(&version))
    );
    update(&source, &signer.keys(), &installation, &version).unwrap();

    assert_eq!(contents(installation.exe()), "new binary");
    assert_eq!(contents(&installation.previous()), "old binary");
}

#[test]
fn a_release_signed_by_another_key_is_refused() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let version = Version::new(0, 2, 0);
    source.publish(&version, b"new binary", &Signer::new());
    let installation = installed();

    let error = update(&source, &signer.keys(), &installation, &version).unwrap_err();

    assert_eq!(
        (error.code, error.exit),
        ("update_verification_failed", Exit::Internal)
    );
    assert!(
        error
            .message
            .contains("SHA256SUMS.minisig is not a valid signature"),
        "{}",
        error.message
    );
    untouched(&installation);
}

#[test]
fn an_archive_that_does_not_match_its_signature_is_refused() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let version = Version::new(0, 2, 0);
    source.publish(&version, b"new binary", &signer);
    let name = FakeSource::archive(&version);
    source
        .files
        .get_mut(&(version.clone(), name.clone()))
        .unwrap()
        .push(0);
    let installation = installed();

    let error = update(&source, &signer.keys(), &installation, &version).unwrap_err();

    assert_eq!(error.code, "update_verification_failed");
    assert!(
        error
            .message
            .contains(&format!("{name}.minisig is not a valid signature"))
    );
    untouched(&installation);
}

#[test]
fn a_checksum_that_does_not_match_is_refused_even_when_everything_is_signed() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let version = Version::new(0, 2, 0);
    source.publish(&version, b"new binary", &signer);
    let name = FakeSource::archive(&version);
    let wrong = format!("{}  {name}\n", "0".repeat(64));
    source.put(&version, "SHA256SUMS", wrong.as_bytes(), &signer);
    let installation = installed();

    let error = update(&source, &signer.keys(), &installation, &version).unwrap_err();

    assert_eq!(error.code, "update_verification_failed");
    assert!(
        error.message.contains("SHA256SUMS says 0000"),
        "{}",
        error.message
    );
    untouched(&installation);
}

#[test]
fn signatures_replayed_from_an_older_release_are_refused() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let old = Version::new(0, 1, 0);
    source.publish(&old, b"old release", &signer);
    // A "0.3.0" whose files are really 0.1.0's, signatures and all.
    let new = Version::new(0, 3, 0);
    let old_archive = FakeSource::archive(&old);
    let new_archive = FakeSource::archive(&new);
    for (from, to) in [
        ("SHA256SUMS".to_owned(), "SHA256SUMS".to_owned()),
        (old_archive.clone(), new_archive.clone()),
    ] {
        for suffix in ["", ".minisig"] {
            let bytes = source.files[&(old.clone(), format!("{from}{suffix}"))].clone();
            source
                .files
                .insert((new.clone(), format!("{to}{suffix}")), bytes);
        }
    }
    source.releases.push(Release {
        version: new.clone(),
        url: String::new(),
        assets: source
            .files
            .keys()
            .filter(|(of, _)| *of == new)
            .map(|(_, asset)| asset.clone())
            .collect(),
    });
    let installation = installed();

    let error = update(&source, &signer.keys(), &installation, &new).unwrap_err();

    assert_eq!(error.code, "update_verification_failed");
    assert!(
        error.message.contains("another file or release"),
        "{}",
        error.message
    );
    untouched(&installation);
}

#[test]
fn an_automatic_update_never_downgrades_or_goes_below_the_minimum() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let latest = Version::new(0, 2, 0);
    source.publish(&latest, b"new binary", &signer);

    assert_eq!(
        newest(&source, &Version::new(0, 3, 0)).unwrap(),
        Newest::UpToDate(source.release_of(&latest))
    );
    assert_eq!(
        newest(&source, &latest).unwrap(),
        Newest::UpToDate(source.release_of(&latest))
    );
    assert!(
        source.downloads.borrow().is_empty(),
        "a check downloads nothing"
    );
    assert_eq!(
        newest(&FakeSource::default(), &latest).unwrap(),
        Newest::NoRelease
    );

    assert!(is_upgrade(&Version::new(0, 1, 0), &Version::new(0, 1, 1)));
    assert!(!is_upgrade(&Version::new(0, 1, 1), &Version::new(0, 1, 0)));
    assert!(
        !is_upgrade(&Version::new(0, 0, 0), &Version::new(0, 0, 9)),
        "below the minimum"
    );
}

#[test]
fn an_interrupted_download_changes_nothing() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let version = Version::new(0, 2, 0);
    source.publish(&version, b"new binary", &signer);
    source.interrupted.push(FakeSource::archive(&version));
    let installation = installed();

    let error = update(&source, &signer.keys(), &installation, &version).unwrap_err();

    assert_eq!((error.code, error.exit), ("connection", Exit::Network));
    untouched(&installation);
}

#[test]
fn a_release_without_a_build_for_this_platform_downloads_nothing() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let version = Version::new(0, 2, 0);
    source.publish(&version, b"new binary", &signer);
    let archive = FakeSource::archive(&version);
    source.releases[0].assets.retain(|asset| *asset != archive);
    let installation = installed();

    let error = update(&source, &signer.keys(), &installation, &version).unwrap_err();

    assert_eq!(error.code, "update_asset_missing");
    assert!(error.hint.unwrap().contains("cargo install jev-cli"));
    assert!(source.downloads.borrow().is_empty());
    untouched(&installation);
}

#[test]
fn a_new_binary_that_fails_its_self_test_is_rolled_back() {
    let signer = Signer::new();
    let mut source = FakeSource::default();
    let version = Version::new(0, 2, 0);
    source.publish(&version, b"broken binary", &signer);
    let installation = installed();

    stage(
        &source,
        &source.release_of(&version),
        &signer.keys(),
        &installation,
    )
    .unwrap();
    let error = installation
        .apply(&version, &|_| Err("it crashed".to_owned()))
        .unwrap_err();

    assert_eq!(error.code, "update_self_test_failed");
    assert_eq!(contents(installation.exe()), "old binary");
}
