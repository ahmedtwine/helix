use std::path::PathBuf;
use std::sync::OnceLock;

use helix_view::{graphics::Rect, Editor};
use tui::buffer::Buffer as Surface;

use crate::compositor::{Component, Context};

pub struct StartupContext {
    pub root: PathBuf,
    pub opening_directory: bool,
    pub file_count: usize,
    pub tutor: bool,
}

pub enum UiRequest {
    FilePicker { root: PathBuf },
    FileExplorer { root: PathBuf },
    StartupDirectory { root: PathBuf },
}

pub enum Opened {
    Layer(Box<dyn Component>),
    Handled,
}

pub trait UiHooks: Send + Sync + 'static {
    fn claims_startup(&self, _cx: &StartupContext) -> bool {
        false
    }

    fn startup(&self, _editor: &mut Editor, _cx: &StartupContext) {}

    fn mount(&self, layer: Box<dyn Component>) -> Box<dyn Component> {
        layer
    }

    fn render_top(&self, _area: Rect, _surface: &mut Surface, _ctx: &mut Context) {}

    fn render_bufferline(&self, _editor: &Editor, _area: Rect, _surface: &mut Surface) -> bool {
        false
    }


    fn input(&self, _event: &crate::compositor::Event, _cx: &mut Context) -> bool {
        false
    }

    fn open(&self, _request: &UiRequest, _editor: &mut Editor) -> Option<Opened> {
        None
    }
}

static HOOKS: OnceLock<Box<dyn UiHooks>> = OnceLock::new();

pub fn install(hooks: impl UiHooks) -> bool {
    HOOKS.set(Box::new(hooks)).is_ok()
}

pub fn get() -> Option<&'static dyn UiHooks> {
    HOOKS.get().map(|hooks| &**hooks)
}

pub fn mount(layer: Box<dyn Component>) -> Box<dyn Component> {
    match get() {
        Some(hooks) => hooks.mount(layer),
        None => layer,
    }
}

pub fn open(request: UiRequest, editor: &mut Editor) -> Option<Opened> {
    get()?.open(&request, editor)
}

pub fn render_bufferline(editor: &Editor, area: Rect, surface: &mut Surface) -> bool {
    match get() {
        Some(hooks) => hooks.render_bufferline(editor, area, surface),
        None => false,
    }
}

pub fn input(event: &crate::compositor::Event, cx: &mut Context) -> bool {
    match get() {
        Some(hooks) => hooks.input(event, cx),
        None => false,
    }
}

pub fn claims_startup(cx: &StartupContext) -> bool {
    match get() {
        Some(hooks) => hooks.claims_startup(cx),
        None => false,
    }
}

pub fn startup(editor: &mut Editor, cx: &StartupContext) {
    if let Some(hooks) = get() {
        hooks.startup(editor, cx);
    }
}
