pub mod engine;

use helix_term::compositor::{Context, Event};
use helix_view::graphics::Rect;
use helix_view::input::{MouseButton, MouseEventKind};
use helix_view::theme::{Style, Theme};
use helix_view::Editor;
use helix_vcs::Hunk;
use tui::buffer::Buffer as Surface;

use super::{Source, StudioEvent};
use engine::{Cell, Kind, Row};

const GUTTER: u16 = 5;
const SCROLL: usize = 3;

pub struct Diff {
    context: u32,
    offset: usize,
}

impl Diff {
    pub fn new(settings: &toml::Table) -> Self {
        let context = settings
            .get("context")
            .and_then(toml::Value::as_integer)
            .unwrap_or(3)
            .clamp(0, 32) as u32;

        Self { context, offset: 0 }
    }
}

impl Source for Diff {
    fn visible(&self, editor: &Editor) -> bool {
        hunk_count(editor).is_some_and(|count| count > 0)
    }

    fn observe(&mut self, event: &StudioEvent, _editor: &mut Editor) {
        if matches!(event, StudioEvent::DocumentChanged) {
            self.offset = 0;
        }
    }

    fn handle_input(&mut self, event: &Event, area: Rect, cx: &mut Context) -> bool {
        let Event::Mouse(mouse) = event else {
            return false;
        };

        if area.width == 0
            || mouse.column < area.x
            || mouse.column >= area.right()
            || mouse.row < area.y
            || mouse.row >= area.bottom()
        {
            return false;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.offset = self.offset.saturating_add(SCROLL);
                helix_event::request_redraw();
                true
            }
            MouseEventKind::ScrollUp => {
                self.offset = self.offset.saturating_sub(SCROLL);
                helix_event::request_redraw();
                true
            }
            MouseEventKind::Down(MouseButton::Left) if mouse.row == area.y => {
                stage(cx.editor);
                helix_event::request_redraw();
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        let rows = match rows(cx.editor, self.context) {
            Some(rows) if !rows.is_empty() => rows,
            _ => return,
        };

        let palette = Palette::new(&cx.editor.theme);

        surface.set_stringn(
            area.x,
            area.y,
            &format!(
                " {}  {} hunks  ·  click to stage ",
                name(cx.editor),
                hunk_count(cx.editor).unwrap_or(0)
            ),
            area.width as usize,
            palette.header,
        );

        let body = area.clip_top(1);
        if body.height == 0 || body.width < GUTTER * 2 + 3 {
            return;
        }

        let half = (body.width - 1) / 2;
        let left = Rect::new(body.x, body.y, half, body.height);
        let right = Rect::new(body.x + half + 1, body.y, body.width - half - 1, body.height);

        self.offset = self
            .offset
            .min(rows.len().saturating_sub(body.height as usize));

        for (index, row) in rows.iter().skip(self.offset).take(body.height as usize).enumerate() {
            let y = body.y + index as u16;

            surface.set_stringn(body.x + half, y, "│", 1, palette.divider);

            if row.is_gap() {
                surface.set_stringn(left.x, y, "⋯", left.width as usize, palette.divider);
                surface.set_stringn(right.x, y, "⋯", right.width as usize, palette.divider);
                continue;
            }

            paint(surface, left, y, row.left.as_ref(), &palette);
            paint(surface, right, y, row.right.as_ref(), &palette);
        }
    }
}

struct Palette {
    header: Style,
    divider: Style,
    number: Style,
    context: Style,
    added: Style,
    removed: Style,
}

impl Palette {
    fn new(theme: &Theme) -> Self {
        Self {
            header: theme
                .try_get("ui.bufferline.active")
                .unwrap_or_else(|| theme.get("ui.statusline")),
            divider: theme.get("ui.linenr"),
            number: theme.get("ui.linenr"),
            context: theme.get("ui.text"),
            added: theme
                .try_get("diff.plus")
                .unwrap_or_else(|| theme.get("ui.text")),
            removed: theme
                .try_get("diff.minus")
                .unwrap_or_else(|| theme.get("ui.text")),
        }
    }

    fn of(&self, kind: Kind) -> Style {
        match kind {
            Kind::Added => self.added,
            Kind::Removed => self.removed,
            Kind::Context | Kind::Gap => self.context,
        }
    }
}

fn paint(surface: &mut Surface, area: Rect, y: u16, cell: Option<&Cell>, palette: &Palette) {
    let Some(cell) = cell else {
        return;
    };

    let style = palette.of(cell.kind);
    surface.set_stringn(
        area.x,
        y,
        &format!("{:>4} ", cell.number + 1),
        GUTTER as usize,
        palette.number,
    );

    let marker = match cell.kind {
        Kind::Added => "+",
        Kind::Removed => "-",
        Kind::Context | Kind::Gap => " ",
    };

    surface.set_stringn(
        area.x + GUTTER,
        y,
        &format!("{marker}{}", cell.text),
        area.width.saturating_sub(GUTTER) as usize,
        style,
    );
}

fn hunk_count(editor: &Editor) -> Option<u32> {
    let view = editor.tree.try_get(editor.tree.focus)?;
    let doc = editor.document(view.doc)?;
    Some(doc.diff_handle()?.load().len())
}

fn rows(editor: &Editor, context: u32) -> Option<Vec<Row>> {
    let view = editor.tree.try_get(editor.tree.focus)?;
    let doc = editor.document(view.doc)?;
    let diff = doc.diff_handle()?.load();

    let hunks: Vec<Hunk> = (0..diff.len()).map(|index| diff.nth_hunk(index)).collect();

    Some(engine::rows(
        diff.diff_base(),
        diff.doc(),
        &hunks,
        context,
    ))
}

fn name(editor: &Editor) -> String {
    editor
        .tree
        .try_get(editor.tree.focus)
        .and_then(|view| editor.document(view.doc))
        .and_then(|doc| doc.path())
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn stage(editor: &mut Editor) {
    let path = editor
        .tree
        .try_get(editor.tree.focus)
        .and_then(|view| editor.document(view.doc))
        .and_then(|doc| doc.path())
        .map(|path| path.to_path_buf());

    let Some(path) = path else {
        return;
    };

    match engine::stage(&path) {
        Ok(()) => editor.set_status(format!("staged {}", path.display())),
        Err(err) => editor.set_error(err),
    }
}
