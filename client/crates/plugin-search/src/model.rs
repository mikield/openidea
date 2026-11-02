//! What Search Everywhere shows (React: `src/features/search/everywhere/model.ts`):
//! tabs, the sections each queries, and the rows built from their results.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    All,
    Classes,
    Files,
    Symbols,
    Actions,
}

pub const TABS: [(Tab, &str); 5] = [
    (Tab::All, "All"),
    (Tab::Classes, "Classes"),
    (Tab::Files, "Files"),
    (Tab::Symbols, "Symbols"),
    (Tab::Actions, "Actions"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Recent,
    Classes,
    Files,
    Symbols,
    Actions,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Recent => "Recent Files",
            Section::Classes => "Classes",
            Section::Files => "Files",
            Section::Symbols => "Symbols",
            Section::Actions => "Actions",
        }
    }
}

/// React's `ALL_TAB_LIMIT` / `TAB_LIMIT`.
pub const ALL_TAB_LIMIT: usize = 6;
pub const TAB_LIMIT: usize = 50;

/// Sections a tab queries (React's `sectionsFor`). An empty Actions query lists
/// every action (React shows recent actions, which are not tracked yet).
pub fn sections_for(tab: Tab, empty_query: bool) -> Vec<Section> {
    match (tab, empty_query) {
        (Tab::All | Tab::Files, true) => vec![Section::Recent],
        (Tab::Actions, true) => vec![Section::Actions],
        (_, true) => vec![],
        (Tab::All, false) => vec![Section::Classes, Section::Files, Section::Symbols, Section::Actions],
        (Tab::Classes, false) => vec![Section::Classes],
        (Tab::Files, false) => vec![Section::Files],
        (Tab::Symbols, false) => vec![Section::Symbols],
        (Tab::Actions, false) => vec![Section::Actions],
    }
}

pub fn limit(tab: Tab) -> usize {
    if tab == Tab::All { ALL_TAB_LIMIT } else { TAB_LIMIT }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub path: PathBuf,
    pub rel_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolMatch {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub path: PathBuf,
    pub rel_path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionMatch {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Row {
    Header(&'static str),
    File(FileMatch),
    Symbol(SymbolMatch),
    Action(ActionMatch),
}

impl Row {
    pub fn selectable(&self) -> bool {
        !matches!(self, Row::Header(_))
    }
}

/// Results of one section.
pub enum Results {
    Files(Vec<FileMatch>),
    Symbols(Vec<SymbolMatch>),
    Actions(Vec<ActionMatch>),
}

/// Rows for the sections' results: headers only in the All tab, empty sections left out.
pub fn rows(tab: Tab, results: Vec<(Section, Results)>) -> Vec<Row> {
    let mut out = Vec::new();
    for (section, results) in results {
        let items: Vec<Row> = match results {
            Results::Files(f) => f.into_iter().map(Row::File).collect(),
            Results::Symbols(s) => s.into_iter().map(Row::Symbol).collect(),
            Results::Actions(a) => a.into_iter().map(Row::Action).collect(),
        };
        if items.is_empty() {
            continue;
        }
        if tab == Tab::All {
            out.push(Row::Header(section.title()));
        }
        out.extend(items.into_iter().take(limit(tab)));
    }
    out
}

/// Actions whose title (or id) contains every query word, case-insensitively,
/// title prefix matches first.
pub fn match_actions(query: &str, actions: &[ActionMatch], limit: usize) -> Vec<ActionMatch> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(bool, &ActionMatch)> = actions
        .iter()
        .filter(|a| {
            let hay = format!("{} {}", a.title.to_lowercase(), a.id.to_lowercase());
            words.iter().all(|w| hay.contains(w))
        })
        .map(|a| (!a.title.to_lowercase().starts_with(&q), a))
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
    hits.into_iter().take(limit).map(|(_, a)| a.clone()).collect()
}

/// The next selectable row from `from` in `direction` (wrapping), if any.
pub fn step(rows: &[Row], from: Option<usize>, direction: isize) -> Option<usize> {
    let n = rows.len() as isize;
    if n == 0 {
        return None;
    }
    let mut ix = from.map_or(if direction > 0 { -1 } else { 0 }, |i| i as isize);
    for _ in 0..n {
        ix = (ix + direction).rem_euclid(n);
        if rows[ix as usize].selectable() {
            return Some(ix as usize);
        }
    }
    None
}

/// React's `symbol-<kind>` icon for a backend symbol kind.
pub fn symbol_icon(kind: &str) -> String {
    let name = match kind {
        "typeAlias" | "typeParameter" => "type",
        "enumMember" | "constant" => "constant",
        other => other,
    };
    format!("symbol-{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(id: &str, title: &str) -> ActionMatch {
        ActionMatch { id: id.into(), title: title.into(), category: None }
    }

    #[test]
    fn sections_follow_react() {
        assert_eq!(sections_for(Tab::All, true), [Section::Recent]);
        assert_eq!(sections_for(Tab::Classes, true), []);
        assert_eq!(
            sections_for(Tab::All, false),
            [Section::Classes, Section::Files, Section::Symbols, Section::Actions]
        );
        assert_eq!(sections_for(Tab::Files, false), [Section::Files]);
    }

    #[test]
    fn rows_have_headers_only_in_the_all_tab_and_skip_empty_sections() {
        let file = FileMatch { path: "/p/a.rs".into(), rel_path: "a.rs".into() };
        let results =
            || vec![(Section::Classes, Results::Symbols(vec![])), (Section::Files, Results::Files(vec![file.clone()]))];
        let all = rows(Tab::All, results());
        assert!(matches!(all.as_slice(), [Row::Header("Files"), Row::File(_)]));
        let files = rows(Tab::Files, results());
        assert!(matches!(files.as_slice(), [Row::File(_)]));
    }

    #[test]
    fn actions_match_words_with_prefixes_first() {
        let actions =
            [action("file.save", "Save"), action("vcs.saveStash", "Stash Changes and Save"), action("x", "Exit")];
        let hits: Vec<String> = match_actions("sav", &actions, 10).into_iter().map(|a| a.title).collect();
        assert_eq!(hits, ["Save", "Stash Changes and Save"]);
        assert!(match_actions("stash save", &actions, 10).len() == 1);
    }

    #[test]
    fn selection_skips_headers_and_wraps() {
        let file = || Row::File(FileMatch { path: "/a".into(), rel_path: "a".into() });
        let rows = [Row::Header("Files"), file(), file(), Row::Header("Actions"), Row::Action(action("a", "A"))];
        assert_eq!(step(&rows, None, 1), Some(1));
        assert_eq!(step(&rows, Some(2), 1), Some(4));
        assert_eq!(step(&rows, Some(4), 1), Some(1));
        assert_eq!(step(&rows, Some(1), -1), Some(4));
        assert_eq!(step(&[Row::Header("x")], None, 1), None);
    }
}
