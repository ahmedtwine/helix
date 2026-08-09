pub mod modes;

use std::path::{Path, PathBuf};

use helix_term::compositor::{Context, Event};
use helix_view::editor::Action;
use helix_view::graphics::Rect;
use helix_view::input::{KeyEvent, MouseButton, MouseEventKind};
use helix_view::keyboard::{KeyCode, KeyModifiers};
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use super::{Source, StudioEvent};
use crate::icons;
use crate::tree::Tree;
use modes::{Changes, Mode};

const SCROLL: usize = 3;
const PAGE: usize = 10;

struct Tab {
    mode: Mode,
    start: u16,
    end: u16,
}

pub struct Sidebar {
    mode: Mode,
    tree: Option<Tree>,
    changes: Changes,
    offset: usize,
    cursor: usize,
    closing: bool,
    tabs: Vec<Tab>,
}

impl Sidebar {
    pub fn new(settings: &toml::Table) -> Self {
        let fallback = settings
            .get("mode")
            .and_then(toml::Value::as_str)
            .map(Mode::parse)
            .unwrap_or(Mode::Explorer);

        Self {
            mode: modes::load(fallback),
            tree: None,
            changes: Changes::default(),
            offset: 0,
            cursor: 0,
            closing: false,
            tabs: Vec::new(),
        }
    }

    fn switch(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }

        self.mode = mode;
        self.offset = 0;
        modes::remember(mode);
    }

    fn cycle(&mut self) {
        let next = Mode::ALL
            .iter()
            .cycle()
            .skip_while(|mode| **mode != self.mode)
            .nth(1)
            .copied()
            .unwrap_or(self.mode);

        self.switch(next);
    }

    fn cursor(&self) -> usize {
        match self.mode {
            Mode::Explorer => self.tree.as_ref().map_or(0, |tree| tree.cursor),
            Mode::Git => self.cursor,
        }
    }

    fn last(&self) -> usize {
        match self.mode {
            Mode::Explorer => self.tree.as_ref().map_or(0, Tree::len),
            Mode::Git => self.changes.paths().len(),
        }
        .saturating_sub(1)
    }

    fn step(&mut self, amount: usize, forward: bool) {
        match self.mode {
            Mode::Explorer => {
                if let Some(tree) = self.tree.as_mut() {
                    tree.move_by(amount, forward);
                }
            }
            Mode::Git => {
                self.cursor = match forward {
                    true => self.cursor.saturating_add(amount).min(self.last()),
                    false => self.cursor.saturating_sub(amount),
                }
            }
        }
    }

    fn jump(&mut self, end: bool) {
        match self.mode {
            Mode::Explorer => {
                if let Some(tree) = self.tree.as_mut() {
                    match end {
                        true => tree.to_end(),
                        false => tree.to_start(),
                    }
                }
            }
            Mode::Git => {
                self.cursor = match end {
                    true => self.last(),
                    false => 0,
                }
            }
        }
    }

    fn collapse(&mut self) {
        if let Some(tree) = self.tree.as_mut() {
            tree.collapse_or_parent();
        }
    }

    fn enter(&mut self, action: Action, cx: &mut Context) {
        let index = self.cursor();
        self.activate(index, action, cx);
    }

    fn key(&mut self, key: KeyEvent, cx: &mut Context) -> bool {
        self.ensure(cx.editor);

        let plain = key.modifiers.is_empty();
        let ctrl = key.modifiers == KeyModifiers::CONTROL;

        match key.code {
            KeyCode::Down => self.step(1, true),
            KeyCode::Up => self.step(1, false),
            KeyCode::Char('j') if plain => self.step(1, true),
            KeyCode::Char('k') if plain => self.step(1, false),
            KeyCode::Char('d') if ctrl => self.step(PAGE, true),
            KeyCode::Char('u') if ctrl => self.step(PAGE, false),
            KeyCode::Char('g') if plain => self.jump(false),
            KeyCode::Char('G') if plain => self.jump(true),
            KeyCode::Left => self.collapse(),
            KeyCode::Char('h') if plain => self.collapse(),
            KeyCode::Tab => self.cycle(),
            KeyCode::Char('v') if ctrl => self.enter(Action::VerticalSplit, cx),
            KeyCode::Char('s') if ctrl => self.enter(Action::HorizontalSplit, cx),
            KeyCode::Enter | KeyCode::Right => self.enter(Action::Replace, cx),
            KeyCode::Char('l') if plain => self.enter(Action::Replace, cx),
            _ => return false,
        }

        helix_event::request_redraw();
        true
    }

    fn ensure(&mut self, editor: &Editor) {
        if self.tree.is_none() {
            let config = editor.config();
            let mut tree = Tree::new(
                root(),
                config.file_explorer.hidden,
                config.file_explorer.git_ignore,
            );

            if let Some(path) = current_path(editor) {
                tree.reveal(&path);
            }

            self.tree = Some(tree);
        }

        if self.mode == Mode::Git && !self.changes.scanned() {
            self.changes.scan(editor, root());
        }
    }

    fn window(&mut self, height: usize, cursor: usize, len: usize) -> usize {
        if height == 0 {
            return 0;
        }

        if cursor < self.offset {
            self.offset = cursor;
        } else if cursor >= self.offset + height {
            self.offset = cursor + 1 - height;
        }

        self.offset = self.offset.min(len.saturating_sub(height));
        self.offset
    }

    fn activate(&mut self, index: usize, action: Action, cx: &mut Context) {
        let path = match self.mode {
            Mode::Explorer => {
                let Some(tree) = self.tree.as_mut() else {
                    return;
                };
                let Some((path, is_dir)) = tree
                    .rows
                    .get(index)
                    .map(|row| (row.path.clone(), row.is_dir))
                else {
                    return;
                };

                tree.select(index);

                if is_dir {
                    tree.toggle(index);
                    return;
                }

                path
            }
            Mode::Git => match self.changes.paths().get(index) {
                Some(path) => path.clone(),
                None => return,
            },
        };

        open(cx.editor, &path, action);
        self.closing = true;
    }
}

impl Source for Sidebar {
    fn observe(&mut self, event: &StudioEvent, _editor: &mut Editor) {
        if matches!(event, StudioEvent::DocumentChanged) {
            self.changes.invalidate();
        }
    }

    fn dismissed(&mut self) -> bool {
        std::mem::take(&mut self.closing)
    }

    fn handle_input(&mut self, event: &Event, area: Rect, cx: &mut Context) -> bool {
        let mouse = match event {
            Event::Key(key) => return self.key(*key, cx),
            Event::Mouse(mouse) => mouse,
            _ => return false,
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
                true
            }
            MouseEventKind::ScrollUp => {
                self.offset = self.offset.saturating_sub(SCROLL);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse.row == area.y {
                    if let Some(mode) = self
                        .tabs
                        .iter()
                        .find(|tab| mouse.column >= tab.start && mouse.column < tab.end)
                        .map(|tab| tab.mode)
                    {
                        self.switch(mode);
                    }
                } else {
                    let body = area.clip_top(1);
                    let index = self.offset + (mouse.row - body.y) as usize;
                    self.activate(index, Action::Replace, cx);
                }
                helix_event::request_redraw();
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        self.ensure(cx.editor);

        let theme = &cx.editor.theme;
        let header = theme
            .try_get("ui.bufferline.active")
            .unwrap_or_else(|| theme.get("ui.statusline"));
        let directory = theme.get("ui.text.directory");
        let text = theme.get("ui.text");
        let selection = theme.get("ui.selection");

        let inactive = theme
            .try_get("ui.bufferline")
            .unwrap_or_else(|| theme.get("ui.statusline.inactive"));

        self.tabs.clear();
        let mut x = area.x;
        for mode in Mode::ALL {
            let style = match mode == self.mode {
                true => header,
                false => inactive,
            };

            let start = x;
            let end = surface
                .set_stringn(x, area.y, mode.label(), area.right().saturating_sub(x) as usize, style)
                .0;

            self.tabs.push(Tab { mode, start, end });
            x = end;
        }

        let body = area.clip_top(1);
        let height = body.height as usize;
        let width = body.width as usize;

        let (rows, cursor) = match self.mode {
            Mode::Explorer => match &self.tree {
                Some(tree) => (
                    tree.rows
                        .iter()
                        .map(|row| {
                            (
                                format!(
                                    "{}{} {}",
                                    "  ".repeat(row.depth as usize),
                                    match row.is_dir {
                                        true => icons::directory(row.expanded).to_string(),
                                        false => icons::file(&row.path).to_string(),
                                    },
                                    row.name()
                                ),
                                row.is_dir,
                            )
                        })
                        .collect::<Vec<_>>(),
                    tree.cursor,
                ),
                None => (Vec::new(), 0),
            },
            Mode::Git => (
                self.changes
                    .paths()
                    .iter()
                    .map(|path| (label(path), false))
                    .collect(),
                self.cursor,
            ),
        };

        let offset = self.window(height, cursor.min(rows.len()), rows.len());

        for (line, (row, is_dir)) in rows.iter().skip(offset).take(height).enumerate() {
            let y = body.y + line as u16;
            let index = offset + line;

            let style = match (index == cursor, *is_dir) {
                (true, _) => selection,
                (false, true) => directory,
                (false, false) => text,
            };

            surface.set_stringn(body.x, y, row, width, style);
        }
    }
}

fn root() -> PathBuf {
    crate::workspace_root()
}

fn current_path(editor: &Editor) -> Option<PathBuf> {
    let view = editor.tree.try_get(editor.tree.focus)?;
    editor.document(view.doc)?.path().map(Path::to_path_buf)
}

fn label(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn open(editor: &mut Editor, path: &Path, action: Action) {
    if let Err(err) = editor.open(path, action) {
        editor.set_error(format!("unable to open {}: {}", path.display(), err));
    }
}
