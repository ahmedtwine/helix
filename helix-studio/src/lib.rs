pub mod chrome;
pub mod config;
pub mod explorer;
pub mod icons;
pub mod overlay;
pub mod panel;
pub mod picker;
pub mod session;
pub mod tree;

use std::path::PathBuf;
use std::sync::Mutex;

use helix_event::register_hook;
use helix_term::compositor::{Context, Event};
use helix_term::events::{OnModeSwitch, PostCommand};
use helix_term::ui_hooks::{self, Opened, StartupContext, UiHooks, UiRequest};
use helix_view::events::{
    DocumentDidChange, DocumentDidClose, DocumentDidOpen, SelectionDidChange,
};
use helix_view::graphics::Rect;
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use panel::{Hover, StudioEvent};

#[derive(Default)]
pub struct Studio {
    chrome: Mutex<chrome::Chrome>,
}

impl UiHooks for Studio {
    fn render_bufferline(&self, editor: &Editor, area: Rect, surface: &mut Surface) -> bool {
        let Ok(mut chrome) = self.chrome.lock() else {
            return false;
        };
        chrome.render(editor, area, surface);
        true
    }

    fn render_top(&self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        panel::render(area, surface, cx);
    }

    fn input(&self, event: &Event, cx: &mut Context) -> bool {
        if panel::handle_input(event, cx) {
            return true;
        }

        let Ok(mut chrome) = self.chrome.lock() else {
            return false;
        };

        let before = chrome.hover();
        let handled = chrome.input(event, cx.editor);
        let after = chrome.hover();
        drop(chrome);

        if before != after {
            let hover = match after {
                Some(doc) => Hover::Tab(doc),
                None => Hover::None,
            };
            panel::observe(&StudioEvent::Hover(hover), cx.editor);
        }

        handled
    }

    fn claims_startup(&self, cx: &StartupContext) -> bool {
        if cx.tutor || (cx.file_count > 0 && !cx.opening_directory) {
            return false;
        }

        !session::restorable(&cx.root).is_empty()
    }

    fn startup(&self, editor: &mut Editor, cx: &StartupContext) {
        panel::install();
        register_session_hooks(cx.root.clone());
        register_stream_hooks();

        if self.claims_startup(cx) {
            session::restore(editor, &cx.root);
        }
    }

    fn open(&self, request: &UiRequest, editor: &mut Editor) -> Option<Opened> {
        match request {
            UiRequest::FilePicker { .. } => Some(Opened::Layer(Box::new(overlay::centered(
                picker::file_picker(editor, workspace_root()),
            )))),
            UiRequest::FileExplorer { .. } => {
                panel::toggle("sidebar");
                Some(Opened::Handled)
            }
            UiRequest::StartupDirectory { root } => Some(Opened::Layer(Box::new(
                overlay::centered(picker::file_picker(
                    editor,
                    helix_loader::find_workspace_in(root).0,
                )),
            ))),
        }
    }
}

fn register_session_hooks(root: PathBuf) {
    let on_open = root.clone();
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        session::save(event.editor, &on_open);
        Ok(())
    });

    let on_close = root;
    register_hook!(move |event: &mut DocumentDidClose<'_>| {
        session::save(event.editor, &on_close);
        Ok(())
    });
}

fn register_stream_hooks() {
    register_hook!(move |event: &mut PostCommand<'_, '_>| {
        let name = event.command.name().to_string();
        let mode = event.cx.editor.mode();
        panel::observe(
            &StudioEvent::Command { name: &name, mode },
            event.cx.editor,
        );
        Ok(())
    });

    register_hook!(move |event: &mut OnModeSwitch<'_, '_>| {
        panel::observe(
            &StudioEvent::ModeChanged {
                from: event.old_mode,
                to: event.new_mode,
            },
            event.cx.editor,
        );
        Ok(())
    });

    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        panel::observe(&StudioEvent::DocumentChanged, event.editor);
        Ok(())
    });

    let hover = panel::hover::engine::Handler::spawn(panel::hover_delay());

    let moved = hover.clone();
    register_hook!(move |event: &mut SelectionDidChange<'_>| {
        helix_event::send_blocking(
            &moved,
            panel::hover::engine::Event::Moved {
                doc: event.doc.id(),
                view: event.view,
            },
        );
        Ok(())
    });

    register_hook!(move |event: &mut DocumentDidChange<'_>| {
        let _ = event;
        helix_event::send_blocking(&hover, panel::hover::engine::Event::Dismiss);
        Ok(())
    });
}

pub fn workspace_root() -> PathBuf {
    helix_loader::find_workspace().0
}

pub fn install() {
    ui_hooks::install(Studio::default());
}
