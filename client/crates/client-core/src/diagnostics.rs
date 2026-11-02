//! Problems reported by the backend, per file: the editor marks them, the
//! Problems tool window lists them.
//!
//! Each `IdeEvent::Diagnostics` carries one producer's complete set for one
//! file (a language server, an inspection), so a new set replaces that
//! producer's previous one and an empty set clears it.

use crate::Backend;
use gpui::{Context, Entity, EventEmitter, Subscription};
use ide_core::{Diagnostic, IdeEvent};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The diagnostics of every file, without GPUI: what [`DiagnosticsStore`] holds.
#[derive(Debug, Default)]
pub struct Problems {
    /// `(path, producer)` → that producer's full set for the file.
    sets: BTreeMap<(PathBuf, String), Vec<Diagnostic>>,
}

impl Problems {
    /// Replaces `source`'s set for `path`.
    pub fn set(&mut self, path: PathBuf, source: String, items: Vec<Diagnostic>) {
        if items.is_empty() {
            self.sets.remove(&(path, source));
        } else {
            self.sets.insert((path, source), items);
        }
    }

    /// Every producer's diagnostics for `path`, by range (the editor needs them in order).
    pub fn for_path(&self, path: &Path) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self
            .sets
            .range((path.to_path_buf(), String::new())..)
            .take_while(|((p, _), _)| p == path)
            .flat_map(|(_, items)| items.iter().cloned())
            .collect();
        out.sort_by_key(|d| (d.range.start, d.range.end));
        out
    }

    /// Every diagnostic with its file, files in path order.
    pub fn all(&self) -> impl Iterator<Item = (&Path, &Diagnostic)> {
        self.sets.iter().flat_map(|((path, _), items)| items.iter().map(move |d| (path.as_path(), d)))
    }

    pub fn len(&self) -> usize {
        self.sets.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

/// Emitted after the diagnostics of `path` changed.
pub struct DiagnosticsChanged {
    pub path: PathBuf,
}

pub struct DiagnosticsStore {
    problems: Problems,
    _events: Subscription,
}

impl EventEmitter<DiagnosticsChanged> for DiagnosticsStore {}

impl DiagnosticsStore {
    pub fn new(backend: &Entity<Backend>, cx: &mut Context<Self>) -> Self {
        let events = cx.subscribe(backend, |this, _, event: &IdeEvent, cx| {
            if let IdeEvent::Diagnostics { path, source, items } = event {
                this.problems.set(path.0.clone(), source.clone(), items.clone());
                cx.emit(DiagnosticsChanged { path: path.0.clone() });
                cx.notify();
            }
        });
        Self { problems: Problems::default(), _events: events }
    }

    pub fn problems(&self) -> &Problems {
        &self.problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_core::{Pos, Range, Severity};

    fn diag(line: u32, message: &str) -> Diagnostic {
        serde_json::from_value(serde_json::json!({
            "range": Range { start: Pos::new(line, 0), end: Pos::new(line, 1) },
            "severity": Severity::Error,
            "message": message,
        }))
        .unwrap()
    }

    #[test]
    fn a_producer_replaces_its_own_set_and_merges_with_others() {
        let mut problems = Problems::default();
        let main = PathBuf::from("/p/src/main.rs");
        problems.set(main.clone(), "rust-analyzer".into(), vec![diag(3, "b"), diag(1, "a")]);
        problems.set(main.clone(), "spell".into(), vec![diag(2, "typo")]);
        problems.set(PathBuf::from("/p/src/lib.rs"), "rust-analyzer".into(), vec![diag(0, "c")]);
        let lines: Vec<u32> = problems.for_path(&main).iter().map(|d| d.range.start.line).collect();
        assert_eq!(lines, [1, 2, 3], "in range order across producers");
        assert_eq!(problems.len(), 4);

        problems.set(main.clone(), "rust-analyzer".into(), vec![]);
        assert_eq!(problems.for_path(&main).len(), 1, "an empty set clears only that producer");
        let files: Vec<&Path> = problems.all().map(|(p, _)| p).collect();
        assert_eq!(files, [Path::new("/p/src/lib.rs"), main.as_path()]);
    }
}
