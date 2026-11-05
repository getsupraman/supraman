use crate::parse::{text, unquote};
use crate::{GitError, Repo};

const PATCH: &[&str] = &["--patch", "-M", "--no-ext-diff", "--no-textconv", "--src-prefix=a/", "--dst-prefix=b/"];

/// What a patch compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffOf<'a> {
    /// A commit against its first parent, a root commit against the empty tree.
    Commit(&'a str),
    Revs(&'a str, &'a str),
    RevToWorktree(&'a str),
    RevToIndex(&'a str),
    IndexToWorktree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Without its prefix and its `\n`.
    pub text: String,
    /// `\ No newline at end of file` follows it.
    pub no_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// The function or heading git printed after the second `@@`.
    pub section: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilePatch {
    /// `None` for an added file.
    pub old_path: Option<String>,
    /// `None` for a deleted file.
    pub new_path: Option<String>,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

/// The patch of `paths` (a rename needs both of its paths); every changed file when `paths` is empty.
pub fn patch(repo: &Repo, of: DiffOf<'_>, paths: &[&str]) -> Result<Vec<FilePatch>, GitError> {
    // Plumbing only: porcelain `git diff` refreshes the index and writes it despite `--no-optional-locks` (2.44).
    let mut args: Vec<&str> = match of {
        DiffOf::Commit(_) => vec!["diff-tree", "-r", "--root", "--diff-merges=first-parent", "--no-commit-id"],
        DiffOf::Revs(..) => vec!["diff-tree", "-r"],
        DiffOf::RevToWorktree(_) => vec!["diff-index"],
        DiffOf::RevToIndex(_) => vec!["diff-index", "--cached"],
        DiffOf::IndexToWorktree => vec!["diff-files"],
    };
    args.extend(PATCH);
    args.push("--end-of-options");
    match of {
        DiffOf::Commit(rev) | DiffOf::RevToWorktree(rev) | DiffOf::RevToIndex(rev) => args.push(rev),
        DiffOf::Revs(from, to) => args.extend([from, to]),
        DiffOf::IndexToWorktree => {}
    }
    args.push("--");
    args.extend(paths);
    repo.read(&args, None, |out| parse_patch(out))
}

fn parse_patch(out: &[u8]) -> Result<Vec<FilePatch>, String> {
    let mut files: Vec<FilePatch> = Vec::new();
    let mut in_hunk = false;
    for raw in out.split(|b| *b == b'\n') {
        let line = text(raw);
        if let Some(header) = line.strip_prefix("diff --git ") {
            files.push(header_paths(header));
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if line.starts_with("@@ ") {
            file.hunks.push(hunk_header(&line)?);
            in_hunk = true;
        } else if in_hunk {
            hunk_line(file, &line);
        } else {
            file_header(file, &line);
        }
    }
    Ok(files)
}

fn hunk_line(file: &mut FilePatch, line: &str) {
    let Some(hunk) = file.hunks.last_mut() else { return };
    let kind = match line.as_bytes().first() {
        Some(b' ') => LineKind::Context,
        Some(b'+') => LineKind::Added,
        Some(b'-') => LineKind::Removed,
        Some(b'\\') => {
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            return;
        }
        _ => return,
    };
    hunk.lines.push(Line { kind, text: line[1..].to_string(), no_newline: false });
}

fn file_header(file: &mut FilePatch, line: &str) {
    let path = |p: &str, prefix: &str| {
        let p = unquote(p.trim_end_matches('\t'));
        (p != "/dev/null").then(|| p.strip_prefix(prefix).map(String::from).unwrap_or(p))
    };
    if let Some(p) = line.strip_prefix("--- ") {
        file.old_path = path(p, "a/");
    } else if let Some(p) = line.strip_prefix("+++ ") {
        file.new_path = path(p, "b/");
    } else if let Some(p) = line.strip_prefix("rename from ").or_else(|| line.strip_prefix("copy from ")) {
        file.old_path = Some(unquote(p));
    } else if let Some(p) = line.strip_prefix("rename to ").or_else(|| line.strip_prefix("copy to ")) {
        file.new_path = Some(unquote(p));
    } else if line.starts_with("new file mode ") {
        file.old_path = None;
    } else if line.starts_with("deleted file mode ") {
        file.new_path = None;
    } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
        file.binary = true;
    }
}

/// `a/<p> b/<p>`, each maybe C-quoted; for an unquoted pair with spaces, the split that gives one path twice.
fn header_paths(header: &str) -> FilePatch {
    let (old, new) = if header.starts_with('"') || header.ends_with('"') {
        quoted_pair(header)
    } else {
        let half = header.len().saturating_sub(1) / 2;
        match header.get(..half).zip(header.get(half + 1..)) {
            Some((a, b)) if a.strip_prefix("a/").is_some() && a.get(2..) == b.get(2..) => (a.to_string(), b.to_string()),
            _ => header.split_once(" b/").map(|(a, b)| (a.to_string(), format!("b/{b}"))).unwrap_or_default(),
        }
    };
    let strip = |p: String, prefix: &str| Some(p.strip_prefix(prefix).map(String::from).unwrap_or(p));
    FilePatch { old_path: strip(old, "a/"), new_path: strip(new, "b/"), ..FilePatch::default() }
}

fn quoted_pair(header: &str) -> (String, String) {
    let bytes = header.as_bytes();
    let end = if bytes.first() == Some(&b'"') {
        let mut i = 1;
        while i < bytes.len() && bytes[i] != b'"' {
            i += if bytes[i] == b'\\' { 2 } else { 1 };
        }
        (i + 1).min(bytes.len())
    } else {
        header.find(' ').unwrap_or(header.len())
    };
    (unquote(&header[..end]), unquote(header[end..].trim_start()))
}

/// `@@ -a[,b] +c[,d] @@[ section]`; a missing count is 1.
fn hunk_header(line: &str) -> Result<Hunk, String> {
    let bad = || format!("hunk header {line:?}");
    let rest = line.strip_prefix("@@ -").ok_or_else(bad)?;
    let (ranges, section) = rest.split_once(" @@").ok_or_else(bad)?;
    let (old, new) = ranges.split_once(" +").ok_or_else(bad)?;
    let range = |r: &str| -> Result<(u32, u32), String> {
        let (start, count) = r.split_once(',').unwrap_or((r, "1"));
        Ok((start.parse().map_err(|_| bad())?, count.parse().map_err(|_| bad())?))
    };
    let ((old_start, old_lines), (new_start, new_lines)) = (range(old)?, range(new)?);
    Ok(Hunk { old_start, old_lines, new_start, new_lines, section: section.trim_start().to_string(), lines: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modification_a_rename_and_a_binary() {
        let out = b"diff --git a/a b.txt b/a b.txt\nindex 1..2 100644\n--- a/a b.txt\t\n+++ b/a b.txt\t\n@@ -1 +1,2 @@ fn x\n-old\n+new\n+-- not a header\n\\ No newline at end of file\n\
diff --git a/old b/new\nsimilarity index 90%\nrename from old\nrename to new\n\
diff --git a/bin b/bin\nnew file mode 100644\nBinary files /dev/null and b/bin differ\n";
        let files = parse_patch(out).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!((files[0].old_path.as_deref(), files[0].new_path.as_deref()), (Some("a b.txt"), Some("a b.txt")));
        let hunk = &files[0].hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines, hunk.section.as_str()), (1, 1, 1, 2, "fn x"));
        assert_eq!(hunk.lines.iter().map(|l| l.kind).collect::<Vec<_>>(), [LineKind::Removed, LineKind::Added, LineKind::Added]);
        assert!(hunk.lines[2].no_newline && hunk.lines[2].text == "-- not a header");
        assert_eq!((files[1].old_path.as_deref(), files[1].new_path.as_deref()), (Some("old"), Some("new")));
        assert!(files[2].binary && files[2].old_path.is_none());
        assert_eq!(header_paths(r#""a/t\"x" "b/t\"x""#).new_path.as_deref(), Some("t\"x"));
    }
}
