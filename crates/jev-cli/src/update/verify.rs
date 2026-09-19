//! Signature and checksum checks, against the release public keys compiled into the binary.

use std::fmt::Write as _;

use minisign_verify::{PublicKey, Signature};
use semver::Version;

use crate::env::Env;

/// The key that signs every release.
const PRIMARY_KEY: &str = include_str!("../../keys/release-primary.pub");
/// The key that takes over when the primary key is rotated (SECURITY.md, "Key rotation").
const NEXT_KEY: &str = include_str!("../../keys/release-next.pub");

/// The public keys a release may be signed with.
pub(crate) struct TrustedKeys(pub(super) Vec<PublicKey>);

impl TrustedKeys {
    /// The primary and the next release key. Nothing else can change which keys are trusted.
    pub(crate) fn embedded() -> Self {
        Self(
            [PRIMARY_KEY, NEXT_KEY]
                .into_iter()
                .filter_map(|key| PublicKey::decode(key).ok())
                .collect(),
        )
    }

    /// The keys to use in this run: the embedded ones, except that a build with the test hooks
    /// trusts `JEV_TEST_UPDATE_KEY` instead, so tests can sign releases of their own. Release
    /// builds do not contain the hook.
    pub(crate) fn for_run(env: &Env) -> Self {
        #[cfg(feature = "internal-test-hooks")]
        if let Some(key) = env.get("JEV_TEST_UPDATE_KEY") {
            return Self(PublicKey::from_base64(key).into_iter().collect());
        }
        let _ = env;
        Self::embedded()
    }

    /// Checks that `signature` is a valid minisign signature of `data` by one of the keys, made
    /// for the file `file` of release `version`.
    ///
    /// # Errors
    ///
    /// What is wrong, for a person to read.
    pub(crate) fn verify(
        &self,
        data: &[u8],
        signature: &[u8],
        file: &str,
        version: &Version,
    ) -> Result<(), String> {
        let signature = std::str::from_utf8(signature)
            .ok()
            .and_then(|text| Signature::decode(text).ok())
            .ok_or_else(|| format!("{file}.minisig is not a minisign signature"))?;
        // Only prehashed signatures, which is what minisign makes: `allow_legacy` stays false.
        if !self
            .0
            .iter()
            .any(|key| key.verify(data, &signature, false).is_ok())
        {
            return Err(format!(
                "{file}.minisig is not a valid signature of {file} by a release key"
            ));
        }
        let expected = format!("file:{file}\tversion:{version}");
        if signature.trusted_comment() != expected {
            return Err(format!(
                "{file}.minisig was made for another file or release (its trusted comment is {:?})",
                signature.trusted_comment()
            ));
        }
        Ok(())
    }
}

/// The SHA-256 of `data`, in lowercase hex.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The checksum `SHA256SUMS` lists for `file`, from lines in `sha256sum`'s format:
/// `<hex>  <name>`, or `<hex> *<name>` for binary mode.
pub(crate) fn listed_checksum<'a>(sums: &'a str, file: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (checksum, name) = line.split_once(' ')?;
        let name = name.strip_prefix([' ', '*'])?.trim_end();
        (name == file && checksum.len() == 64).then_some(checksum)
    })
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{TrustedKeys, listed_checksum, sha256_hex};
    use crate::update::tests::Signer;

    #[test]
    fn both_committed_keys_are_trusted() {
        assert_eq!(TrustedKeys::embedded().0.len(), 2);
    }

    #[test]
    fn a_signature_must_come_from_a_trusted_key_and_name_its_file_and_version() {
        let signer = Signer::new();
        let other = Signer::new();
        let keys = signer.keys();
        let version = Version::new(0, 2, 0);
        let data = b"archive bytes";

        let good = signer.sign(data, "jev.tar.gz", &version);
        assert_eq!(keys.verify(data, &good, "jev.tar.gz", &version), Ok(()));

        let cases = [
            (
                keys.verify(b"other bytes", &good, "jev.tar.gz", &version),
                "not a valid signature",
            ),
            (
                keys.verify(
                    data,
                    &other.sign(data, "jev.tar.gz", &version),
                    "jev.tar.gz",
                    &version,
                ),
                "not a valid signature",
            ),
            (
                keys.verify(data, &good, "SHA256SUMS", &version),
                "another file or release",
            ),
            (
                keys.verify(data, &good, "jev.tar.gz", &Version::new(0, 3, 0)),
                "another file or release",
            ),
            (
                keys.verify(data, b"garbage", "jev.tar.gz", &version),
                "not a minisign signature",
            ),
        ];
        for (outcome, problem) in cases {
            let error = outcome.unwrap_err();
            assert!(error.contains(problem), "{error}");
        }
    }

    #[test]
    fn a_signature_made_by_the_minisign_cli_verifies() {
        // Made as scripts/release/sign.sh makes them, with a throwaway key (its secret half was
        // not kept): `minisign -S -m SHA256SUMS -t "file:SHA256SUMS<TAB>version:0.2.0"`.
        let key = "RWTr1ouNq3HoHbu1fctwO32Ve4dS9FlKN0cM7RjkVxj/BfiiA2S2pn2v";
        let signature = "untrusted comment: signature from minisign secret key\n\
RUTr1ouNq3HoHeiAM4Z0DDXnYyFmzEjCHkVHnpmLqAFmcs+UUgwaCRgqXUAeqZ9YP6xXw/Is6pdsmfX/DEgWpHxCYvYdRRtiEg8=\n\
trusted comment: file:SHA256SUMS\tversion:0.2.0\n\
wHZU6lTddA8KLmi0u3OD661HFtF/7Lkvyht//5fCR1uTjnKEddsp34XwAbL0c31xItXu6nY0nM7pY80gxPDMCQ==\n";
        let keys = TrustedKeys(vec![minisign_verify::PublicKey::from_base64(key).unwrap()]);

        assert_eq!(
            keys.verify(
                b"jev release fixture\n",
                signature.as_bytes(),
                "SHA256SUMS",
                &Version::new(0, 2, 0)
            ),
            Ok(())
        );
    }

    #[test]
    fn the_embedded_keys_do_not_trust_a_test_key() {
        let signer = Signer::new();
        let version = Version::new(0, 2, 0);
        let signature = signer.sign(b"data", "f", &version);

        assert!(
            TrustedKeys::embedded()
                .verify(b"data", &signature, "f", &version)
                .is_err()
        );
    }

    #[test]
    fn checksums_are_read_in_sha256sum_format() {
        let hash = sha256_hex(b"abc");
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let other = "0".repeat(64);
        let sums = format!("{other}  jev.zip\n{hash} *jev.tar.gz\nshort  x\n");

        assert_eq!(listed_checksum(&sums, "jev.tar.gz"), Some(hash.as_str()));
        assert_eq!(listed_checksum(&sums, "jev.zip"), Some(other.as_str()));
        assert_eq!(listed_checksum(&sums, "x"), None);
        assert_eq!(listed_checksum(&sums, "jev.tar"), None);
    }
}
