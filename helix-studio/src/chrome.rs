use helix_term::compositor::Event;
use helix_view::document::SCRATCH_BUFFER_NAME;
use helix_view::editor::{Action, BufferLine, CloseError};
use helix_view::graphics::Rect;
use helix_view::input::{MouseButton, MouseEventKind};
use helix_view::theme::Style;
use helix_view::{DocumentId, Editor};
use tui::buffer::Buffer as Surface;

pub const CLOSE: &str = "\u{00d7}";

struct Tab {
    doc: DocumentId,
    start: u16,
    end: u16,
    close: u16,
}

#[derive(Default)]
pub struct Chrome {
    tabs: Vec<Tab>,
    hover: Option<DocumentId>,
    row: Option<u16>,
}

impl Chrome {
    pub fn visible(editor: &Editor) -> bool {
        match editor.config().bufferline {
            BufferLine::Always => true,
            BufferLine::Multiple => editor.documents().count() > 1,
            BufferLine::Never => false,
        }
    }

    pub fn hover(&self) -> Option<DocumentId> {
        self.hover
    }

    pub fn render(&mut self, editor: &Editor, area: Rect, surface: &mut Surface) {
        let theme = &editor.theme;
        let background = theme
            .try_get("ui.bufferline.background")
            .unwrap_or_else(|| theme.get("ui.statusline"));
        let active = theme
            .try_get("ui.bufferline.active")
            .unwrap_or_else(|| theme.get("ui.statusline.active"));
        let inactive = theme
            .try_get("ui.bufferline")
            .unwrap_or_else(|| theme.get("ui.statusline.inactive"));

        let close_fg = theme
            .try_get("diagnostic.error")
            .and_then(|style| style.fg)
            .or_else(|| theme.get("ui.text").fg);
        let close_style = match close_fg {
            Some(fg) => Style::default().fg(fg),
            None => Style::default(),
        };

        surface.clear_with(area, background);

        self.tabs.clear();
        self.row = Some(area.y);

        let current = editor.tree.try_get(editor.tree.focus).map(|view| view.doc);
        let mut x = area.x;

        for doc in editor.documents() {
            let name = doc
                .path()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or(SCRATCH_BUFFER_NAME);

            let style = if current == Some(doc.id()) {
                active
            } else {
                inactive
            };

            let label = format!(" {}{}  ", name, if doc.is_modified() { " +" } else { "" });
            let width = label.chars().count() as u16;

            if x + width > area.right() {
                break;
            }

            let start = x;
            let end = surface
                .set_stringn(x, area.y, &label, area.width as usize, style)
                .0;
            let close = end.saturating_sub(2);

            if self.hover == Some(doc.id()) {
                surface.set_stringn(close, area.y, CLOSE, 1, close_style);
            }

            self.tabs.push(Tab {
                doc: doc.id(),
                start,
                end,
                close,
            });

            x = end;
        }
    }

    pub fn input(&mut self, event: &Event, editor: &mut Editor) -> bool {
        let Event::Mouse(mouse) = event else {
            return false;
        };

        if self.row != Some(mouse.row) || !Chrome::visible(editor) {
            return self.clear_hover();
        }

        let hit = self
            .tabs
            .iter()
            .find(|tab| mouse.column >= tab.start && mouse.column < tab.end)
            .map(|tab| (tab.doc, tab.close));

        let Some((doc, close)) = hit else {
            self.clear_hover();
            return true;
        };

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse.column == close {
                    self.close(doc, editor);
                } else {
                    editor.switch(doc, Action::Replace);
                }
                helix_event::request_redraw();
            }
            _ => {
                if self.hover != Some(doc) {
                    self.hover = Some(doc);
                    helix_event::request_redraw();
                }
            }
        }

        true
    }

    fn clear_hover(&mut self) -> bool {
        if self.hover.take().is_some() {
            helix_event::request_redraw();
        }
        false
    }

    fn close(&mut self, doc: DocumentId, editor: &mut Editor) {
        match editor.close_document(doc, false) {
            Ok(()) => self.hover = None,
            Err(CloseError::BufferModified(name)) => {
                editor.set_error(format!("buffer {} has unsaved changes", name))
            }
            Err(_) => {}
        }
    }
}
