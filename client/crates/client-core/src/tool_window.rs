//! The dock panel around a contributed view: plugins hand in a plain view and
//! never implement the dock's panel traits themselves.

use gpui::{
    AnyView, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div,
};
use gpui_component::dock::{BasePanel, Panel, PanelEvent};

pub(crate) struct ToolWindowPanel {
    /// The tool window id; the panel name the layout is saved under.
    id: &'static str,
    title: SharedString,
    /// Whether the group shows a title or tab bar (the editor area has its own tabs).
    title_bar: bool,
    content: AnyView,
    focus: FocusHandle,
}

impl ToolWindowPanel {
    pub(crate) fn new(
        id: &'static str,
        title: SharedString,
        title_bar: bool,
        content: AnyView,
        cx: &mut Context<Self>,
    ) -> Self {
        Self { id, title, title_bar, content, focus: cx.focus_handle() }
    }
}

impl EventEmitter<PanelEvent> for ToolWindowPanel {}

impl Focusable for ToolWindowPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ToolWindowPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.content.clone())
    }
}

impl BasePanel for ToolWindowPanel {
    fn panel_name(&self) -> &'static str {
        self.id
    }

    /// Tool windows are hidden (their dock toggled), not closed: a closed one
    /// would have nothing to bring it back.
    fn closable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for ToolWindowPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.title.clone())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn title_bar(&self, _: &App) -> bool {
        self.title_bar
    }
}
