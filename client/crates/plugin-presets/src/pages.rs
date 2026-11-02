//! Settings pages for the presets: Appearance (theme, Sync with OS) and Keymap.
//! A choice writes the global setting at once; the workbench re-applies on
//! `SettingsChanged`.

use client_core::keymap::default_preset;
use client_core::theme::token;
use client_core::{Services, SettingsChanged, SettingsScope};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use ide_core::settings::Settings;
use serde_json::{Value, json};

/// One setting picked from a list (`appearance.theme`, `keymap`).
pub struct Choice {
    pub key: &'static str,
    pub label: &'static str,
    pub options: Vec<(SharedString, SharedString)>,
    pub current: fn(&Settings) -> String,
}

/// An on/off setting under the choices: its key, label and current value.
pub type Toggle = (&'static str, &'static str, fn(&Settings) -> bool);

/// A page of choices, plus an optional on/off setting below them.
pub struct ChoicePage {
    services: Services,
    choice: Choice,
    toggle: Option<Toggle>,
}

impl ChoicePage {
    pub fn new(services: &Services, choice: Choice, toggle: Option<Toggle>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&services.settings, |_, _, _: &SettingsChanged, cx| cx.notify()).detach();
        Self { services: services.clone(), choice, toggle }
    }

    fn write(&self, key: &'static str, value: Value, cx: &mut Context<Self>) {
        let task = self.services.settings.update(cx, |store, cx| store.set(SettingsScope::Global, key, value, cx));
        cx.spawn(async move |_, _| {
            if let Err(e) = task.await {
                tracing::warn!("settings_set {key}: {}", e.message);
            }
        })
        .detach();
    }
}

impl Render for ChoicePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.services.settings.read(cx).settings().clone();
        let current = (self.choice.current)(&settings);
        let (selected, hover, muted) =
            (token(cx, "bg-list-selected"), token(cx, "bg-tree-hover"), token(cx, "fg-muted"));
        let key = self.choice.key;
        let rows: Vec<_> = self
            .choice
            .options
            .iter()
            .enumerate()
            .map(|(ix, (id, label))| {
                let chosen = *id == current;
                let value = id.to_string();
                h_flex()
                    .id(("choice", ix))
                    .h(px(28.))
                    .px(px(10.))
                    .gap(px(8.))
                    .rounded(px(4.))
                    .when(chosen, |r| r.bg(selected))
                    .when(!chosen, |r| r.hover(move |r| r.bg(hover)))
                    .child(div().w(px(14.)).child(if chosen { "✓" } else { "" }))
                    .child(label.clone())
                    .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| page.write(key, json!(value), cx)))
            })
            .collect();
        let toggle = self.toggle.map(|(key, label, get)| {
            let on = get(&settings);
            h_flex()
                .id("choice-toggle")
                .h(px(28.))
                .px(px(10.))
                .gap(px(8.))
                .rounded(px(4.))
                .hover(move |r| r.bg(hover))
                .child(div().w(px(14.)).child(if on { "☑" } else { "☐" }))
                .child(label)
                .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| page.write(key, json!(!on), cx)))
        });
        v_flex()
            .gap(px(6.))
            .child(div().text_color(muted).child(self.choice.label))
            .child(v_flex().w(px(320.)).gap(px(2.)).children(rows))
            .children(toggle)
    }
}

/// The theme the Appearance page marks: the setting, or Islands Light when unset (React's default).
pub fn current_theme(s: &Settings) -> String {
    if s.appearance.theme.is_empty() { "light".into() } else { s.appearance.theme.clone() }
}

/// The preset the Keymap page marks: the setting, or this OS's IDEA keymap.
pub fn current_keymap(s: &Settings) -> String {
    if s.keymap.is_empty() { default_preset().into() } else { s.keymap.clone() }
}
