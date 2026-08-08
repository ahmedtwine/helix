use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

pub struct Row {
    pub path: PathBuf,
    pub depth: u16,
    pub is_dir: bool,
    pub expanded: bool,
}

impl Row {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

pub struct Tree {
    pub root: PathBuf,
    pub rows: Vec<Row>,
    pub cursor: usize,
    hidden: bool,
    git_ignore: bool,
}

impl Tree {
    pub fn new(root: PathBuf, hidden: bool, git_ignore: bool) -> Self {
        let rows = read_dir(&root, 0, hidden, git_ignore);
        Self {
            root,
            rows,
            cursor: 0,
            hidden,
            git_ignore,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    pub fn move_by(&mut self, amount: usize, forward: bool) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        self.cursor = if forward {
            self.cursor.saturating_add(amount).min(last)
        } else {
            self.cursor.saturating_sub(amount)
        };
    }

    pub fn to_start(&mut self) {
        self.cursor = 0;
    }

    pub fn to_end(&mut self) {
        self.cursor = self.rows.len().saturating_sub(1);
    }

    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.cursor = index;
        }
    }

    pub fn expand(&mut self, index: usize) {
        let (path, depth) = match self.rows.get(index) {
            Some(row) if row.is_dir && !row.expanded => (row.path.clone(), row.depth + 1),
            _ => return,
        };

        let children = read_dir(&path, depth, self.hidden, self.git_ignore);
        self.rows[index].expanded = true;
        self.rows.splice(index + 1..index + 1, children);
    }

    pub fn collapse(&mut self, index: usize) {
        let depth = match self.rows.get(index) {
            Some(row) if row.is_dir && row.expanded => row.depth,
            _ => return,
        };

        let end = self.rows[index + 1..]
            .iter()
            .position(|row| row.depth <= depth)
            .map(|offset| index + 1 + offset)
            .unwrap_or(self.rows.len());

        self.rows[index].expanded = false;
        self.rows.drain(index + 1..end);
    }

    pub fn toggle(&mut self, index: usize) {
        match self.rows.get(index) {
            Some(row) if row.is_dir && row.expanded => self.collapse(index),
            Some(row) if row.is_dir => self.expand(index),
            _ => {}
        }
    }

    pub fn collapse_or_parent(&mut self) {
        let Some(row) = self.rows.get(self.cursor) else {
            return;
        };

        if row.is_dir && row.expanded {
            self.collapse(self.cursor);
            return;
        }

        let depth = row.depth;
        if depth == 0 {
            return;
        }

        if let Some(parent) = self.rows[..self.cursor]
            .iter()
            .rposition(|row| row.depth < depth)
        {
            self.cursor = parent;
        }
    }

    pub fn collapse_all(&mut self) {
        self.rows.retain(|row| row.depth == 0);
        for row in self.rows.iter_mut() {
            row.expanded = false;
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    pub fn reveal(&mut self, path: &Path) {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return;
        };

        let mut walked = self.root.clone();
        for component in relative.components() {
            walked.push(component);

            let Some(index) = self.rows.iter().position(|row| row.path == walked) else {
                return;
            };

            self.cursor = index;
            if self.rows[index].is_dir {
                self.expand(index);
            }
        }
    }

    pub fn reload(&mut self) {
        self.rows = read_dir(&self.root, 0, self.hidden, self.git_ignore);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }
}

fn read_dir(path: &Path, depth: u16, hidden: bool, git_ignore: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = WalkBuilder::new(path)
        .hidden(!hidden)
        .git_ignore(git_ignore)
        .git_global(git_ignore)
        .git_exclude(git_ignore)
        .follow_links(false)
        .max_depth(Some(1))
        .build()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let entry_path = entry.path();
            if entry_path == path {
                return None;
            }
            Some(Row {
                is_dir: entry_path.is_dir(),
                path: entry_path.to_path_buf(),
                depth,
                expanded: false,
            })
        })
        .collect();

    rows.sort_by(|a, b| (!a.is_dir, a.name()).cmp(&(!b.is_dir, b.name())));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "").unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        fs::create_dir(dir.path().join("src/ui")).unwrap();
        fs::write(dir.path().join("src/ui/mod.rs"), "").unwrap();
        fs::write(dir.path().join("README.md"), "").unwrap();
        dir
    }

    fn tree(root: &Path) -> Tree {
        Tree::new(root.to_path_buf(), false, false)
    }

    #[test]
    fn directories_sort_before_files() {
        let dir = fixture();
        let tree = tree(dir.path());
        assert_eq!(tree.rows[0].name(), "src");
        assert!(tree.rows[0].is_dir);
        assert_eq!(tree.rows[1].name(), "README.md");
    }

    #[test]
    fn expand_splices_children_after_parent() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        assert_eq!(tree.len(), 2);

        tree.expand(0);

        assert_eq!(tree.len(), 5);
        assert!(tree.rows[0].expanded);
        assert_eq!(tree.rows[1].name(), "ui");
        assert_eq!(tree.rows[1].depth, 1);
        assert_eq!(tree.rows[4].name(), "README.md");
        assert_eq!(tree.rows[4].depth, 0);
    }

    #[test]
    fn collapse_drops_the_whole_subtree() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        tree.expand(0);
        tree.expand(1);
        assert_eq!(tree.len(), 6);

        tree.collapse(0);

        assert_eq!(tree.len(), 2);
        assert!(!tree.rows[0].expanded);
        assert_eq!(tree.rows[1].name(), "README.md");
    }

    #[test]
    fn nested_expand_keeps_depth_order() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        tree.expand(0);
        tree.expand(1);

        let depths: Vec<u16> = tree.rows.iter().map(|row| row.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 1, 1, 0]);
    }

    #[test]
    fn collapse_or_parent_walks_up() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        tree.expand(0);
        tree.select(3);

        tree.collapse_or_parent();

        assert_eq!(tree.cursor, 0);
    }

    #[test]
    fn toggle_is_idempotent() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        let before = tree.len();

        tree.toggle(0);
        tree.toggle(0);

        assert_eq!(tree.len(), before);
        assert!(!tree.rows[0].expanded);
    }

    #[test]
    fn reveal_expands_every_ancestor_and_lands_on_the_file() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        assert_eq!(tree.len(), 2);

        tree.reveal(&dir.path().join("src/ui/mod.rs"));

        assert_eq!(tree.selected().unwrap().name(), "mod.rs");
        assert!(tree.rows[0].expanded);
        assert!(tree.rows[1].expanded);
        assert_eq!(tree.rows[1].name(), "ui");
    }

    #[test]
    fn reveal_leaves_unrelated_directories_collapsed() {
        let dir = fixture();
        fs::create_dir(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/guide.md"), "").unwrap();
        let mut tree = tree(dir.path());

        tree.reveal(&dir.path().join("src/main.rs"));

        let docs = tree.rows.iter().find(|row| row.name() == "docs").unwrap();
        assert!(!docs.expanded);
    }

    #[test]
    fn reveal_ignores_paths_outside_the_root() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        let before = tree.len();

        tree.reveal(Path::new("/somewhere/else/main.rs"));

        assert_eq!(tree.len(), before);
        assert_eq!(tree.cursor, 0);
    }

    #[test]
    fn cursor_stays_in_bounds() {
        let dir = fixture();
        let mut tree = tree(dir.path());
        tree.move_by(999, true);
        assert_eq!(tree.cursor, tree.len() - 1);
        tree.move_by(999, false);
        assert_eq!(tree.cursor, 0);
    }
}
