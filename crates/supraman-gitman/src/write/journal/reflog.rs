//! A ref's reflog, oldest first: a files repo's `logs/` file, else git's own walk of it.

use super::walk::{Line, parse_log};
use crate::parse::text;
use crate::write::{Call, Session, WriteError};

/// `name`'s reflog (`HEAD` and `refs/worktree/…` this checkout's own, any other ref the common one's); none when it has none.
pub(super) fn lines(s: &Session, name: &str) -> Result<Vec<Line>, WriteError> {
    let dirs = s.writer().dirs();
    if dirs.reftable() {
        return walked(s, name);
    }
    let own = name == "HEAD" || name.starts_with("refs/worktree/");
    let path = if own { &dirs.git_dir } else { &dirs.common_dir }.join("logs").join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(parse_log(&text(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(WriteError::io("read", &path)(source)),
    }
}

/// git gives no old id: each line's is the line before's new id, the oldest line's zeros.
fn walked(s: &Session, name: &str) -> Result<Vec<Line>, WriteError> {
    let out = s.output(&Call::new(&["log", "--ignore-missing", "-g", "--date=raw", "--format=%H%x00%gD%x00%gs", name, "--"]))?;
    let mut lines: Vec<Line> = text(&out).lines().rev().filter_map(walked_line).collect();
    let mut older: Option<String> = None;
    for line in &mut lines {
        line.old = older.replace(line.new.clone()).unwrap_or_else(|| "0".repeat(line.new.len()));
    }
    Ok(lines)
}

/// `<new>\0<ref>@{<time> <zone>}\0<message>`, its old id still empty.
fn walked_line(line: &str) -> Option<Line> {
    let mut fields = line.splitn(3, '\0');
    let (new, selector, message) = (fields.next()?, fields.next()?, fields.next()?);
    let time = selector.rsplit_once("@{")?.1.split(' ').next()?.parse().ok()?;
    Some(Line { old: String::new(), new: new.to_string(), time, message: message.to_string() })
}

#[cfg(test)]
mod tests;
