//! The Search Everywhere popup: tabs, a query field, results by section.
//! Up / Down move, Enter opens, Tab switches tabs, Escape closes.

use crate::model::{
    ActionMatch, FileMatch, Results, Row, Section, SymbolMatch, TABS, Tab, limit, match_actions, rows, sections_for,
    step, symbol_icon,
};
use client_core::icons::file_icon_name;
use client_core::keymap::shortcut_label;
use client_core::theme::token;
use client_core::{Extensions, Icon, Services, Workbench};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{h_flex, v_flex};
use serde_json::json;
use std::path::Path;

pub struct SearchEverywhere {
    services: Services,
    workbench: WeakEntity<Workbench>,
    tab: Tab,
    input: Entity<InputState>,
    rows: Vec<Row>,
    selected: Option<usize>,
    /// Drops results of a query that has been typed past.
    generation: u64,
    _input: Subscription,
}

impl EventEmitter<DismissEvent> for SearchEverywhere {}

impl Focusable for SearchEverywhere {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl SearchEverywhere {
    pub fn new(
        services: &Services,
        workbench: WeakEntity<Workbench>,
        tab: Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type / to see commands"));
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.search(cx);
            }
        });
        let mut popup = Self {
            services: services.clone(),
            workbench,
            tab,
            input,
            rows: Vec::new(),
            selected: None,
            generation: 0,
            _input: subscription,
        };
        popup.search(cx);
        popup
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_owned()
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (tab, query) = (self.tab, self.query(cx));
        let conn = self.services.backend.read(cx).connection().clone();
        let actions: Vec<ActionMatch> = cx
            .try_global::<Extensions>()
            .map(|ext| {
                ext.actions()
                    .into_iter()
                    .map(|a| ActionMatch {
                        id: a.id.to_string(),
                        title: a.title.to_string(),
                        category: a.category.as_ref().map(|c| c.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let n = limit(tab);
        let sections = sections_for(tab, query.is_empty());
        let requests: Vec<_> = sections
            .iter()
            .map(|section| {
                let section = *section;
                let fut: std::pin::Pin<Box<dyn std::future::Future<Output = Results>>> = match section {
                    Section::Recent => {
                        let read = conn.invoke::<Vec<std::path::PathBuf>>("recent_files", json!({}));
                        let root = self.services.backend.read(cx).project().root.0.clone();
                        Box::pin(async move {
                            let paths = read.await.unwrap_or_default();
                            Results::Files(
                                paths
                                    .into_iter()
                                    .map(|path| {
                                        let rel = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
                                        FileMatch { path, rel_path: rel }
                                    })
                                    .collect(),
                            )
                        })
                    }
                    Section::Files => {
                        let read = conn.invoke::<Vec<FileMatch>>("find_files", json!({ "query": query, "limit": n }));
                        Box::pin(async move { Results::Files(read.await.unwrap_or_default()) })
                    }
                    Section::Classes | Section::Symbols => {
                        let filter = if section == Section::Classes { "types" } else { "all" };
                        let read = conn.invoke::<Vec<SymbolMatch>>(
                            "find_symbols",
                            json!({ "query": query, "filter": filter, "limit": n }),
                        );
                        Box::pin(async move { Results::Symbols(read.await.unwrap_or_default()) })
                    }
                    Section::Actions => {
                        let hits = if query.is_empty() {
                            actions.iter().take(n).cloned().collect()
                        } else {
                            match_actions(&query, &actions, n)
                        };
                        Box::pin(async move { Results::Actions(hits) })
                    }
                };
                (section, fut)
            })
            .collect();
        cx.spawn(async move |this, cx| {
            let mut results = Vec::new();
            for (section, fut) in requests {
                results.push((section, fut.await));
            }
            let _ = this.update(cx, |popup, cx| {
                if popup.generation == generation {
                    popup.rows = rows(tab, results);
                    popup.selected = step(&popup.rows, None, 1);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.search(cx);
        cx.notify();
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix).cloned() else { return };
        cx.emit(DismissEvent);
        match row {
            Row::Header(_) => {}
            Row::File(FileMatch { path, .. }) => {
                self.services.editors.update(cx, |editors, cx| editors.open(path, cx));
            }
            // A symbol opens with the caret on it.
            Row::Symbol(SymbolMatch { path, range, .. }) => {
                self.services.editors.update(cx, |editors, cx| editors.open_at(path, Some(range.start), cx));
            }
            Row::Action(action) => {
                let workbench = self.workbench.clone();
                // After the popup closed: the action may open another one.
                window.defer(cx, move |window, cx| {
                    let _ = workbench.update(cx, |wb, cx| wb.run_action(&action.id, window, cx));
                });
            }
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "down" => self.selected = step(&self.rows, self.selected, 1),
            "up" => self.selected = step(&self.rows, self.selected, -1),
            "enter" => {
                if let Some(ix) = self.selected {
                    self.activate(ix, window, cx);
                }
            }
            "escape" => cx.emit(DismissEvent),
            "tab" => {
                let at = TABS.iter().position(|(t, _)| *t == self.tab).unwrap_or(0) as isize;
                let next = (at + if key.modifiers.shift { -1 } else { 1 }).rem_euclid(TABS.len() as isize);
                self.set_tab(TABS[next as usize].0, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (muted, selected) = (token(cx, "fg-muted"), token(cx, "bg-list-selected"));
        let base = h_flex()
            .id(("se-row", ix))
            .h(px(26.))
            .px(px(10.))
            .gap(px(8.))
            .rounded(px(4.))
            .when(self.selected == Some(ix), |r| r.bg(selected))
            .on_click(cx.listener(move |popup, _: &ClickEvent, window, cx| popup.activate(ix, window, cx)));
        match &self.rows[ix] {
            Row::Header(title) => div()
                .px(px(10.))
                .pt(px(8.))
                .pb(px(2.))
                .text_size(px(12.))
                .text_color(muted)
                .child(*title)
                .into_any_element(),
            Row::File(f) => {
                let name =
                    Path::new(&f.rel_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let dir = Path::new(&f.rel_path).parent().map(|d| d.display().to_string()).unwrap_or_default();
                base.child(Icon::new(file_icon_name(&name)).size(px(16.)))
                    .child(name)
                    .child(div().text_color(muted).truncate().child(dir))
                    .into_any_element()
            }
            Row::Symbol(s) => base
                .child(Icon::new(symbol_icon(&s.kind)).size(px(16.)))
                .child(s.name.clone())
                .child(div().text_color(muted).truncate().child(s.container.clone().unwrap_or_default()))
                .child(div().flex_1())
                .child(div().text_color(muted).child(s.rel_path.clone()))
                .into_any_element(),
            Row::Action(a) => {
                let keys = shortcut_label(cx, &a.id).unwrap_or_default();
                base.child(div().size(px(16.)))
                    .child(a.title.clone())
                    .child(div().text_color(muted).child(a.category.clone().unwrap_or_default()))
                    .child(div().flex_1())
                    .child(div().text_color(muted).child(keys))
                    .into_any_element()
            }
        }
    }
}

impl Render for SearchEverywhere {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (fg, muted, hover) = (token(cx, "fg"), token(cx, "fg-muted"), token(cx, "bg-tab-hover"));
        let tabs: Vec<_> = TABS
            .iter()
            .map(|(tab, label)| {
                let tab = *tab;
                let active = tab == self.tab;
                div()
                    .id(SharedString::from(format!("se-tab-{label}")))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .text_color(if active { fg } else { muted })
                    .when(active, |t| t.bg(token(cx, "bg-tab-selected")))
                    .hover(move |t| t.bg(hover))
                    .child(*label)
                    .on_click(cx.listener(move |popup, _: &ClickEvent, _, cx| popup.set_tab(tab, cx)))
            })
            .collect();
        let rows: Vec<_> = (0..self.rows.len()).map(|ix| self.row(ix, cx)).collect();
        let empty = self.rows.is_empty().then(|| {
            let text = if self.query(cx).is_empty() { "Type to search" } else { "Nothing found" };
            div().p(px(16.)).text_color(muted).child(text)
        });
        v_flex()
            .id("search-everywhere")
            .w(px(670.))
            .bg(token(cx, "bg-popup"))
            .border_1()
            .border_color(token(cx, "border-popup"))
            .rounded(px(10.))
            .shadow_lg()
            .text_size(px(13.))
            .text_color(fg)
            .overflow_hidden()
            // Keep clicks inside from closing the popup.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(Self::on_key_down))
            .child(h_flex().px(px(8.)).pt(px(8.)).gap(px(2.)).children(tabs))
            .child(div().p(px(8.)).child(Input::new(&self.input)))
            .child(
                div()
                    .id("se-results")
                    // As tall as the results, up to a limit, then scrolling.
                    .max_h(px(400.))
                    .overflow_y_scroll()
                    .px(px(4.))
                    .pb(px(6.))
                    .children(rows)
                    .children(empty),
            )
    }
}
