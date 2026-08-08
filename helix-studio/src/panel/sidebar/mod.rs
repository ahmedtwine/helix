pub mod modes;

use std::path::{Path, PathBuf};

use helix_term::compositor::{Context, Event};
use helix_view::editor::Action;
use helix_view::graphics::Rect;
use helix_view::input::{MouseButton, MouseEventKind};
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use super::{Source, StudioEvent};
use crate::icons;
use crate::tree::Tree;
use modes::{Changes, Mode};

const SCROLL: usize = 3;

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

    fn activate(&mut self, index: usize, cx: &mut Context) {
        match self.mode {
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
                } else {
                    open(cx.editor, &path);
                }
            }
            Mode::Git => {
                if let Some(path) = self.changes.paths().get(index) {
                    open(cx.editor, path);
                }
            }
        }
    }
}

impl Source for Sidebar {
    fn observe(&mut self, event: &StudioEvent, _editor: &mut Editor) {
        if matches!(event, StudioEvent::DocumentChanged) {
            self.changes.invalidate();
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
                    self.activate(index, cx);
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
                usize::MAX,
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
    helix_stdx::env::current_working_dir()
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

fn open(editor: &mut Editor, path: &Path) {
    if let Err(err) = editor.open(path, Action::Replace) {
        editor.set_error(format!("unable to open {}: {}", path.display(), err));
    }
}
