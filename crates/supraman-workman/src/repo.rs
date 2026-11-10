//! A repo's identity as Workman keys it (`RepoKeys` in `Workman.Core/Domain/Repos.cs`): a normalised remote or root binds it.

use crate::model::ProjectRepo;

pub const REMOTE: &str = "remote";
pub const ROOT: &str = "root";

/// `git@github.com:Acme/workman.git` and `https://github.com/Acme/workman` are both `github.com/acme/workman`.
pub fn normalise_remote(remote: &str) -> Option<String> {
    let value = remote.trim().replace('\\', "/");
    if value.is_empty() {
        return None;
    }
    let (schemed, rest) = match scheme_end(&value) {
        Some(end) => (true, &value[end..]),
        None => (false, value.as_str()),
    };
    let head = rest.split('/').next().unwrap_or(rest);
    let mut value = match head.rfind('@') {
        Some(at) => rest[at + 1..].to_string(),
        None => rest.to_string(),
    };
    if !schemed && let Some(colon) = value.find(':') {
        let host = &value[..colon];
        if host.chars().count() >= 2 && !host.contains('/') {
            let path = value[colon + 1..].trim_start_matches('/').to_string();
            value = format!("{host}/{path}");
        }
    }
    let value = strip_tail(&value, ".git");
    (!value.is_empty()).then(|| value.to_lowercase())
}

/// The main checkout's root: forward slashes, no trailing `/` or `/.git`, a Windows path lowercased whole.
pub fn normalise_root(root: &str) -> Option<String> {
    let value = root.trim().replace('\\', "/");
    let value = strip_tail(&value, "/.git");
    let windows = value.starts_with("//") || {
        let b = value.as_bytes();
        b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'/')
    };
    let value = if windows { value.to_lowercase() } else { value };
    (!value.is_empty()).then_some(value)
}

pub fn keys(remote: Option<&str>, root: Option<&str>) -> Vec<(&'static str, String)> {
    let remote = remote.and_then(normalise_remote).map(|k| (REMOTE, k));
    let root = root.and_then(normalise_root).map(|k| (ROOT, k));
    remote.into_iter().chain(root).collect()
}

pub fn binds(repos: &[ProjectRepo], keys: &[(&str, String)]) -> bool {
    repos.iter().any(|r| keys.iter().any(|(kind, key)| r.kind == *kind && &r.key == key))
}

/// Where `^[a-z][a-z0-9+.\-]*://` ends, case-insensitive.
fn scheme_end(value: &str) -> Option<usize> {
    let sep = value.find("://")?;
    let scheme = &value[..sep];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))).then_some(sep + 3)
}

/// `value` without its trailing `/*(<tail>)?/*`, `tail` matched case-insensitively.
fn strip_tail(value: &str, tail: &str) -> String {
    let value = value.trim_end_matches('/');
    let cut = value.len().checked_sub(tail.len()).filter(|&at| value.is_char_boundary(at) && value[at..].eq_ignore_ascii_case(tail));
    let value = cut.map_or(value, |at| &value[..at]);
    value.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Workman's `docs/repo-keys.json`, the table both of its implementations run.
    #[test]
    fn the_keys_follow_workmans_table() {
        let remote = [
            ("git@github.com:Acme/workman.git", Some("github.com/acme/workman")),
            ("https://github.com/Acme/workman", Some("github.com/acme/workman")),
            ("https://github.com/Acme/workman.git/", Some("github.com/acme/workman")),
            ("https://user@github.com/Acme/workman/", Some("github.com/acme/workman")),
            ("ssh://git@github.com/Acme/workman.git", Some("github.com/acme/workman")),
            ("github.com:Acme/workman", Some("github.com/acme/workman")),
            ("  https://GitHub.com/Acme/Workman.git  ", Some("github.com/acme/workman")),
            ("git@github.com:/Acme/workman.git", Some("github.com/acme/workman")),
            ("https://dev.azure.com/org/Proj/_git/Repo", Some("dev.azure.com/org/proj/_git/repo")),
            ("ssh://git@host.example:2222/team/app.git", Some("host.example:2222/team/app")),
            ("C:/repos/local.git", Some("c:/repos/local")),
            ("github.com/acme/workman", Some("github.com/acme/workman")),
            ("", None),
            ("   ", None),
        ];
        for (given, want) in remote {
            assert_eq!(normalise_remote(given).as_deref(), want, "remote {given:?}");
        }
        let root = [
            (r"P:\Projects\Acme\Skills", Some("p:/projects/acme/skills")),
            ("P:/Projects/Acme/Skills/", Some("p:/projects/acme/skills")),
            ("p:/projects/acme/skills/.git", Some("p:/projects/acme/skills")),
            ("P:/Projects/Acme/Skills/.git/", Some("p:/projects/acme/skills")),
            ("P:/Projects/Acme/AutoWork", Some("p:/projects/acme/autowork")),
            ("/home/dev/src/Skills", Some("/home/dev/src/Skills")),
            ("/home/dev/src/Skills/", Some("/home/dev/src/Skills")),
            ("/home/dev/src/Skills/.git", Some("/home/dev/src/Skills")),
            (r"\\Server\Share\Repo", Some("//server/share/repo")),
            ("p:/projects/acme/skills", Some("p:/projects/acme/skills")),
            ("", None),
        ];
        for (given, want) in root {
            assert_eq!(normalise_root(given).as_deref(), want, "root {given:?}");
        }
    }

    #[test]
    fn either_key_binds() {
        let repo = |kind: &str, key: &str| ProjectRepo { kind: kind.into(), key: key.into() };
        let bound = [repo(REMOTE, "github.com/acme/a"), repo(ROOT, "p:/x/b")];
        assert!(binds(&bound, &keys(Some("git@github.com:Acme/a.git"), None)));
        assert!(binds(&bound, &keys(None, Some(r"P:\X\B"))));
        assert!(!binds(&bound, &keys(Some("https://github.com/acme/b"), Some("p:/x/a"))));
        assert!(!binds(&[repo(ROOT, "github.com/acme/a")], &keys(Some("github.com/acme/a"), None)), "the kinds must agree");
    }
}
