use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use helix_view::editor::Action;
use helix_view::Editor;
use serde::{Deserialize, Serialize};

pub const MAX_FILES: usize = 64;

#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    workspace: BTreeMap<String, Workspace>,
}

#[derive(Default, Serialize, Deserialize)]
struct Workspace {
    #[serde(default)]
    files: Vec<PathBuf>,
}

fn store_path() -> PathBuf {
    helix_loader::cache_dir().join("studio-session.toml")
}

fn read_store() -> Store {
    fs::read_to_string(store_path())
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

fn key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

pub fn open_files(editor: &Editor, root: &Path) -> Vec<PathBuf> {
    editor
        .documents()
        .filter_map(|doc| doc.path())
        .filter(|path| path.starts_with(root))
        .take(MAX_FILES)
        .map(|path| path.to_path_buf())
        .collect()
}

pub fn save(editor: &Editor, root: &Path) {
    let files = open_files(editor, root);
    let mut store = read_store();

    if files.is_empty() {
        store.workspace.remove(&key(root));
    } else {
        store.workspace.insert(key(root), Workspace { files });
    }

    let Ok(text) = toml::to_string(&store) else {
        return;
    };

    let path = store_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, text);
}

pub fn restorable(root: &Path) -> Vec<PathBuf> {
    read_store()
        .workspace
        .remove(&key(root))
        .map(|workspace| {
            workspace
                .files
                .into_iter()
                .filter(|file| file.is_file())
                .collect()
        })
        .unwrap_or_default()
}

pub fn restore(editor: &mut Editor, root: &Path) -> bool {
    let mut first = None;

    for file in restorable(root) {
        if let Ok(id) = editor.open(&file, Action::Load) {
            first.get_or_insert(id);
        }
    }

    match first {
        Some(id) => {
            editor.switch(id, Action::Replace);
            true
        }
        None => false,
    }
}
