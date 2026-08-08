use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use helix_term::job::Callback;
use helix_term::ui::{Picker, PickerColumn};
use helix_view::{theme::Style, Editor};
use tui::text::{Span, Spans};

use crate::icons;
use crate::overlay;
use crate::tree::Tree;

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub depth: u16,
    pub is_dir: bool,
    pub expanded: bool,
}

pub struct ExplorerData {
    directory_style: Style,
}

pub type ExplorerPicker = Picker<Entry, ExplorerData>;

pub fn file_explorer(editor: &Editor, tree: Arc<Mutex<Tree>>) -> ExplorerPicker {
    let data = ExplorerData {
        directory_style: editor.theme.get("ui.text.directory"),
    };

    let (entries, cursor) = snapshot(&tree);

    let columns = [PickerColumn::new(
        "path",
        |entry: &Entry, data: &ExplorerData| {
            let indent = "  ".repeat(entry.depth as usize);
            let name = entry
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();

            if entry.is_dir {
                Spans::from(vec![
                    Span::raw(indent),
                    Span::styled(
                        format!("{} {}/", icons::directory(entry.expanded), name),
                        data.directory_style,
                    ),
                ])
                .into()
            } else {
                Spans::from(vec![
                    Span::raw(indent),
                    Span::raw(format!("  {}{}", icons::file(&entry.path), name)),
                ])
                .into()
            }
        },
    )];

    let toggle_tree = Arc::clone(&tree);

    Picker::new(
        columns,
        0,
        entries,
        data,
        move |cx, entry: &Entry, action| {
            if !entry.is_dir {
                if let Err(err) = cx.editor.open(&entry.path, action) {
                    let message = match err.source() {
                        Some(source) => format!("{}", source),
                        None => format!("unable to open \"{}\"", entry.path.display()),
                    };
                    cx.editor.set_error(message);
                }
                return;
            }

            let tree = Arc::clone(&toggle_tree);
            let path = entry.path.clone();

            let callback = Box::pin(async move {
                let call: Callback = Callback::EditorCompositor(Box::new(move |editor, compositor| {
                    if let Ok(mut tree) = tree.lock() {
                        if let Some(index) = tree.rows.iter().position(|row| row.path == path) {
                            tree.select(index);
                            tree.toggle(index);
                        }
                    }

                    compositor.remove(helix_term::ui::picker::ID);
                    let picker = file_explorer(editor, Arc::clone(&tree));
                    compositor.push(Box::new(overlay::centered(picker)));
                }));
                Ok(call)
            });

            cx.jobs.callback(callback);
        },
    )
    .with_preview(|_editor, entry| Some((entry.path.as_path().into(), None)))
    .with_initial_cursor(cursor)
}

fn snapshot(tree: &Arc<Mutex<Tree>>) -> (Vec<Entry>, u32) {
    let Ok(tree) = tree.lock() else {
        return (Vec::new(), 0);
    };

    let entries = tree
        .rows
        .iter()
        .map(|row| Entry {
            path: row.path.clone(),
            depth: row.depth,
            is_dir: row.is_dir,
            expanded: row.expanded,
        })
        .collect();

    (entries, tree.cursor as u32)
}

pub fn open(editor: &Editor, root: PathBuf) -> ExplorerPicker {
    let config = editor.config();
    let mut tree = Tree::new(
        root,
        config.file_explorer.hidden,
        config.file_explorer.git_ignore,
    );

    if let Some(path) = current_path(editor) {
        tree.reveal(&path);
    }

    file_explorer(editor, Arc::new(Mutex::new(tree)))
}

fn current_path(editor: &Editor) -> Option<PathBuf> {
    let view = editor.tree.try_get(editor.tree.focus)?;
    editor.document(view.doc)?.path().map(Path::to_path_buf)
}
