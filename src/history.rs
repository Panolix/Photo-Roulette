//! A small back-stack of items already shown in this session.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct History {
    paths: Vec<PathBuf>,
    pos: Option<usize>,
}

impl History {
    pub fn clear(&mut self) {
        self.paths.clear();
        self.pos = None;
    }

    /// Record a newly dealt item, dropping any forward history first.
    pub fn record(&mut self, path: PathBuf) {
        let keep = self.pos.map_or(0, |pos| pos + 1);
        self.paths.truncate(keep);
        self.paths.push(path);
        self.pos = Some(self.paths.len() - 1);
    }

    pub fn can_go_back(&self) -> bool {
        self.pos.is_some_and(|pos| pos > 0)
    }

    pub fn previous(&self) -> Option<(usize, PathBuf)> {
        let pos = self.pos?;
        let previous = pos.checked_sub(1)?;
        self.paths
            .get(previous)
            .cloned()
            .map(|path| (previous, path))
    }

    pub fn set_pos(&mut self, pos: usize) -> bool {
        if pos < self.paths.len() {
            self.pos = Some(pos);
            true
        } else {
            false
        }
    }

    pub fn remove(&mut self, path: &Path) {
        let Some(index) = self.paths.iter().position(|candidate| candidate == path) else {
            return;
        };
        self.paths.remove(index);

        self.pos = match self.pos {
            None => None,
            Some(_) if self.paths.is_empty() => None,
            Some(pos) if pos > index => Some(pos - 1),
            Some(pos) if pos == index => Some(index.min(self.paths.len() - 1)),
            other => other,
        };
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.paths.len()
    }
}

#[cfg(test)]
mod tests {
    use super::History;
    use std::path::PathBuf;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("/photos/{name}.jpg"))
    }

    #[test]
    fn back_walks_recorded_items() {
        let mut history = History::default();
        history.record(path("a"));
        history.record(path("b"));
        history.record(path("c"));

        assert!(history.can_go_back());
        let (pos, previous) = history.previous().expect("previous item");
        assert_eq!(previous, path("b"));
        assert!(history.set_pos(pos));

        let (pos, previous) = history.previous().expect("previous item");
        assert_eq!(previous, path("a"));
        assert!(history.set_pos(pos));
        assert!(!history.can_go_back());
    }

    #[test]
    fn new_item_drops_forward_history() {
        let mut history = History::default();
        history.record(path("a"));
        history.record(path("b"));
        history.record(path("c"));
        let (pos, _) = history.previous().expect("previous");
        history.set_pos(pos);
        history.record(path("d"));

        assert_eq!(history.len(), 3);
        let (_, previous) = history.previous().expect("previous");
        assert_eq!(previous, path("b"));
    }

    #[test]
    fn removing_the_current_item_keeps_a_sane_position() {
        let mut history = History::default();
        history.record(path("a"));
        history.record(path("b"));
        history.record(path("c"));

        history.remove(&path("b"));
        assert_eq!(history.len(), 2);
        let (_, previous) = history.previous().expect("previous after removal");
        assert_eq!(previous, path("a"));
    }

    #[test]
    fn clearing_resets_everything() {
        let mut history = History::default();
        history.record(path("a"));
        history.clear();
        assert!(!history.can_go_back());
        assert!(history.previous().is_none());
    }
}
