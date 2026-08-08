pub mod engine;

use helix_core::unicode::width::UnicodeWidthStr;
use helix_term::compositor::Context;
use helix_view::graphics::Rect;
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use super::{Source, StudioEvent};
use engine::Mode;

pub struct Hover {
    mode: Mode,
    lines: usize,
    width: usize,
}

impl Hover {
    pub fn new(settings: &toml::Table) -> Self {
        let number = |key: &str, fallback: i64, high: i64| {
            settings
                .get(key)
                .and_then(toml::Value::as_integer)
                .unwrap_or(fallback)
                .clamp(1, high) as usize
        };

        Self {
            mode: settings
                .get("mode")
                .and_then(toml::Value::as_str)
                .map(Mode::parse)
                .unwrap_or(Mode::Signature),
            lines: number("lines", 8, 40),
            width: number("max-width", 110, 400),
        }
    }

    fn text(&self, editor: &Editor) -> Option<Vec<String>> {
        let view = editor.tree.try_get(editor.tree.focus)?;
        let doc = editor.document(view.doc)?;
        let cursor = doc
            .selection(view.id)
            .primary()
            .cursor(doc.text().slice(..));

        let mut lines: Vec<String> = diagnostics(doc, cursor);

        if let Some(raw) = engine::contents_at(doc.id(), cursor) {
            if !lines.is_empty() {
                lines.push(String::new());
            }

            lines.extend(
                self.mode
                    .render(&raw)
                    .lines()
                    .map(|line| line.trim_end().to_string())
                    .filter(|line| !line.is_empty()),
            );
        }

        lines.truncate(self.lines);

        (!lines.is_empty()).then_some(lines)
    }
}

fn diagnostics(doc: &helix_view::Document, cursor: usize) -> Vec<String> {
    doc.diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.range.start <= cursor && cursor <= diagnostic.range.end
        })
        .map(|diagnostic| {
            let marker = match diagnostic.severity {
                Some(helix_core::diagnostic::Severity::Error) => "error",
                Some(helix_core::diagnostic::Severity::Warning) => "warning",
                Some(helix_core::diagnostic::Severity::Info) => "info",
                _ => "hint",
            };

            format!("{marker}: {}", diagnostic.message.replace('\n', " "))
        })
        .collect()
}

impl Source for Hover {
    fn visible(&self, editor: &Editor) -> bool {
        self.text(editor).is_some()
    }

    fn size(&self, editor: &Editor, available: (u16, u16)) -> (u16, u16) {
        let Some(lines) = self.text(editor) else {
            return (0, 0);
        };

        let width = lines
            .iter()
            .map(|line| line.width())
            .max()
            .unwrap_or(0)
            .min(self.width)
            .min(available.0 as usize);

        (width as u16, lines.len() as u16)
    }

    fn observe(&mut self, event: &StudioEvent, _editor: &mut Editor) {
        if matches!(event, StudioEvent::DocumentChanged) {
            engine::clear();
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        let Some(lines) = self.text(cx.editor) else {
            return;
        };

        let theme = &cx.editor.theme;
        let style = theme
            .try_get("ui.popup.info")
            .unwrap_or_else(|| theme.get("ui.popup"));

        for (index, line) in lines.iter().enumerate() {
            let y = area.y + index as u16;
            if y >= area.bottom() {
                break;
            }

            surface.set_stringn(area.x, y, line, area.width as usize, style);
        }
    }
}
