pub mod diff;
pub mod hover;
pub mod layout;
pub mod recommender;
pub mod sidebar;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;

use helix_term::compositor::{Context, Event};
use helix_view::document::Mode;
use helix_view::graphics::{Margin, Rect};
use helix_view::input::{KeyCode, KeyEvent};
use helix_view::{DocumentId, Editor};
use serde::Deserialize;
use tui::buffer::Buffer as Surface;
use tui::widgets::{Block, Widget};

use crate::chrome::Chrome;
use layout::{Anchor, Placement};

pub const DEFAULTS: &str = include_str!("../../config/studio.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    None,
    Tab(DocumentId),
}

pub enum StudioEvent<'a> {
    Command { name: &'a str, mode: Mode },
    ModeChanged { from: Mode, to: Mode },
    Hover(Hover),
    DocumentChanged,
}

pub trait Source: 'static {
    fn visible(&self, _editor: &Editor) -> bool {
        true
    }

    fn size(&self, _editor: &Editor, _available: (u16, u16)) -> (u16, u16) {
        (0, 0)
    }

    fn observe(&mut self, _event: &StudioEvent, _editor: &mut Editor) {}

    fn handle_input(&mut self, _event: &Event, _area: Rect, _cx: &mut Context) -> bool {
        false
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context);
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PanelConfig {
    pub enabled: bool,
    pub order: u8,
    pub border: bool,
    pub title: Option<String>,
    pub padding: (u16, u16),
    pub style: String,
    pub toggle: Option<String>,
    pub dismissable: bool,
    #[serde(flatten)]
    pub placement: Placement,
    #[serde(flatten)]
    pub settings: toml::Table,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            order: 0,
            border: false,
            title: None,
            padding: (0, 0),
            style: "ui.statusline".to_string(),
            toggle: None,
            dismissable: false,
            placement: Placement::default(),
            settings: toml::Table::new(),
        }
    }
}

impl PanelConfig {
    fn chrome(&self) -> (u16, u16) {
        let border = if self.border { 2 } else { 0 };
        (border + self.padding.0 * 2, border + self.padding.1 * 2)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct StudioConfig {
    pub panel: HashMap<String, PanelConfig>,
}

impl StudioConfig {
    pub fn load() -> Self {
        let user = helix_loader::config_dir().join("studio.toml");
        let merged = match fs::read_to_string(user).ok() {
            Some(user) => merge(DEFAULTS, &user),
            None => toml::from_str(DEFAULTS).ok(),
        };

        merged.unwrap_or_default()
    }
}

fn merge(defaults: &str, user: &str) -> Option<StudioConfig> {
    let defaults: toml::Value = toml::from_str(defaults).ok()?;
    let user: toml::Value = toml::from_str(user).ok()?;
    helix_loader::merge_toml_values(defaults, user, 3)
        .try_into()
        .ok()
}

pub struct Panel {
    id: String,
    config: PanelConfig,
    source: Box<dyn Source>,
    area: Rect,
    active: bool,
    toggle: Option<KeyEvent>,
}

impl Panel {
    fn render(&mut self, viewport: Rect, surface: &mut Surface, cx: &mut Context) {
        self.area = Rect::default();

        if !self.active || !self.source.visible(cx.editor) {
            return;
        }

        let anchor = anchor(self.config.placement.anchor, viewport, cx.editor);
        let (extra_width, extra_height) = self.config.chrome();
        let available = (
            viewport.width.saturating_sub(extra_width),
            viewport.height.saturating_sub(extra_height),
        );
        let (content_width, content_height) = self.source.size(cx.editor, available);
        let content = (content_width + extra_width, content_height + extra_height);

        let area = self.config.placement.resolve(viewport, anchor, content);
        if area.width <= extra_width || area.height <= extra_height {
            return;
        }

        let style = cx.editor.theme.get(&self.config.style);
        surface.clear_with(area, style);

        let inner = if self.config.border {
            let mut block = Block::bordered().border_style(style);
            if let Some(title) = &self.config.title {
                block = block.title(title.as_str());
            }
            let inner = block.inner(area);
            block.render(area, surface);
            inner
        } else {
            area
        };

        let inner = inner.inner(Margin {
            horizontal: self.config.padding.0,
            vertical: self.config.padding.1,
        });

        self.area = inner;
        self.source.render(inner, surface, cx);
    }
}

fn anchor(anchor: Anchor, viewport: Rect, editor: &Editor) -> Rect {
    match anchor {
        Anchor::Viewport => viewport,
        Anchor::Editor => {
            let bufferline = u16::from(Chrome::visible(editor));
            viewport.clip_top(bufferline).clip_bottom(1)
        }
        Anchor::Status => editor
            .tree
            .try_get(editor.tree.focus)
            .map(|view| {
                Rect::new(
                    view.area.x,
                    view.area.bottom().saturating_sub(1),
                    view.area.width,
                    1,
                )
            })
            .unwrap_or(viewport),
        Anchor::Cursor => editor
            .cursor()
            .0
            .map(|position| Rect::new(position.col as u16, position.row as u16, 1, 1))
            .unwrap_or(viewport),
    }
}

#[derive(Default)]
struct Registry {
    panels: Vec<Panel>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

fn build(id: &str, config: &PanelConfig) -> Option<Box<dyn Source>> {
    match id {
        "tips" => Some(Box::new(recommender::Recommender::new(&config.settings))),
        "sidebar" => Some(Box::new(sidebar::Sidebar::new(&config.settings))),
        "diff" => Some(Box::new(diff::Diff::new(&config.settings))),
        "hover" => Some(Box::new(hover::Hover::new(&config.settings))),
        _ => None,
    }
}

pub fn install() {
    let config = StudioConfig::load();

    let mut entries: Vec<(String, PanelConfig)> = config.panel.into_iter().collect();
    entries.sort_by(|a, b| (a.1.order, &a.0).cmp(&(b.1.order, &b.0)));

    let panels = entries
        .into_iter()
        .filter_map(|(id, config)| {
            let source = build(&id, &config)?;
            Some(Panel {
                active: config.enabled,
                toggle: config.toggle.as_deref().and_then(|key| key.parse().ok()),
                id,
                config,
                source,
                area: Rect::default(),
            })
        })
        .collect();

    REGISTRY.with_borrow_mut(|registry| registry.panels = panels);
}

pub fn hover_delay() -> u64 {
    StudioConfig::load()
        .panel
        .get("hover")
        .and_then(|config| config.settings.get("delay"))
        .and_then(toml::Value::as_integer)
        .unwrap_or(400) as u64
}

pub fn render(viewport: Rect, surface: &mut Surface, cx: &mut Context) {
    REGISTRY.with_borrow_mut(|registry| {
        for panel in registry.panels.iter_mut() {
            panel.render(viewport, surface, cx);
        }
    });
}

pub fn toggle(id: &str) -> bool {
    REGISTRY.with_borrow_mut(|registry| {
        match registry.panels.iter_mut().find(|panel| panel.id == id) {
            Some(panel) => {
                panel.active = !panel.active;
                helix_event::request_redraw();
                true
            }
            None => false,
        }
    })
}

pub fn handle_input(event: &Event, cx: &mut Context) -> bool {
    REGISTRY.with_borrow_mut(|registry| {
        if let Event::Key(key) = event {
            if let Some(panel) = registry
                .panels
                .iter_mut()
                .find(|panel| panel.toggle == Some(*key))
            {
                panel.active = !panel.active;
                helix_event::request_redraw();
                return true;
            }

            if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                let dismissed = registry
                    .panels
                    .iter_mut()
                    .filter(|panel| panel.active && panel.config.dismissable)
                    .fold(false, |_, panel| {
                        panel.active = false;
                        true
                    });

                if dismissed {
                    helix_event::request_redraw();
                    return true;
                }
            }
        }

        registry
            .panels
            .iter_mut()
            .rev()
            .filter(|panel| panel.active)
            .any(|panel| panel.source.handle_input(event, panel.area, cx))
    })
}

pub fn observe(event: &StudioEvent, editor: &mut Editor) {
    REGISTRY.with_borrow_mut(|registry| {
        for panel in registry.panels.iter_mut() {
            panel.source.observe(event, editor);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_defaults_parse() {
        let config: StudioConfig = toml::from_str(DEFAULTS).unwrap();
        assert!(config.panel.contains_key("tips"));
        assert!(config.panel.contains_key("sidebar"));
    }

    #[test]
    fn user_config_overrides_one_key_without_dropping_the_rest() {
        let user = "[panel.tips]\nside = \"top\"\n";
        let config = merge(DEFAULTS, user).unwrap();
        let tips = &config.panel["tips"];

        assert_eq!(tips.placement.side, layout::Side::Top);
        assert!(tips.enabled);
    }

    #[test]
    fn unknown_panel_ids_are_ignored_rather_than_fatal() {
        let config = merge(DEFAULTS, "[panel.nope]\nenabled = true\n").unwrap();
        assert!(build("nope", &config.panel["nope"]).is_none());
    }
}
