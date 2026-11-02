use std::io::{IsTerminal as _, Write};
use std::path::{Path, PathBuf};

use supraman_core::update::{self, KEY_VAR, MANIFEST_FILE, Manifest, SUMS_FILE, UpdateError, VerifyingKey};

use crate::{Failure, value};

pub(crate) fn release(args: &[&str]) -> Result<(), Failure> {
    match args {
        ["manifest", rest @ ..] => {
            let (manifest, sums) = manifest_cmd(rest, chrono::Utc::now())?;
            println!("manifest {}", manifest.display());
            println!("sums {}", sums.display());
            Ok(())
        }
        ["sign", path] => {
            let secret = std::env::var(KEY_VAR).ok();
            let sig = sign_manifest(Path::new(path), secret.as_deref(), update::release_keys())?;
            println!("signed {}", sig.display());
            Ok(())
        }
        ["verify", path, rest @ ..] => {
            let keys = match rest {
                [] => update::release_keys(),
                ["--public-key", file] => std::fs::read_to_string(file)
                    .map_err(|e| Failure::Refused(format!("error: {file}: {e}")))
                    .map(|text| update::parse_public_keys(&text))?,
                _ => return Err(Failure::Usage(format!("unexpected arguments: {}", rest.join(" ")))),
            };
            let m = verify_manifest(Path::new(path), keys)?;
            println!("verified {} {} sha256 {}", m.version, m.zip, m.sha256);
            Ok(())
        }
        ["keygen", "--public-out", file] => {
            let stdout = std::io::stdout();
            let console = stdout.is_terminal();
            keygen(Path::new(file), console, &mut stdout.lock())?;
            eprintln!("wrote the public key to {file}; the secret key went to stdout");
            Ok(())
        }
        _ => Err(Failure::Usage(format!("unrecognised arguments: release {}", args.join(" ")))),
    }
}

fn refused(e: UpdateError) -> Failure {
    Failure::Refused(format!("error: {e}"))
}

/// Writes `supraman-update.json` and `SHA256SUMS` beside the zip.
fn manifest_cmd(args: &[&str], now: chrono::DateTime<chrono::Utc>) -> Result<(PathBuf, PathBuf), Failure> {
    let (mut zip, mut version, mut commit, mut notes) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--version" => version = Some(value(&mut it, arg)?),
            "--commit" => commit = Some(value(&mut it, arg)?),
            "--notes-url" => notes = Some(value(&mut it, arg)?),
            flag if flag.starts_with("--") => return Err(Failure::Usage(format!("unknown option {flag}"))),
            path if zip.is_none() => zip = Some(PathBuf::from(path)),
            extra => return Err(Failure::Usage(format!("unexpected argument '{extra}'"))),
        }
    }
    let zip = zip.ok_or_else(|| Failure::Usage("release manifest needs a <zip>".into()))?;
    let version = version.ok_or_else(|| Failure::Usage("release manifest needs --version".into()))?;
    let commit = commit.ok_or_else(|| Failure::Usage("release manifest needs --commit".into()))?;
    let expected = format!("Supraman-{version}.zip");
    if zip.file_name().and_then(|n| n.to_str()) != Some(expected.as_str()) {
        return Err(Failure::Refused(format!("error: {} is not {expected}", zip.display())));
    }
    let manifest = Manifest::for_zip(&zip, version, commit, notes, now).map_err(refused)?;
    let dir = zip.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let manifest_path = dir.join(MANIFEST_FILE);
    let sums_path = dir.join(SUMS_FILE);
    let write = |path: &Path, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| Failure::Refused(format!("error: {}: {e}", path.display())));
    write(&manifest_path, &manifest.to_bytes())?;
    write(&sums_path, manifest.sums_line().as_bytes())?;
    let _ = std::fs::remove_file(update::signature_path(&manifest_path));
    Ok((manifest_path, sums_path))
}

/// Signs only with a secret key whose public half is a release key, so a signed manifest always verifies in the app.
fn sign_manifest(path: &Path, secret: Option<&str>, keys: update::Result<Vec<VerifyingKey>>) -> Result<PathBuf, Failure> {
    let secret = secret
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Failure::Refused(format!("error: {KEY_VAR} is not set")))?;
    let keys = keys.map_err(refused)?;
    let key = update::parse_secret_key(secret).map_err(refused)?;
    if !keys.contains(&key.verifying_key()) {
        return Err(Failure::Refused(format!(
            "error: {KEY_VAR} is not the secret half of a key in crates/supraman-core/src/update.pub"
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| Failure::Refused(format!("error: {}: {e}", path.display())))?;
    Manifest::parse(&bytes).map_err(refused)?;
    let sig = update::signature_path(path);
    std::fs::write(&sig, update::sign(&bytes, &key)).map_err(|e| Failure::Refused(format!("error: {}: {e}", sig.display())))?;
    Ok(sig)
}

/// The manifest's signature against `keys`, then the zip beside it against the manifest.
fn verify_manifest(path: &Path, keys: update::Result<Vec<VerifyingKey>>) -> Result<Manifest, Failure> {
    let keys = keys.map_err(refused)?;
    let read = |p: &Path| std::fs::read(p).map_err(|e| Failure::Refused(format!("error: {}: {e}", p.display())));
    let bytes = read(path)?;
    let sig = read(&update::signature_path(path))?;
    let manifest = update::verify(&bytes, &sig, &keys).map_err(refused)?;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    manifest.check_zip(&dir.join(&manifest.zip)).map_err(refused)?;
    Ok(manifest)
}

/// The secret key goes to `out` only, and never to a console.
fn keygen(public_out: &Path, console: bool, out: &mut dyn Write) -> Result<(), Failure> {
    if console {
        return Err(Failure::Refused(
            "error: stdout is a console: pipe the secret key on, e.g. `sm release keygen --public-out <file> | gh secret set UPDATE_SIGNING_KEY`".into(),
        ));
    }
    let key = update::generate_key().map_err(refused)?;
    let public = format!("{}\n", update::encode_public_key(&key.verifying_key()));
    std::fs::write(public_out, public).map_err(|e| Failure::Refused(format!("error: {}: {e}", public_out.display())))?;
    let secret = update::encode_secret_key(&key);
    out.write_all(secret.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| Failure::Refused(format!("error: cannot write the secret key to stdout: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use supraman_core::update::SigningKey;

    use super::*;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-11-02T10:00:00Z").unwrap().with_timezone(&chrono::Utc)
    }

    fn secret(key: &SigningKey) -> String {
        update::encode_secret_key(key).to_string()
    }

    fn released(dir: &Path) -> PathBuf {
        let zip = dir.join("Supraman-0.1.0.zip");
        std::fs::write(&zip, b"the zip").unwrap();
        let zip_arg = zip.to_str().unwrap();
        manifest_cmd(&[zip_arg, "--version", "0.1.0", "--commit", "d8ec160abc", "--notes-url", "https://x.test/n"], now())
            .unwrap_or_else(|_| panic!("manifest"))
            .0
    }

    fn refusal(r: Result<impl std::fmt::Debug, Failure>) -> String {
        match r {
            Err(Failure::Refused(line)) => line,
            Err(_) => panic!("not a refusal"),
            Ok(v) => panic!("not refused: {v:?}"),
        }
    }

    #[test]
    fn manifest_writes_the_manifest_and_sums_beside_the_zip() {
        let dir = tempfile::tempdir().unwrap();
        let path = released(dir.path());
        assert_eq!(path, dir.path().join(MANIFEST_FILE));
        let m = Manifest::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((m.version.as_str(), m.zip.as_str(), m.size), ("0.1.0", "Supraman-0.1.0.zip", 7));
        assert_eq!(m.notes_url.as_deref(), Some("https://x.test/n"));
        assert_eq!(std::fs::read_to_string(dir.path().join(SUMS_FILE)).unwrap(), m.sums_line());
    }

    #[test]
    fn manifest_refuses_a_zip_named_for_another_version() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("Supraman-0.2.0.zip");
        std::fs::write(&zip, b"z").unwrap();
        let line = refusal(manifest_cmd(&[zip.to_str().unwrap(), "--version", "0.1.0", "--commit", "abcdef1"], now()));
        assert!(line.contains("is not Supraman-0.1.0.zip"), "{line}");
    }

    #[test]
    fn signed_with_a_release_key_verifies_and_a_change_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = released(dir.path());
        let key = update::generate_key().unwrap();
        let keys = || Ok(vec![key.verifying_key()]);
        let sig = sign_manifest(&path, Some(&secret(&key)), keys()).unwrap_or_else(|_| panic!("sign"));
        assert_eq!(sig, dir.path().join("supraman-update.json.sig"));
        assert_eq!(verify_manifest(&path, keys()).unwrap_or_else(|_| panic!("verify")).version, "0.1.0");

        let other = update::generate_key().unwrap();
        assert!(refusal(verify_manifest(&path, Ok(vec![other.verifying_key()]))).contains("signature is not valid"));

        std::fs::write(dir.path().join("Supraman-0.1.0.zip"), b"the zib").unwrap();
        assert!(refusal(verify_manifest(&path, keys())).contains("sha256"));

        std::fs::write(&sig, b"").unwrap();
        assert!(refusal(verify_manifest(&path, keys())).contains("invalid signature file"));
    }

    #[test]
    fn sign_refuses_without_the_key_or_with_a_key_not_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = released(dir.path());
        let key = update::generate_key().unwrap();
        let keys = || Ok(vec![key.verifying_key()]);
        assert!(refusal(sign_manifest(&path, None, keys())).contains("UPDATE_SIGNING_KEY is not set"));
        assert!(refusal(sign_manifest(&path, Some("  "), keys())).contains("is not set"));

        let other = secret(&update::generate_key().unwrap());
        let line = refusal(sign_manifest(&path, Some(&other), keys()));
        assert!(line.contains("not the secret half") && !line.contains(&other), "{line}");

        let line = refusal(sign_manifest(&path, Some(&secret(&key)), Err(UpdateError::PlaceholderKey)));
        assert!(line.contains("placeholder"), "{line}");
        assert!(!update::signature_path(&path).exists());
    }

    #[test]
    fn the_committed_update_pub_is_a_key_or_the_placeholder() {
        match update::release_keys() {
            Ok(keys) => assert!(!keys.is_empty()),
            Err(e) => assert!(matches!(e, UpdateError::PlaceholderKey), "{e}"),
        }
    }

    #[test]
    fn keygen_refuses_a_console_and_writes_a_matching_pair() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("update.pub");
        let mut out = Vec::new();
        assert!(refusal(keygen(&public, true, &mut out)).contains("stdout is a console"));
        assert!(out.is_empty() && !public.exists());

        keygen(&public, false, &mut out).unwrap_or_else(|_| panic!("keygen"));
        let key = update::parse_secret_key(std::str::from_utf8(&out).unwrap()).unwrap();
        let keys = update::parse_public_keys(&std::fs::read_to_string(&public).unwrap()).unwrap();
        assert_eq!(keys, vec![key.verifying_key()]);
    }
}
