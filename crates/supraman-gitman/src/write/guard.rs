use std::path::Path;

use supraman_core::paths::within;

/// The names of the live Supraman sessions whose cwd is the worktree or below it.
pub trait Occupancy: Send + Sync {
    fn who_works_in(&self, worktree: &Path) -> Vec<String>;
}

impl<F: Fn(&Path) -> Vec<String> + Send + Sync> Occupancy for F {
    fn who_works_in(&self, worktree: &Path) -> Vec<String> {
        self(worktree)
    }
}

/// The names among `sessions` (name, cwd) whose cwd is `worktree` or below it.
pub fn working_in<'a>(sessions: impl IntoIterator<Item = (&'a str, &'a Path)>, worktree: &Path) -> Vec<String> {
    sessions.into_iter().filter(|(_, cwd)| within(cwd, worktree)).map(|(name, _)| name.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_below_the_worktree_works_in_it_and_a_sibling_does_not() {
        let sessions = [("fm-in", Path::new("C:/w/repo/src")), ("fm-top", Path::new("C:/w/repo")), ("fm-sibling", Path::new("C:/w/repo-2"))];
        assert_eq!(working_in(sessions, Path::new("C:/w/repo")), ["fm-in", "fm-top"]);
    }
}
