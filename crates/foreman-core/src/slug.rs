use crate::{CoreError, Result};

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

pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
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
}
