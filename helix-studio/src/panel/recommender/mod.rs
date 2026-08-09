pub mod engine;

use helix_core::unicode::width::UnicodeWidthStr;
use helix_term::compositor::Context;
use helix_view::graphics::Rect;
use helix_view::theme::{Style, Theme};
use helix_view::{Document, Editor, ViewId};
use tui::buffer::Buffer as Surface;

use super::{Hover, Source, StudioEvent};
use engine::{Engine, Focus, Signals};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Paint {
    Chip,
    Keys,
    Label,
}

struct Cell {
    parts: Vec<(String, Paint)>,
}

impl Cell {
    fn width(&self) -> usize {
        self.parts.iter().map(|(text, _)| text.width()).sum()
    }
}

pub struct Recommender {
    engine: Engine,
    focus: Focus,
    sections: usize,
    rows: usize,
    grid: bool,
    separator: String,
}

impl Recommender {
    pub fn new(settings: &toml::Table) -> Self {
        let number = |key: &str, fallback: i64, high: i64| {
            settings
                .get(key)
                .and_then(toml::Value::as_integer)
                .unwrap_or(fallback)
                .clamp(1, high) as usize
        };

        Self {
            engine: Engine::default().with_per_section(number("per-section", 6, 24)),
            focus: Focus::Editor,
            sections: number("sections", 6, 16),
            rows: number("rows", 2, 8),
            grid: settings
                .get("grid")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false),
            separator: settings
                .get("separator")
                .and_then(toml::Value::as_str)
                .unwrap_or("   ")
                .to_string(),
        }
    }

    fn signals(&self, editor: &Editor) -> Signals {
        let view = editor.tree.try_get(editor.tree.focus);
        let doc = view.and_then(|view| editor.document(view.doc));
        let shape = match (view, doc) {
            (Some(view), Some(doc)) => shape(doc, view.id),
            _ => (false, false, false),
        };

        Signals {
            mode: editor.mode(),
            focus: self.focus,
            pending: super::keyboard_owner().or_else(|| {
                editor
                    .autoinfo
                    .as_ref()
                    .map(|info| info.title.to_lowercase())
            }),
            modified: doc.is_some_and(|doc| doc.is_modified()),
            lsp: doc.is_some_and(|doc| doc.language_servers().next().is_some()),
            diagnostics: doc.is_some_and(|doc| !doc.diagnostics().is_empty()),
            selection: shape.0,
            linewise: shape.1,
            multiline: shape.2,
            multi_cursor: match (view, doc) {
                (Some(view), Some(doc)) => doc.selection(view.id).len() > 1,
                _ => false,
            },
            multi_buffer: editor.documents().count() > 1,
            splits: editor.tree.views().count() > 1,
        }
    }

    fn pinned(&self, editor: &Editor) -> Vec<Cell> {
        self.build(self.engine.pinned(&self.signals(editor)), 0)
    }

    fn cells(&self, editor: &Editor) -> Vec<Cell> {
        let sections = self.engine.suggest(&self.signals(editor), self.sections);

        let pad = match self.grid {
            true => sections
                .iter()
                .flat_map(|section| section.items.iter())
                .map(|item| item.keys.width() + item.label.width() + 2)
                .max()
                .unwrap_or(0),
            false => 0,
        };

        self.build(sections, pad)
    }

    fn build(&self, sections: Vec<engine::Section>, pad: usize) -> Vec<Cell> {
        let mut cells = Vec::new();

        for section in sections {
            let mut chip = Some(format!(" {} ", section.label));

            for item in section.items {
                let used = item.keys.width() + item.label.width() + 2;
                let padding = " ".repeat(pad.saturating_sub(used));

                let mut parts = Vec::with_capacity(3);
                parts.extend(chip.take().map(|chip| (chip, Paint::Chip)));
                parts.push((format!(" {}", item.keys), Paint::Keys));
                parts.push((
                    format!(" {}{}{}", item.label, padding, self.separator),
                    Paint::Label,
                ));

                cells.push(Cell { parts });
            }
        }

        cells
    }
}

fn pack(cells: &[Cell], width: usize, rows: usize, reserve: usize) -> Vec<Vec<usize>> {
    if width == 0 || rows == 0 {
        return Vec::new();
    }

    let limit = |row: usize| match row + 1 == rows {
        true => width.saturating_sub(reserve),
        false => width,
    };

    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut line: Vec<usize> = Vec::new();
    let mut used = 0usize;

    for (index, cell) in cells.iter().enumerate() {
        let size = cell.width();
        if size > limit(lines.len()) {
            continue;
        }

        if used + size > limit(lines.len()) {
            if lines.len() + 1 == rows {
                break;
            }
            lines.push(std::mem::take(&mut line));
            used = 0;
        }

        used += size;
        line.push(index);
    }

    if !line.is_empty() {
        lines.push(line);
    }

    lines
}

fn fit(cells: &[Cell], width: usize, rows: usize, reserve: usize) -> Vec<Vec<usize>> {
    let mut budget = rows;

    loop {
        let lines = pack(cells, width, budget, reserve);
        if budget <= 1 || lines.len() >= budget {
            return lines;
        }
        budget = lines.len().max(1);
    }
}

impl Source for Recommender {
    fn size(&self, editor: &Editor, available: (u16, u16)) -> (u16, u16) {
        let reserve: usize = self.pinned(editor).iter().map(Cell::width).sum();
        let lines = fit(
            &self.cells(editor),
            available.0 as usize,
            self.rows.min(available.1.max(1) as usize),
            reserve,
        );

        (available.0, lines.len().max(1) as u16)
    }

    fn observe(&mut self, event: &StudioEvent, _editor: &mut Editor) {
        match event {
            StudioEvent::Command { name, .. } => self.engine.record(name),
            StudioEvent::Hover(Hover::Tab(_)) => self.focus = Focus::Tab,
            StudioEvent::Hover(Hover::None) | StudioEvent::ModeChanged { .. } => {
                self.focus = Focus::Editor
            }
            _ => {}
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        let pinned = self.pinned(cx.editor);
        let reserve: usize = pinned.iter().map(Cell::width).sum();

        let cells = self.cells(cx.editor);
        let rows = self.rows.min(area.height.max(1) as usize);
        let lines = fit(&cells, area.width as usize, rows, reserve);

        let theme = &cx.editor.theme;
        let palette = Palette {
            chip: theme
                .try_get("ui.statusline.active")
                .unwrap_or_else(|| theme.get("ui.statusline")),
            keys: foreground(theme, "keyword"),
            label: foreground(theme, "comment"),
        };

        for (index, line) in lines.iter().enumerate() {
            let y = area.y + index as u16;
            if y >= area.bottom() {
                break;
            }
            paint(surface, area.x, y, area, palette, line.iter().map(|index| &cells[*index]));
        }

        if !pinned.is_empty() {
            let y = area.y + (lines.len().max(1) as u16).saturating_sub(1);
            let x = area.right().saturating_sub(reserve as u16).max(area.x);
            paint(surface, x, y, area, palette, pinned.iter());
        }
    }
}

#[derive(Clone, Copy)]
struct Palette {
    chip: Style,
    keys: Style,
    label: Style,
}

fn paint<'a>(
    surface: &mut Surface,
    mut x: u16,
    y: u16,
    area: Rect,
    palette: Palette,
    cells: impl Iterator<Item = &'a Cell>,
) {
    for cell in cells {
        for (text, kind) in &cell.parts {
            let style = match kind {
                Paint::Chip => palette.chip,
                Paint::Keys => palette.keys,
                Paint::Label => palette.label,
            };
            x = write(surface, x, y, area, text, style);
        }
    }
}

fn shape(doc: &Document, view: ViewId) -> (bool, bool, bool) {
    let text = doc.text().slice(..);
    let primary = doc.selection(view).primary();
    let (from, to) = (primary.from(), primary.to());

    if to.saturating_sub(from) <= 1 || to > text.len_chars() {
        return (false, false, false);
    }

    let first = text.char_to_line(from);
    let last = text.char_to_line(to - 1);
    let linewise = from == text.line_to_char(first)
        && (to == text.len_chars() || to == text.line_to_char(last + 1));

    (true, linewise, last > first)
}

fn foreground(theme: &Theme, key: &str) -> Style {
    let color = theme
        .try_get(key)
        .and_then(|style| style.fg)
        .or_else(|| theme.get("ui.text").fg);

    match color {
        Some(color) => Style::default().fg(color),
        None => Style::default(),
    }
}

fn write(surface: &mut Surface, x: u16, y: u16, area: Rect, text: &str, style: Style) -> u16 {
    if x >= area.right() {
        return x;
    }

    surface
        .set_stringn(x, y, text, (area.right() - x) as usize, style)
        .0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(text: &str) -> Cell {
        Cell {
            parts: vec![
                (format!(" {text}"), Paint::Keys),
                (format!(" {text} label  "), Paint::Label),
            ],
        }
    }

    fn cells(count: usize) -> Vec<Cell> {
        (0..count).map(|index| cell(&format!("k{index}"))).collect()
    }

    fn span(cells: &[Cell], line: &[usize]) -> usize {
        line.iter().map(|index| cells[*index].width()).sum()
    }

    fn widest(cells: &[Cell], lines: &[Vec<usize>]) -> usize {
        lines.iter().map(|line| span(cells, line)).max().unwrap_or(0)
    }

    #[test]
    fn everything_on_one_row_when_it_fits() {
        let cells = cells(3);
        let lines = pack(&cells, 200, 2, 0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 3);
    }

    #[test]
    fn spillover_wraps_onto_a_second_row() {
        let cells = cells(8);
        let lines = pack(&cells, 40, 2, 0);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].len() < 8);
    }

    #[test]
    fn no_row_ever_exceeds_the_width() {
        let width = 37;
        let cells = cells(40);
        let lines = pack(&cells, width, 4, 0);
        assert!(widest(&cells, &lines) <= width);
    }

    #[test]
    fn a_section_chip_never_ends_up_without_its_first_item() {
        let recommender = Recommender::new(&toml::Table::new());
        let sections = recommender.engine.suggest(&engine::Signals::default(), 8);
        let cells = recommender.build(sections, 0);

        assert!(cells
            .iter()
            .any(|cell| cell.parts.iter().any(|(_, kind)| *kind == Paint::Chip)));

        for cell in &cells {
            if cell.parts.iter().any(|(_, kind)| *kind == Paint::Chip) {
                assert!(
                    cell.parts.iter().any(|(_, kind)| *kind == Paint::Keys),
                    "a section chip can be wrapped away from every one of its items"
                );
            }
        }
    }

    #[test]
    fn the_row_budget_is_never_exceeded() {
        let cells = cells(200);
        assert_eq!(pack(&cells, 30, 2, 0).len(), 2);
    }

    #[test]
    fn a_single_row_budget_truncates_instead_of_wrapping() {
        let cells = cells(200);
        assert_eq!(pack(&cells, 30, 1, 0).len(), 1);
    }

    #[test]
    fn a_cell_wider_than_the_panel_is_dropped_rather_than_clipped() {
        let mut mixed = vec![cell("this-one-is-very-wide-indeed")];
        mixed.extend(cells(2));

        let lines = pack(&mixed, 20, 2, 0);
        assert!(widest(&mixed, &lines) <= 20);
        assert!(!lines.is_empty());
    }

    #[test]
    fn the_reserved_area_is_kept_clear_on_the_last_row() {
        let reserve = 30;
        let width = 60;
        let cells = cells(40);
        let lines = pack(&cells, width, 2, reserve);

        let last = span(&cells, lines.last().unwrap());
        assert!(
            last <= width - reserve,
            "last row used {last} of the {} it may use",
            width - reserve
        );
    }

    #[test]
    fn earlier_rows_still_use_the_full_width() {
        let cells = cells(40);
        let lines = pack(&cells, 60, 3, 30);
        assert!(span(&cells, &lines[0]) > 30);
    }

    #[test]
    fn a_single_row_panel_still_honours_the_reservation() {
        let cells = cells(40);
        let lines = pack(&cells, 60, 1, 30);
        assert!(span(&cells, &lines[0]) <= 30);
    }

    #[test]
    fn the_panel_never_grows_a_row_it_cannot_fill() {
        let cells = cells(2);
        assert_eq!(fit(&cells, 200, 3, 0).len(), 1);
    }

    #[test]
    fn the_reservation_follows_the_row_that_actually_ends_the_panel() {
        let cells = cells(3);
        let lines = fit(&cells, 200, 3, 160);

        assert_eq!(lines.len(), 1);
        assert!(
            span(&cells, &lines[0]) <= 40,
            "the pinned block would have been painted over the predictions"
        );
    }

    #[test]
    fn zero_width_produces_nothing_rather_than_looping() {
        let cells = cells(4);
        assert!(pack(&cells, 0, 2, 0).is_empty());
    }
}
