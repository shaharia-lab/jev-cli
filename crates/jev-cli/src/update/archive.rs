//! Release archives, as `scripts/release/package.sh` packs them: `jev-<version>-<target>.tar.gz`,
//! or `.zip` for Windows, holding one directory of the same name with the binary inside.
//!
//! An archive is only opened after its signature and checksum have been verified, and only the
//! binary is read out of it, into memory: no path from the archive is ever used on disk.

use std::io::{Cursor, Read};

use semver::Version;

/// The archive of `version` for `target`.
pub(crate) fn name(version: &Version, target: &str) -> String {
    let extension = if is_windows(target) { "zip" } else { "tar.gz" };
    format!("jev-{version}-{target}.{extension}")
}

/// The path of the binary inside that archive.
pub(crate) fn binary_entry(version: &Version, target: &str) -> String {
    let binary = if is_windows(target) { "jev.exe" } else { "jev" };
    format!("jev-{version}-{target}/{binary}")
}

fn is_windows(target: &str) -> bool {
    target.contains("-windows-")
}

/// Reads the file at `entry` out of the archive `name`, refusing more than `limit` bytes.
///
/// # Errors
///
/// What is wrong, for a person to read.
pub(crate) fn extract(
    archive: &[u8],
    name: &str,
    entry: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let is_zip = std::path::Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"));
    let file = if is_zip {
        from_zip(archive, entry, limit)?
    } else {
        from_tar_gz(archive, entry, limit)?
    };
    file.ok_or_else(|| format!("it does not contain {entry}"))
}

fn from_tar_gz(archive: &[u8], entry: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let unreadable =
        |error: std::io::Error| format!("it is not a readable tar.gz archive: {error}");
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for file in tar.entries().map_err(unreadable)? {
        let file = file.map_err(unreadable)?;
        let is_binary = file.header().entry_type().is_file()
            && file.path().is_ok_and(|path| path.to_str() == Some(entry));
        if is_binary {
            return read_capped(file, limit).map(Some);
        }
    }
    Ok(None)
}

fn from_zip(archive: &[u8], entry: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))
        .map_err(|error| format!("it is not a readable zip archive: {error}"))?;
    let file = match zip.by_name(entry) {
        Ok(file) if file.is_file() => file,
        Ok(_) | Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(format!("it is not a readable zip archive: {error}")),
    };
    read_capped(file, limit).map(Some)
}

fn read_capped(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("the binary could not be read: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("the binary is larger than {limit} bytes"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{binary_entry, extract, name};
    use crate::update::tests::{tar_gz, zip};

    #[test]
    fn names_follow_the_release_layout() {
        let version = Version::new(0, 2, 0);

        assert_eq!(
            name(&version, "x86_64-unknown-linux-musl"),
            "jev-0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            name(&version, "aarch64-pc-windows-msvc"),
            "jev-0.2.0-aarch64-pc-windows-msvc.zip"
        );
        assert_eq!(
            binary_entry(&version, "aarch64-apple-darwin"),
            "jev-0.2.0-aarch64-apple-darwin/jev"
        );
        assert_eq!(
            binary_entry(&version, "x86_64-pc-windows-msvc"),
            "jev-0.2.0-x86_64-pc-windows-msvc/jev.exe"
        );
    }

    #[test]
    fn the_binary_comes_out_of_either_format() {
        let files = [
            ("jev-1/README.md", &b"readme"[..]),
            ("jev-1/jev", b"binary"),
        ];

        for (archive, name) in [(tar_gz(&files), "a.tar.gz"), (zip(&files), "a.zip")] {
            assert_eq!(
                extract(&archive, name, "jev-1/jev", 100),
                Ok(b"binary".to_vec())
            );
            assert!(
                extract(&archive, name, "jev-1/jev.exe", 100)
                    .unwrap_err()
                    .contains("does not contain jev-1/jev.exe")
            );
            assert!(
                extract(&archive, name, "jev-1/jev", 3)
                    .unwrap_err()
                    .contains("larger than 3 bytes")
            );
            assert!(
                extract(b"not an archive", name, "jev-1/jev", 100)
                    .unwrap_err()
                    .contains("not a readable")
            );
        }
    }
}
