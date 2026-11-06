/// A warm start (a saved cache with files) shows the cached rows, stale or not, without waiting for the scan; a cold one waits.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptsLoad {
    /// Whether the saved cache held files; `None` until it is read.
    cache: Option<bool>,
    read: bool,
    progress: Option<(usize, usize)>,
    scans: u64,
    /// Each continued closed session to the newest session of its chain ([`supraman_claude::chains`]).
    pub continued: std::collections::HashMap<String, String>,
}

impl TranscriptsLoad {
    pub fn new(has_claude_dir: bool) -> Self {
        Self { cache: None, read: !has_claude_dir, progress: None, scans: 0, continued: Default::default() }
    }

    /// Whether the cached rows are listed before the scan.
    pub fn opened(&mut self, has_cache: bool) -> bool {
        self.cache = Some(has_cache);
        has_cache
    }

    pub fn listed_cached(&mut self) {
        self.read = true;
    }

    /// `(read, of)` of the files the running scan reads.
    pub fn reading(&mut self, at: (usize, usize)) {
        self.progress = Some(at);
    }

    /// Whether it was the first scan.
    pub fn scanned(&mut self) -> bool {
        self.read = true;
        self.progress = None;
        self.scans += 1;
        self.scans == 1
    }

    pub fn read(&self) -> bool {
        self.read
    }

    /// What the loading line names while the start waits for the transcripts.
    pub fn pending(&self) -> Option<String> {
        (!self.read && self.cache != Some(true)).then(|| match self.progress {
            Some((read, of)) => format!("transcripts {read}/{of}"),
            None => "transcripts".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_warm_start_shows_the_cached_rows_without_the_loading_line_while_the_scan_runs() {
        let mut t = TranscriptsLoad::new(true);
        assert_eq!(t.pending().as_deref(), Some("transcripts"), "until the saved cache is read");
        assert!(t.opened(true), "a cache with files lists its rows before the scan");
        assert_eq!(t.pending(), None, "the start does not wait for the scan");
        t.listed_cached();
        assert!(t.read());
        t.reading((3, 10));
        assert_eq!(t.pending(), None, "the scan renewing the cached rows names nothing");
        assert!(t.scanned(), "the first scan");
        assert!(!t.scanned());
        assert_eq!(t.pending(), None);
    }

    #[test]
    fn a_cold_start_keeps_the_loading_line_until_the_scan_is_applied() {
        let mut t = TranscriptsLoad::new(true);
        assert!(!t.opened(false), "no cached rows to list first");
        assert_eq!(t.pending().as_deref(), Some("transcripts"));
        t.reading((3, 10));
        assert_eq!(t.pending().as_deref(), Some("transcripts 3/10"));
        assert!(!t.read());
        assert!(t.scanned());
        assert!(t.read());
        assert_eq!(t.pending(), None);
    }

    #[test]
    fn without_a_claude_dir_no_start_waits_for_transcripts() {
        let t = TranscriptsLoad::new(false);
        assert!(t.read());
        assert_eq!(t.pending(), None);
    }
}
