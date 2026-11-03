//! A release's update manifest (`supraman-update.json`), its ed25519 signature and the release keys
//! compiled in from `update.pub` (docs/RELEASE.md, "Signing").

use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, SecondsFormat, Utc};
pub use ed25519_dalek::{SigningKey, VerifyingKey};
use ed25519_dalek::{Signature, Signer as _};
pub use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_FILE: &str = "supraman-update.json";
pub const SUMS_FILE: &str = "SHA256SUMS";
/// The env var `sm release sign` reads the base64 secret key from.
pub const KEY_VAR: &str = "UPDATE_SIGNING_KEY";

const RELEASE_KEYS: &str = include_str!("update.pub");
const PLACEHOLDER_MARK: &str = "PLACEHOLDER";

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("invalid update manifest: {0}")]
    Manifest(String),
    #[error("the signature is not valid for this manifest and the release key")]
    BadSignature,
    #[error("invalid signature file: not the base64 of 64 bytes")]
    MalformedSignature,
    #[error("{0}")]
    Key(String),
    #[error("crates/supraman-core/src/update.pub is the placeholder: run scripts/new-signing-key.ps1 and commit its update.pub")]
    PlaceholderKey,
    #[error("{0}")]
    Zip(String),
}

pub type Result<T> = std::result::Result<T, UpdateError>;

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> UpdateError + '_ {
    move |source| UpdateError::Io { path: path.to_path_buf(), source }
}

/// `supraman-update.json`. What is signed is the file's bytes as written ([`Manifest::to_bytes`]), never a re-serialisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub commit: String,
    pub zip: String,
    pub sha256: String,
    pub size: u64,
    pub published: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_url: Option<String>,
}

impl Manifest {
    /// The manifest of the zip at `zip`, hashing it.
    pub fn for_zip(zip: &Path, version: &str, commit: &str, notes_url: Option<&str>, published: DateTime<Utc>) -> Result<Self> {
        let name = zip
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| UpdateError::Manifest(format!("{} has no file name", zip.display())))?;
        let (sha256, size) = sha256_file(zip)?;
        let manifest = Self {
            version: version.to_string(),
            commit: commit.to_string(),
            zip: name.to_string(),
            sha256,
            size,
            published: published.to_rfc3339_opts(SecondsFormat::Secs, true),
            notes_url: notes_url.map(str::to_string),
        };
        manifest.check()?;
        Ok(manifest)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).expect("a manifest serialises");
        bytes.push(b'\n');
        bytes
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|e| UpdateError::Manifest(e.to_string()))?;
        manifest.check()?;
        Ok(manifest)
    }

    pub fn semver(&self) -> semver::Version {
        semver::Version::parse(&self.version).expect("checked by parse")
    }

    fn check(&self) -> Result<()> {
        let bad = |why: String| Err(UpdateError::Manifest(why));
        if let Err(e) = semver::Version::parse(&self.version) {
            return bad(format!("version '{}' is not semver: {e}", self.version));
        }
        if !(7..=40).contains(&self.commit.len()) || !is_lower_hex(&self.commit) {
            return bad(format!("commit '{}' is not a git sha", self.commit));
        }
        if self.zip.is_empty() || self.zip.contains(['/', '\\', ':']) || self.zip.starts_with('.') || !self.zip.ends_with(".zip") {
            return bad(format!("zip '{}' is not a plain .zip file name", self.zip));
        }
        if self.sha256.len() != 64 || !is_lower_hex(&self.sha256) {
            return bad(format!("sha256 '{}' is not 64 lowercase hex digits", self.sha256));
        }
        if DateTime::parse_from_rfc3339(&self.published).is_err() {
            return bad(format!("published '{}' is not RFC 3339", self.published));
        }
        Ok(())
    }

    /// The zip's line in `SHA256SUMS` (the `sha256sum` format).
    pub fn sums_line(&self) -> String {
        format!("{}  {}\n", self.sha256, self.zip)
    }

    /// Refuses a file whose size or sha256 differs from the manifest's.
    pub fn check_zip(&self, path: &Path) -> Result<()> {
        let (sha256, size) = sha256_file(path)?;
        if size != self.size {
            return Err(UpdateError::Zip(format!("{}: {size} bytes, the manifest says {}", path.display(), self.size)));
        }
        if sha256 != self.sha256 {
            return Err(UpdateError::Zip(format!("{}: sha256 {sha256}, the manifest says {}", path.display(), self.sha256)));
        }
        Ok(())
    }
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut file = std::fs::File::open(path).map_err(io(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(io(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let hex = crate::hex::encode(&hasher.finalize());
    Ok((hex, size))
}

/// `<manifest>.sig`.
pub fn signature_path(manifest: &Path) -> PathBuf {
    let mut name = manifest.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

/// The signature file's contents: base64 of the 64-byte signature of `manifest_bytes`, one line.
pub fn sign(manifest_bytes: &[u8], secret_key: &SigningKey) -> Vec<u8> {
    let mut text = B64.encode(secret_key.sign(manifest_bytes).to_bytes()).into_bytes();
    text.push(b'\n');
    text
}

/// The manifest, when `signature_bytes` (a signature file) is one of `keys`' signature of `manifest_bytes` and the manifest is well formed.
pub fn verify(manifest_bytes: &[u8], signature_bytes: &[u8], keys: &[VerifyingKey]) -> Result<Manifest> {
    let text = std::str::from_utf8(signature_bytes).map_err(|_| UpdateError::MalformedSignature)?;
    let raw = B64.decode(text.trim()).map_err(|_| UpdateError::MalformedSignature)?;
    let raw: [u8; 64] = raw.try_into().map_err(|_| UpdateError::MalformedSignature)?;
    let signature = Signature::from_bytes(&raw);
    if !keys.iter().any(|k| k.verify_strict(manifest_bytes, &signature).is_ok()) {
        return Err(UpdateError::BadSignature);
    }
    Manifest::parse(manifest_bytes)
}

/// The release keys compiled in from `update.pub`: one base64 key per line, `#` lines ignored; during a rotation it holds both.
pub fn release_keys() -> Result<Vec<VerifyingKey>> {
    parse_public_keys(RELEASE_KEYS)
}

pub fn parse_public_keys(text: &str) -> Result<Vec<VerifyingKey>> {
    if text.contains(PLACEHOLDER_MARK) {
        return Err(UpdateError::PlaceholderKey);
    }
    let keys = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(parse_public_key)
        .collect::<Result<Vec<_>>>()?;
    if keys.is_empty() {
        return Err(UpdateError::Key("no public key in update.pub".into()));
    }
    Ok(keys)
}

pub fn parse_public_key(line: &str) -> Result<VerifyingKey> {
    let bad = || UpdateError::Key(format!("'{line}' is not the base64 of a 32-byte ed25519 public key"));
    let raw: [u8; 32] = B64.decode(line.trim()).map_err(|_| bad())?.try_into().map_err(|_| bad())?;
    VerifyingKey::from_bytes(&raw).map_err(|_| bad())
}

/// Never names the text or a byte of it in an error: it is a secret.
pub fn parse_secret_key(text: &str) -> Result<SigningKey> {
    let bad = || UpdateError::Key(format!("{KEY_VAR} is not the base64 of a 32-byte ed25519 secret key"));
    let raw = zeroize::Zeroizing::new(B64.decode(text.trim()).map_err(|_| bad())?);
    let seed: &[u8; 32] = raw.as_slice().try_into().map_err(|_| bad())?;
    Ok(SigningKey::from_bytes(seed))
}

pub fn generate_key() -> Result<SigningKey> {
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(|e| UpdateError::Key(format!("no random source: {e}")))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn encode_public_key(key: &VerifyingKey) -> String {
    B64.encode(key.to_bytes())
}

pub fn encode_secret_key(key: &SigningKey) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(B64.encode(key.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn zip(dir: &Path, body: &[u8]) -> PathBuf {
        let path = dir.join("Supraman-1.2.3.zip");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn manifest(dir: &Path) -> Manifest {
        let at = DateTime::parse_from_rfc3339("2026-11-02T10:00:00Z").unwrap().with_timezone(&Utc);
        Manifest::for_zip(&zip(dir, b"zip bytes"), "1.2.3", "d8ec160abcdef", Some("https://example.test/notes"), at).unwrap()
    }

    #[test]
    fn manifest_round_trips_through_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest(dir.path());
        assert_eq!(m.zip, "Supraman-1.2.3.zip");
        assert_eq!(m.size, 9);
        assert_eq!(m.published, "2026-11-02T10:00:00Z");
        assert_eq!(m.sha256, sha256_file(&dir.path().join("Supraman-1.2.3.zip")).unwrap().0);
        assert_eq!(Manifest::parse(&m.to_bytes()).unwrap(), m);
        assert_eq!(m.semver(), semver::Version::new(1, 2, 3));
        assert_eq!(m.sums_line(), format!("{}  Supraman-1.2.3.zip\n", m.sha256));
        let text = String::from_utf8(m.to_bytes()).unwrap();
        assert!(text.starts_with("{\n  \"version\": \"1.2.3\",\n  \"commit\""), "{text}");
    }

    #[test]
    fn a_bad_field_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let good = manifest(dir.path());
        for change in [
            (|m: &mut Manifest| m.version = "1.2".into()) as fn(&mut Manifest),
            |m| m.commit = "xyz".into(),
            |m| m.zip = "..\\evil.zip".into(),
            |m| m.sha256 = "AB".repeat(32),
            |m| m.published = "yesterday".into(),
        ] {
            let mut m = good.clone();
            change(&mut m);
            assert!(matches!(Manifest::parse(&m.to_bytes()), Err(UpdateError::Manifest(_))), "{m:?}");
        }
    }

    #[test]
    fn signed_with_the_key_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = manifest(dir.path()).to_bytes();
        let sig = sign(&bytes, &key(1));
        assert_eq!(verify(&bytes, &sig, &[key(1).verifying_key()]).unwrap(), Manifest::parse(&bytes).unwrap());
        assert!(verify(&bytes, &sig, &[key(2).verifying_key(), key(1).verifying_key()]).is_ok(), "any of the keys");
    }

    #[test]
    fn a_changed_byte_or_another_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = manifest(dir.path()).to_bytes();
        let sig = sign(&bytes, &key(1));
        let keys = [key(1).verifying_key()];

        let mut changed = bytes.clone();
        let at = changed.iter().position(|&b| b == b'3').unwrap();
        changed[at] = b'4';
        assert!(matches!(verify(&changed, &sig, &keys), Err(UpdateError::BadSignature)));

        let mut sha_changed = String::from_utf8(bytes.clone()).unwrap();
        let m = Manifest::parse(&bytes).unwrap();
        let flipped = if m.sha256.starts_with('0') { "1" } else { "0" };
        sha_changed = sha_changed.replacen(&m.sha256, &format!("{flipped}{}", &m.sha256[1..]), 1);
        assert!(matches!(verify(sha_changed.as_bytes(), &sig, &keys), Err(UpdateError::BadSignature)));

        let mut raw = B64.decode(String::from_utf8(sig.clone()).unwrap().trim()).unwrap();
        raw[10] ^= 1;
        let bad_sig = B64.encode(raw).into_bytes();
        assert!(matches!(verify(&bytes, &bad_sig, &keys), Err(UpdateError::BadSignature)));

        assert!(matches!(verify(&bytes, &sign(&bytes, &key(2)), &keys), Err(UpdateError::BadSignature)));
        assert!(matches!(verify(&bytes, b"not base64!", &keys), Err(UpdateError::MalformedSignature)));
        assert!(matches!(verify(&bytes, b"AAAA", &keys), Err(UpdateError::MalformedSignature)));
    }

    #[test]
    fn a_changed_zip_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest(dir.path());
        let path = dir.path().join(&m.zip);
        m.check_zip(&path).unwrap();
        std::fs::write(&path, b"zip bytez").unwrap();
        assert!(matches!(m.check_zip(&path), Err(UpdateError::Zip(e)) if e.contains("sha256")));
        std::fs::write(&path, b"zip").unwrap();
        assert!(matches!(m.check_zip(&path), Err(UpdateError::Zip(e)) if e.contains("3 bytes")));
    }

    #[test]
    fn keys_encode_and_parse() {
        let k = generate_key().unwrap();
        let public = encode_public_key(&k.verifying_key());
        assert_eq!(parse_public_keys(&format!("# comment\n{public}\n\n")).unwrap(), vec![k.verifying_key()]);
        assert_eq!(parse_secret_key(&format!("{}\r\n", *encode_secret_key(&k))).unwrap().to_bytes(), k.to_bytes());
        assert_ne!(generate_key().unwrap().to_bytes(), k.to_bytes());
    }

    #[test]
    fn a_bad_secret_key_is_not_echoed() {
        let err = parse_secret_key("c2VjcmV0LXZhbHVl").unwrap_err().to_string();
        assert!(!err.contains("c2VjcmV0"), "{err}");
        let err = parse_secret_key("se-cret!").unwrap_err().to_string();
        assert!(!err.contains("se-cret") && !err.contains("offset"), "{err}");
    }

    #[test]
    fn the_placeholder_and_an_empty_file_are_no_keys() {
        assert!(matches!(parse_public_keys("# PLACEHOLDER: not a key\n"), Err(UpdateError::PlaceholderKey)));
        assert!(matches!(parse_public_keys("# nothing\n"), Err(UpdateError::Key(_))));
        assert!(matches!(parse_public_keys("abc\n"), Err(UpdateError::Key(_))));
    }
}
