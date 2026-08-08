use std::path::PathBuf;
use std::sync::{Arc, Mutex};


use helix_loader::workspace_trust::{TrustQuery, WorkspaceTrust};
use helix_view::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Explorer,
    Git,
}

impl Mode {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "git" | "changes" => Mode::Git,
            _ => Mode::Explorer,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Explorer => " Explorer ",
            Mode::Git => " Changes ",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Explorer => "explorer",
            Mode::Git => "git",
        }
    }

    pub const ALL: [Mode; 2] = [Mode::Explorer, Mode::Git];
}

fn remembered() -> PathBuf {
    helix_loader::cache_dir().join("studio-sidebar")
}

pub fn load(fallback: Mode) -> Mode {
    std::fs::read_to_string(remembered())
        .ok()
        .map(|raw| Mode::parse(raw.trim()))
        .unwrap_or(fallback)
}

pub fn remember(mode: Mode) {
    let path = remembered();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, mode.name());
}

#[derive(Default)]
pub struct Changes {
    paths: Arc<Mutex<Vec<PathBuf>>>,
    scanned: bool,
}

impl Changes {
    pub fn scan(&mut self, editor: &Editor, root: PathBuf) {
        self.scanned = true;

        if let Ok(mut paths) = self.paths.lock() {
            paths.clear();
        }

        let trust_full = WorkspaceTrust::new((&editor.config().workspace_trust).into())
            .query(&helix_loader::find_workspace_in(&root).0, TrustQuery::Git)
            .is_trusted();

        let sink = Arc::clone(&self.paths);
        editor
            .diff_providers
            .clone()
            .for_each_changed_file(root, trust_full, move |change| match change {
                Ok(change) => match sink.lock() {
                    Ok(mut paths) => {
                        paths.push(change.path().to_path_buf());
                        true
                    }
                    Err(_) => false,
                },
                Err(_) => true,
            });
    }

    pub fn scanned(&self) -> bool {
        self.scanned
    }

    pub fn invalidate(&mut self) {
        self.scanned = false;
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.paths
            .lock()
            .map(|paths| paths.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parses_both_spellings_and_falls_back() {
        assert_eq!(Mode::parse("git"), Mode::Git);
        assert_eq!(Mode::parse("changes"), Mode::Git);
        assert_eq!(Mode::parse("explorer"), Mode::Explorer);
        assert_eq!(Mode::parse("nonsense"), Mode::Explorer);
    }

    #[test]
    fn every_mode_has_a_stable_name_that_parses_back() {
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode.name()), mode);
        }
    }
}
