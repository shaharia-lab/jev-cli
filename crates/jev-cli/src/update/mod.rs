//! The updater: find a release, download it, verify it, stage it, swap it in, and roll it back.
//!
//! Trust comes from the release public keys compiled into the binary ([`verify`]), never from the
//! network. A release is only ever read from GitHub Releases of this repository ([`github`]), and
//! nothing is written next to the binary until the archive's signature, `SHA256SUMS`'s signature
//! and the archive's checksum have all been checked. Each signature's trusted comment must name
//! the file and the version it was made for, so a signature cannot be replayed from another file
//! or an older release.
//!
//! The steps are separate so that the automatic background update can reuse them: [`newest`]
//! only asks, [`stage`] downloads and verifies into the staging directory, and [`Installation`]
//! swaps a staged binary in (keeping the previous one for rollback) and runs its self-test.
//! `jev update` does all of them at once.

mod archive;
mod github;
mod install;
mod method;
mod verify;

use semver::Version;
use serde::Serialize;

use crate::env::Env;
use crate::error::CliError;
use crate::exit::Exit;

pub(crate) use github::GitHubReleases;
pub(crate) use install::{Installation, probe};
pub(crate) use method::InstallMethod;
pub(crate) use verify::TrustedKeys;

/// The repository whose releases are the only source of updates.
pub(crate) const REPOSITORY: &str = "shaharia-lab/jev-cli";

/// No update, automatic or asked for with `jev update --version`, installs a release older than
/// this. Raise it in a release to stop anyone being moved to releases known to be unsafe.
pub(crate) const MINIMUM_VERSION: Version = Version::new(0, 1, 0);

/// The only update channel so far: published, non-draft, non-pre-release versions.
pub(crate) const CHANNEL: &str = "stable";

/// The largest archive that is downloaded. A release binary is at most 15 MB (NFR-SIZE-1).
const MAX_ARCHIVE_BYTES: u64 = 100 * 1024 * 1024;

/// The largest binary read out of a verified archive: far above a release binary, and above an
/// unstripped debug build, which is what the tests install.
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

/// The largest `SHA256SUMS` or signature that is downloaded; the real ones are a few hundred bytes.
const MAX_SMALL_FILE_BYTES: u64 = 64 * 1024;

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub(crate) version: Version,
    /// The release page, with its changelog.
    pub(crate) url: String,
    /// The names of the files attached to it.
    pub(crate) assets: Vec<String>,
}

/// Where releases come from. The real one is [`GitHubReleases`]; tests use a fake.
pub(crate) trait UpdateSource {
    /// The newest published stable release, or `None` when there is none yet.
    ///
    /// # Errors
    ///
    /// When the source cannot be reached or answers with something unreadable.
    fn latest(&self) -> Result<Option<Release>, CliError>;

    /// The published release of `version`, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// As for [`UpdateSource::latest`].
    fn release(&self, version: &Version) -> Result<Option<Release>, CliError>;

    /// One file attached to `release`, refusing to read more than `limit` bytes.
    ///
    /// # Errors
    ///
    /// When the download fails, is cut short or is larger than `limit`.
    fn download(&self, release: &Release, asset: &str, limit: u64) -> Result<Vec<u8>, CliError>;
}

/// The version of the running `jev`.
pub(crate) fn current_version(env: &Env) -> Version {
    #[cfg(feature = "internal-test-hooks")]
    if let Some(version) = env
        .get("JEV_TEST_VERSION")
        .and_then(|text| Version::parse(text).ok())
    {
        return version;
    }
    let _ = env;
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// The target triple whose archive this `jev` updates from.
///
/// Linux releases are statically linked musl builds, which run wherever a glibc build does, so a
/// glibc build (from `cargo build`, say) updates to the musl archive.
pub(crate) fn release_target() -> String {
    env!("JEV_BUILD_TARGET").replace("-unknown-linux-gnu", "-unknown-linux-musl")
}

/// What the newest release is, compared with `current`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Newest {
    /// Nothing has been released yet.
    NoRelease,
    /// The newest release is `current`, or older (which is never installed automatically).
    UpToDate(Release),
    /// A newer release that an automatic update may install.
    Available(Release),
}

/// Asks the source for its newest release, and whether it is an update from `current`.
///
/// Only a version newer than `current`, and not older than [`MINIMUM_VERSION`], counts as an
/// update: an automatic update never downgrades.
///
/// # Errors
///
/// Whatever the source fails with.
pub(crate) fn newest(source: &dyn UpdateSource, current: &Version) -> Result<Newest, CliError> {
    Ok(match source.latest()? {
        None => Newest::NoRelease,
        Some(release) if is_upgrade(current, &release.version) => Newest::Available(release),
        Some(release) => Newest::UpToDate(release),
    })
}

/// Whether moving from `current` to `candidate` is an update an automatic path may make.
pub(crate) fn is_upgrade(current: &Version, candidate: &Version) -> bool {
    candidate > current && *candidate >= MINIMUM_VERSION
}

/// Downloads `release` for this platform, verifies it against `keys`, and stages its binary in
/// `installation`, ready to be swapped in.
///
/// Nothing is written until every check has passed: the signature of `SHA256SUMS`, the signature
/// of the archive, and the archive's checksum in `SHA256SUMS`.
///
/// # Errors
///
/// A download that fails, a release without an archive for this platform, or any failed check.
/// The installed binary is untouched in every case.
pub(crate) fn stage(
    source: &dyn UpdateSource,
    release: &Release,
    keys: &TrustedKeys,
    installation: &Installation,
) -> Result<(), CliError> {
    let target = release_target();
    let archive = archive::name(&release.version, &target);
    for file in [archive.as_str(), "SHA256SUMS"] {
        for asset in [file.to_owned(), format!("{file}.minisig")] {
            if !release.assets.contains(&asset) {
                return Err(CliError::update(
                    "update_asset_missing",
                    Exit::Internal,
                    format!("release {} has no `{asset}`", release.version),
                )
                .hint(format!(
                    "there may be no build for {target}; see {} or install with `cargo install jev-cli --locked`",
                    release.url
                )));
            }
        }
    }

    let sums = source.download(release, "SHA256SUMS", MAX_SMALL_FILE_BYTES)?;
    let sums_signature = source.download(release, "SHA256SUMS.minisig", MAX_SMALL_FILE_BYTES)?;
    let archive_signature =
        source.download(release, &format!("{archive}.minisig"), MAX_SMALL_FILE_BYTES)?;
    let archive_bytes = source.download(release, &archive, MAX_ARCHIVE_BYTES)?;

    let failed = |problem: String| {
        CliError::update(
            "update_verification_failed",
            Exit::Internal,
            format!(
                "the download of jev {} failed verification: {problem}",
                release.version
            ),
        )
        .hint(
            "nothing was changed. Try again later; if it keeps failing, report it privately as \
SECURITY.md describes, because it may mean the download was tampered with",
        )
    };
    keys.verify(&sums, &sums_signature, "SHA256SUMS", &release.version)
        .map_err(failed)?;
    keys.verify(
        &archive_bytes,
        &archive_signature,
        &archive,
        &release.version,
    )
    .map_err(failed)?;
    let sums =
        std::str::from_utf8(&sums).map_err(|_| failed("SHA256SUMS is not text".to_owned()))?;
    let listed = verify::listed_checksum(sums, &archive)
        .ok_or_else(|| failed(format!("SHA256SUMS does not list {archive}")))?;
    let actual = verify::sha256_hex(&archive_bytes);
    if !listed.eq_ignore_ascii_case(&actual) {
        return Err(failed(format!(
            "the SHA-256 of {archive} is {actual}, SHA256SUMS says {listed}"
        )));
    }

    let entry = archive::binary_entry(&release.version, &target);
    let binary = archive::extract(&archive_bytes, &archive, &entry, MAX_BINARY_BYTES)
        .map_err(|problem| failed(format!("{archive}: {problem}")))?;
    installation.stage(&binary, &release.version)
}

/// Whether automatic updates are on for this `jev`, and why not when they are off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AutoUpdate {
    pub(crate) enabled: bool,
    /// Why they are off; `null` when they are on.
    pub(crate) reason: Option<String>,
}

impl AutoUpdate {
    /// The status for an install made by `method`, in `env`.
    pub(crate) fn status(method: InstallMethod, env: &Env) -> Self {
        let off = |reason: String| Self {
            enabled: false,
            reason: Some(reason),
        };
        if let Some(manager) = method.package_manager() {
            return off(format!(
                "installed with {manager}, which updates it: {}",
                method.upgrade_command(None).unwrap_or_default()
            ));
        }
        if env
            .get("CI")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        {
            return off("CI=true".to_owned());
        }
        off("automatic updates are not in this version of jev yet; run `jev update`".to_owned())
    }
}

#[cfg(test)]
mod tests;
