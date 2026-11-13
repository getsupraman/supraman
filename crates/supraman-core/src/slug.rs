use crate::{CoreError, Result};

/// What every Supraman session name starts with: `fm-<folder key>-…`.
pub const FM_PREFIX: &str = "fm-";

/// `"Nomos.Extract"` → `"nomos-extract"`. Runs of other characters collapse to one `-`.
pub fn slugify(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// A typed character as a [`validate_name`] character, lowercased; `None` for one a name cannot hold.
pub fn name_char(c: char) -> Option<char> {
    let c = c.to_ascii_lowercase();
    is_name_char(c).then_some(c)
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// A session name that is safe as part of a file name: ASCII letters, digits, `-`, `_` and `.`, not starting with `.`.
pub fn is_session_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A session id that becomes a file name: UUIDs in practice, so letters, digits, `-` and `_` only.
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty() && !name.starts_with('-') && !name.ends_with('-') && name.chars().all(is_name_char);
    if valid {
        Ok(())
    } else {
        Err(CoreError::InvalidName(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_folder_names() {
        assert_eq!(slugify("Nomos"), "nomos");
        assert_eq!(slugify("Nomos.Extract"), "nomos-extract");
        assert_eq!(slugify("  My  Repo__2 "), "my-repo-2");
        assert_eq!(slugify("..."), "");
    }

    #[test]
    fn validate_rejects_non_slugs() {
        assert!(validate_name("nomos-2").is_ok());
        for bad in ["", "Nomos", "-a", "a-", "a.b", "a b", "ä"] {
            assert!(
                matches!(validate_name(bad), Err(CoreError::InvalidName(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn name_char_lowers_what_a_name_holds_and_refuses_the_rest() {
        assert_eq!(name_char('M'), Some('m'));
        assert_eq!(name_char('7'), Some('7'));
        assert_eq!(name_char('-'), Some('-'));
        for bad in [' ', '_', '.', 'ä', 'Ä'] {
            assert_eq!(name_char(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn session_names_and_ids_hold_only_file_name_characters() {
        for good in ["fm-demo-beta", "hand-Alpha-1", "a_b.c"] {
            assert!(is_session_name(good), "{good:?}");
        }
        for bad in ["", "..", ".x", "../x", r"a\b", "a b", "a/b", "ä"] {
            assert!(!is_session_name(bad), "{bad:?}");
        }
        assert!(is_safe_id("0b9d6c1e-2f4a-4c7e-9a51-3d2b8e7f6a10"));
        assert!(is_safe_id("fm_role-1"));
        for bad in ["", "a.b", "../x", "a b", "ä"] {
            assert!(!is_safe_id(bad), "{bad:?}");
        }
        assert!(is_safe_id(&"a".repeat(128)) && !is_safe_id(&"a".repeat(129)));
    }
}
