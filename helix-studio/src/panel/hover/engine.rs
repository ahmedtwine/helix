use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::time::Instant;

use helix_core::syntax::config::LanguageServerFeature;
use helix_event::AsyncHook;
use helix_lsp::lsp;
use helix_term::job;
use helix_view::{DocumentId, Editor, ViewId};
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Signature,
    Documentation,
}

impl Mode {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "documentation" | "docs" => Mode::Documentation,
            _ => Mode::Signature,
        }
    }

    pub fn render(self, raw: &str) -> String {
        match self {
            Mode::Signature => signature(raw),
            Mode::Documentation => documentation(raw),
        }
    }
}

#[derive(Default)]
struct State {
    raw: Option<String>,
    doc: Option<DocumentId>,
    position: usize,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

pub fn clear() {
    if let Ok(mut state) = state().lock() {
        if state.raw.take().is_some() {
            helix_event::request_redraw();
        }
    }
}

fn show(raw: String, doc: DocumentId, position: usize) {
    if let Ok(mut state) = state().lock() {
        state.raw = Some(raw);
        state.doc = Some(doc);
        state.position = position;
    }
    helix_event::request_redraw();
}

pub fn contents_at(doc: DocumentId, position: usize) -> Option<String> {
    let state = state().lock().ok()?;

    match state.doc == Some(doc) && state.position == position {
        true => state.raw.clone(),
        false => None,
    }
}

pub enum Event {
    Moved { doc: DocumentId, view: ViewId },
    Dismiss,
}

pub struct Handler {
    delay: Duration,
    pending: Option<(DocumentId, ViewId)>,
}

impl Handler {
    pub fn spawn(delay: u64) -> Sender<Event> {
        Handler {
            delay: Duration::from_millis(delay.clamp(50, 5_000)),
            pending: None,
        }
        .spawn()
    }
}

impl AsyncHook for Handler {
    type Event = Event;

    fn handle_event(&mut self, event: Event, _timeout: Option<Instant>) -> Option<Instant> {
        clear();

        match event {
            Event::Moved { doc, view } => {
                self.pending = Some((doc, view));
                Some(Instant::now() + self.delay)
            }
            Event::Dismiss => {
                self.pending = None;
                None
            }
        }
    }

    fn finish_debounce(&mut self) {
        let Some((doc, view)) = self.pending.take() else {
            return;
        };

        job::dispatch_blocking(move |editor, _| request(editor, doc, view));
    }
}

fn request(editor: &mut Editor, doc_id: DocumentId, view_id: ViewId) {
    let Some(doc) = editor.document(doc_id) else {
        return;
    };

    if editor.tree.try_get(view_id).is_none() {
        return;
    }

    let Some(server) = doc
        .language_servers_with_feature(LanguageServerFeature::Hover)
        .next()
    else {
        return;
    };

    let offset_encoding = server.offset_encoding();
    let position = doc.position(view_id, offset_encoding);

    let Some(future) = server.text_document_hover(doc.identifier(), position, None) else {
        return;
    };

    let cursor = doc
        .selection(view_id)
        .primary()
        .cursor(doc.text().slice(..));

    tokio::spawn(async move {
        let raw = match future.await {
            Ok(Some(hover)) => flatten(hover.contents),
            _ => return,
        };

        if raw.trim().is_empty() {
            return;
        }

        job::dispatch(move |_editor, _| show(raw, doc_id, cursor)).await;
    });
}

fn flatten(contents: lsp::HoverContents) -> String {
    fn marked(string: lsp::MarkedString) -> String {
        match string {
            lsp::MarkedString::String(value) => value,
            lsp::MarkedString::LanguageString(value) => {
                format!("```{}\n{}\n```", value.language, value.value)
            }
        }
    }

    match contents {
        lsp::HoverContents::Scalar(string) => marked(string),
        lsp::HoverContents::Array(strings) => strings
            .into_iter()
            .map(marked)
            .collect::<Vec<_>>()
            .join("\n"),
        lsp::HoverContents::Markup(markup) => markup.value,
    }
}

fn signature(raw: &str) -> String {
    let mut inside = false;
    let mut lines = Vec::new();

    for line in raw.lines() {
        if line.trim_start().starts_with("```") {
            if inside {
                break;
            }
            inside = true;
            continue;
        }

        if inside {
            lines.push(line);
        }
    }

    match lines.is_empty() {
        true => raw
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .to_string(),
        false => lines.join("\n"),
    }
}

fn documentation(raw: &str) -> String {
    raw.lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = "```rust\nfn check(screen: Screen) -> Vec<Check>\n```\n\n---\n\nRuns every check.";

    #[test]
    fn signature_mode_takes_only_the_first_code_block() {
        assert_eq!(
            signature(RUST),
            "fn check(screen: Screen) -> Vec<Check>"
        );
    }

    #[test]
    fn signature_mode_keeps_multi_line_signatures_whole() {
        let raw = "```ts\nfunction f(\n  a: A,\n): B\n```\ndocs";
        assert_eq!(signature(raw), "function f(\n  a: A,\n): B");
    }

    #[test]
    fn signature_mode_falls_back_to_the_first_prose_line() {
        assert_eq!(signature("\n\njust some text\nmore"), "just some text");
    }

    #[test]
    fn documentation_mode_drops_the_fences_but_keeps_the_prose() {
        let rendered = documentation(RUST);
        assert!(rendered.contains("fn check(screen: Screen) -> Vec<Check>"));
        assert!(rendered.contains("Runs every check."));
        assert!(!rendered.contains("```"));
    }

    #[test]
    fn modes_parse_with_a_safe_fallback() {
        assert_eq!(Mode::parse("documentation"), Mode::Documentation);
        assert_eq!(Mode::parse("docs"), Mode::Documentation);
        assert_eq!(Mode::parse("signature"), Mode::Signature);
        assert_eq!(Mode::parse("nonsense"), Mode::Signature);
    }

    #[test]
    fn a_language_string_keeps_its_fence_so_signature_mode_can_find_it() {
        let contents = lsp::HoverContents::Scalar(lsp::MarkedString::LanguageString(
            lsp::LanguageString {
                language: "rust".to_string(),
                value: "fn f() -> u8".to_string(),
            },
        ));

        assert_eq!(signature(&flatten(contents)), "fn f() -> u8");
    }

    #[test]
    fn stale_contents_are_not_served_for_another_position() {
        let doc = DocumentId::default();
        show("something".to_string(), doc, 42);

        assert_eq!(contents_at(doc, 42).as_deref(), Some("something"));
        assert!(contents_at(doc, 43).is_none());

        clear();
        assert!(contents_at(doc, 42).is_none());
    }
}
