use std::collections::{HashMap, VecDeque};

use helix_view::document::Mode;
use serde::Deserialize;

pub const CATALOG: &str = include_str!("../../../config/commands.toml");

const HISTORY: usize = 8;
const SCALE: u32 = 4;
const EDGE_BONUS: u32 = 40;
const GROUP_BONUS: u32 = 96;
const LEARNED_WEIGHT: u32 = 4;
const LEARNED_CAP: u32 = 24;
const REPEAT_PENALTY: u32 = 60;
const PER_SECTION: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Tab,
    Picker,
}

#[derive(Debug, Clone)]
pub struct Signals {
    pub mode: Mode,
    pub focus: Focus,
    pub pending: Option<String>,
    pub modified: bool,
    pub lsp: bool,
    pub diagnostics: bool,
    pub selection: bool,
    pub linewise: bool,
    pub multiline: bool,
    pub multi_cursor: bool,
    pub multi_buffer: bool,
    pub splits: bool,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            focus: Focus::Editor,
            pending: None,
            modified: false,
            lsp: false,
            diagnostics: false,
            selection: false,
            linewise: false,
            multiline: false,
            multi_cursor: false,
            multi_buffer: false,
            splits: false,
        }
    }
}

impl Signals {
    fn holds(&self, token: &str) -> bool {
        if let Some(negated) = token.strip_prefix('!') {
            return !self.holds(negated);
        }

        if let Some(prefix) = token.strip_prefix("pending:") {
            return self.pending.as_deref() == Some(prefix);
        }

        match token {
            "normal" => self.mode == Mode::Normal,
            "insert" => self.mode == Mode::Insert,
            "select" => self.mode == Mode::Select,
            "modified" => self.modified,
            "lsp" => self.lsp,
            "diagnostics" => self.diagnostics,
            "selection" => self.selection,
            "linewise" => self.linewise,
            "multiline" => self.multiline,
            "multi-cursor" => self.multi_cursor,
            "buffers" => self.multi_buffer,
            "splits" => self.splits,
            "focus:editor" => self.focus == Focus::Editor,
            "focus:tab" => self.focus == Focus::Tab,
            "focus:picker" => self.focus == Focus::Picker,
            _ => false,
        }
    }

    fn passes(&self, gates: &[String]) -> bool {
        gates.iter().all(|gate| self.holds(gate))
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub group: Vec<Group>,
}

#[derive(Debug, Deserialize)]
pub struct Group {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub base: u32,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub when: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub keys: String,
    pub label: String,
    #[serde(default)]
    pub cmd: String,
    pub weight: Option<u32>,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default)]
    pub when: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
}

fn declares_pending(group: &Group) -> bool {
    group.when.iter().any(|gate| gate.starts_with("pending:"))
}

#[derive(Debug, PartialEq, Eq)]
pub struct Item {
    pub keys: String,
    pub label: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Section {
    pub label: String,
    pub items: Vec<Item>,
}

pub struct Engine {
    catalog: Catalog,
    history: VecDeque<String>,
    edges: HashMap<String, HashMap<String, u32>>,
    per_section: usize,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(CATALOG)
    }
}

impl Engine {
    pub fn new(catalog: &str) -> Self {
        Self {
            catalog: toml::from_str(catalog).unwrap_or_default(),
            history: VecDeque::with_capacity(HISTORY),
            edges: HashMap::new(),
            per_section: PER_SECTION,
        }
    }

    pub fn with_per_section(mut self, per_section: usize) -> Self {
        self.per_section = per_section.max(1);
        self
    }

    pub fn record(&mut self, command: &str) {
        if command.is_empty() {
            return;
        }

        if let Some(previous) = self.history.front() {
            *self
                .edges
                .entry(previous.clone())
                .or_default()
                .entry(command.to_string())
                .or_default() += 1;
        }

        self.history.push_front(command.to_string());
        self.history.truncate(HISTORY);
    }

    pub fn last(&self) -> Option<&str> {
        self.history.front().map(String::as_str)
    }

    pub fn suggest(&self, signals: &Signals, sections: usize) -> Vec<Section> {
        let pending = self.ranked(signals, sections, true);

        match pending.is_empty() {
            true => self.ranked(signals, sections, false),
            false => pending,
        }
    }

    fn ranked(&self, signals: &Signals, sections: usize, pending: bool) -> Vec<Section> {
        if pending && signals.pending.is_none() {
            return Vec::new();
        }

        let mut groups: Vec<(u32, &Group)> = self
            .catalog
            .group
            .iter()
            .filter(|group| !group.pinned)
            .filter(|group| declares_pending(group) == pending)
            .filter(|group| signals.passes(&group.when))
            .map(|group| (self.score_group(group), group))
            .collect();

        groups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));

        groups
            .into_iter()
            .take(sections)
            .filter_map(|(_, group)| self.section(group, signals))
            .collect()
    }

    pub fn pinned(&self, signals: &Signals) -> Vec<Section> {
        if signals.pending.is_some() {
            return Vec::new();
        }

        self.catalog
            .group
            .iter()
            .filter(|group| group.pinned)
            .filter(|group| signals.passes(&group.when))
            .filter_map(|group| self.section(group, signals))
            .collect()
    }

    fn section(&self, group: &Group, signals: &Signals) -> Option<Section> {
        let mut entries: Vec<(u32, &Entry)> = group
            .entries
            .iter()
            .filter(|entry| signals.passes(&entry.when))
            .map(|entry| (self.score_entry(group, entry), entry))
            .collect();

        if entries.is_empty() {
            return None;
        }

        entries.sort_by(|a, b| b.0.cmp(&a.0));

        Some(Section {
            label: group.label.clone(),
            items: entries
                .into_iter()
                .take(self.per_section)
                .map(|(_, entry)| Item {
                    keys: entry.keys.clone(),
                    label: entry.label.clone(),
                })
                .collect(),
        })
    }

    fn score_group(&self, group: &Group) -> u32 {
        group.base * SCALE + self.recency(&group.after, GROUP_BONUS)
    }

    fn score_entry(&self, group: &Group, entry: &Entry) -> u32 {
        let base = entry.weight.unwrap_or(group.base) * SCALE;
        let score = base + self.recency(&entry.after, EDGE_BONUS) + self.learned(&entry.cmd);

        match self.last() {
            Some(last) if last == entry.cmd && !entry.repeat => {
                score.saturating_sub(REPEAT_PENALTY)
            }
            _ => score,
        }
    }

    fn recency(&self, after: &[String], bonus: u32) -> u32 {
        self.history
            .iter()
            .enumerate()
            .filter(|(_, command)| after.iter().any(|target| target == *command))
            .map(|(distance, _)| bonus / (distance as u32 + 1))
            .max()
            .unwrap_or(0)
    }

    fn learned(&self, command: &str) -> u32 {
        if command.is_empty() {
            return 0;
        }

        self.history
            .front()
            .and_then(|previous| self.edges.get(previous))
            .and_then(|targets| targets.get(command))
            .map(|count| (count * LEARNED_WEIGHT).min(LEARNED_CAP))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        Engine::default()
    }

    fn labels(sections: &[Section]) -> Vec<&str> {
        sections.iter().map(|section| section.label.as_str()).collect()
    }

    #[test]
    fn shipped_catalog_parses_and_is_populated() {
        let engine = engine();
        assert!(engine.catalog.group.len() >= 10);
        assert!(engine
            .catalog
            .group
            .iter()
            .all(|group| !group.entries.is_empty()));
    }

    #[test]
    fn gates_hide_groups_whose_state_does_not_hold() {
        let engine = engine();
        let plain = engine.suggest(&Signals::default(), 99);

        assert!(!labels(&plain).contains(&"Code"));
        assert!(!labels(&plain).contains(&"Fix"));
        assert!(!labels(&plain).contains(&"Save"));
    }

    #[test]
    fn lsp_signal_reveals_the_code_group() {
        let engine = engine();
        let signals = Signals {
            lsp: true,
            ..Signals::default()
        };

        assert!(labels(&engine.suggest(&signals, 99)).contains(&"Code"));
    }

    #[test]
    fn hovering_a_tab_wins_over_everything_else() {
        let engine = engine();
        let signals = Signals {
            focus: Focus::Tab,
            ..Signals::default()
        };

        let sections = engine.suggest(&signals, 1);
        assert_eq!(labels(&sections), vec!["Tab"]);
        assert!(sections[0]
            .items
            .iter()
            .any(|item| item.keys == "C-w v"));
    }

    #[test]
    fn insert_mode_replaces_normal_mode_suggestions() {
        let engine = engine();
        let signals = Signals {
            mode: Mode::Insert,
            ..Signals::default()
        };

        let sections = engine.suggest(&signals, 99);
        assert_eq!(labels(&sections), vec!["Insert"]);
    }

    #[test]
    fn an_unsaved_buffer_promotes_the_save_group() {
        let engine = engine();
        let signals = Signals {
            modified: true,
            ..Signals::default()
        };

        assert_eq!(labels(&engine.suggest(&signals, 1)), vec!["Save"]);
    }

    #[test]
    fn searching_promotes_next_match_to_the_front() {
        let mut engine = engine();
        engine.record("search");

        let section = engine
            .suggest(&Signals::default(), 1)
            .into_iter()
            .next()
            .unwrap();

        assert_eq!(section.label, "Search");
        assert_eq!(section.items[0].keys, "n");
    }

    #[test]
    fn yanking_promotes_paste_above_its_baseline() {
        let mut engine = engine();
        engine.record("yank");

        let edit = engine
            .suggest(&Signals::default(), 99)
            .into_iter()
            .find(|section| section.label == "Edit")
            .unwrap();

        let paste = edit.items.iter().position(|item| item.keys == "p").unwrap();
        let insert = edit.items.iter().position(|item| item.keys == "i").unwrap();
        assert!(paste < insert);
    }

    #[test]
    fn the_command_just_run_is_pushed_down() {
        let mut engine = engine();
        engine.record("insert_mode");

        let edit = engine
            .suggest(&Signals::default(), 99)
            .into_iter()
            .find(|section| section.label == "Edit")
            .unwrap();

        assert_ne!(edit.items[0].keys, "i");
    }

    #[test]
    fn a_recency_edge_nudges_a_group_without_inverting_it() {
        let mut engine = engine();
        engine.record("undo");

        let edit = engine
            .suggest(&Signals::default(), 99)
            .into_iter()
            .find(|section| section.label == "Edit")
            .unwrap();

        let redo = edit.items.iter().position(|item| item.keys == "U").unwrap();
        let insert = edit.items.iter().position(|item| item.keys == "i").unwrap();

        assert!(
            insert < redo,
            "`U redo` (weight 13) overtook `i insert` (weight 24) on one edge"
        );
    }

    #[test]
    fn a_plain_buffer_teaches_selection_then_editing_then_movement() {
        let engine = engine();
        let sections = engine.suggest(&Signals::default(), 3);

        assert_eq!(labels(&sections), vec!["Select", "Edit", "Navigate"]);
    }

    #[test]
    fn fast_navigation_is_offered_in_a_plain_buffer() {
        let engine = engine();
        let nav = engine
            .suggest(&Signals::default(), 99)
            .into_iter()
            .find(|section| section.label == "Navigate")
            .unwrap();

        let keys: Vec<&str> = nav.items.iter().map(|item| item.keys.as_str()).collect();
        assert_eq!(keys[0], "C-d");
        assert!(keys.contains(&"C-u"));
        assert!(keys.contains(&"C-o"));
    }

    #[test]
    fn jumping_to_a_definition_promotes_the_way_back() {
        let mut engine = engine();
        engine.record("goto_definition");

        let nav = engine
            .suggest(&Signals { lsp: true, ..Signals::default() }, 99)
            .into_iter()
            .find(|section| section.label == "Navigate")
            .unwrap();

        assert_eq!(nav.items[0].keys, "C-o");
    }

    fn selected(linewise: bool, multiline: bool) -> Signals {
        Signals {
            selection: true,
            linewise,
            multiline,
            ..Signals::default()
        }
    }

    #[test]
    fn a_selection_shows_how_to_extend_it_and_how_to_copy_it() {
        let engine = engine();
        let sections = engine.suggest(&selected(false, false), 2);

        assert_eq!(labels(&sections), vec!["Extend", "Selection"]);
    }

    #[test]
    fn a_line_selection_offers_to_add_the_next_line() {
        let engine = engine();
        let extend = engine
            .suggest(&selected(true, false), 99)
            .into_iter()
            .find(|section| section.label == "Extend")
            .unwrap();

        assert!(extend
            .items
            .iter()
            .any(|item| item.label == "add next line"));
    }

    #[test]
    fn negated_gates_hide_entries_that_do_not_apply() {
        let engine = engine();

        let linewise = engine
            .suggest(&selected(true, false), 99)
            .into_iter()
            .find(|section| section.label == "Extend")
            .unwrap();
        assert!(!linewise
            .items
            .iter()
            .any(|item| item.label == "snap to whole lines"));

        let partial = engine
            .suggest(&selected(false, false), 99)
            .into_iter()
            .find(|section| section.label == "Extend")
            .unwrap();
        assert!(partial
            .items
            .iter()
            .any(|item| item.label == "snap to whole lines"));
    }

    #[test]
    fn multiline_selections_offer_to_split_into_cursors() {
        let engine = engine();
        let extend = engine
            .suggest(&selected(true, true), 99)
            .into_iter()
            .find(|section| section.label == "Extend")
            .unwrap();

        assert!(extend
            .items
            .iter()
            .any(|item| item.label == "split into cursors"));
    }

    #[test]
    fn yanking_to_a_register_puts_clipboard_copy_first() {
        let mut engine = engine();
        engine.record("yank");

        let section = engine
            .suggest(
                &Signals {
                    selection: true,
                    ..Signals::default()
                },
                1,
            )
            .into_iter()
            .next()
            .unwrap();

        assert_eq!(section.label, "Selection");
        assert_eq!(section.items[0].keys, "␣y");
        assert_eq!(section.items[0].label, "copy to system clipboard");
    }

    #[test]
    fn learned_edges_accumulate_across_repeats() {
        let mut engine = engine();
        for _ in 0..5 {
            engine.record("move_next_word_start");
            engine.record("delete_selection");
        }

        engine.record("move_next_word_start");
        assert!(engine.learned("delete_selection") > 0);
    }

    #[test]
    fn recency_decays_with_distance() {
        let mut engine = engine();
        engine.record("search");
        let close = engine.recency(&["search".to_string()], 96);

        engine.record("move_next_word_start");
        engine.record("move_prev_word_start");
        let far = engine.recency(&["search".to_string()], 96);

        assert!(close > far);
        assert!(far > 0);
    }

    #[test]
    fn the_select_group_leads_with_word_and_line() {
        let engine = engine();
        let select = engine
            .suggest(&Signals::default(), 99)
            .into_iter()
            .find(|section| section.label == "Select")
            .unwrap();

        assert_eq!(select.items[0].label, "word");
        assert_eq!(select.items[1].label, "line");
    }

    #[test]
    fn repeatable_commands_keep_their_rank_after_being_used() {
        let mut engine = engine();
        engine.record("extend_line_below");

        let select = engine
            .suggest(&Signals::default(), 99)
            .into_iter()
            .find(|section| section.label == "Select")
            .unwrap();

        let line = select
            .items
            .iter()
            .position(|item| item.label == "line")
            .unwrap();

        assert!(line <= 1, "`x line` fell to position {line} after being used");
    }

    #[test]
    fn every_entry_in_a_shown_group_carries_an_explicit_weight() {
        for group in &engine().catalog.group {
            if declares_pending(group) {
                continue;
            }
            for entry in &group.entries {
                assert!(
                    entry.weight.is_some(),
                    "`{}` in group `{}` has no weight, so its rank is file order",
                    entry.label,
                    group.id
                );
            }
        }
    }

    #[test]
    fn the_catalog_never_teaches_a_space_key_we_disabled() {
        let defaults: toml::Value = toml::from_str(crate::config::DEFAULTS).unwrap();
        let space = defaults["keys"]["normal"]["space"].as_table().unwrap();

        let disabled: Vec<&str> = space
            .iter()
            .filter(|(_, command)| command.as_str() == Some("no_op"))
            .map(|(key, _)| key.as_str())
            .collect();

        assert!(!disabled.is_empty());

        for group in &engine().catalog.group {
            for entry in &group.entries {
                if let Some(key) = entry.keys.strip_prefix('␣') {
                    assert!(
                        !disabled.contains(&key),
                        "`{}` teaches space+{key}, which default.toml unbinds",
                        entry.label
                    );
                }
            }
        }
    }

    fn pending(prefix: &str) -> Signals {
        Signals {
            pending: Some(prefix.to_string()),
            ..Signals::default()
        }
    }

    #[test]
    fn the_sidebar_taking_the_keyboard_takes_over_the_panel_too() {
        let engine = engine();
        let sections = engine.suggest(&pending("sidebar"), 99);

        assert_eq!(labels(&sections), vec!["Explorer"]);

        let keys: Vec<&str> = sections[0].items.iter().map(|i| i.keys.as_str()).collect();
        assert_eq!(keys[0], "Enter");
        assert!(keys.contains(&"C-v"));
        assert!(keys.contains(&"C-s"));

        assert!(engine.pinned(&pending("sidebar")).is_empty());
    }

    #[test]
    fn a_pending_prefix_takes_over_the_whole_panel() {
        let engine = engine();
        let sections = engine.suggest(&pending("goto"), 99);

        assert_eq!(labels(&sections), vec!["Goto"]);
        assert!(sections[0].items.iter().any(|item| item.keys == "d"));
    }

    #[test]
    fn each_prefix_menu_maps_to_its_own_group() {
        let engine = engine();
        for (prefix, label) in [
            ("goto", "Goto"),
            ("space", "Space"),
            ("match", "Match"),
            ("window", "Window"),
            ("view", "View"),
        ] {
            assert_eq!(labels(&engine.suggest(&pending(prefix), 99)), vec![label]);
        }
    }

    #[test]
    fn normal_groups_are_hidden_while_a_prefix_is_pending() {
        let engine = engine();
        let sections = engine.suggest(&pending("goto"), 99);

        assert!(!labels(&sections).contains(&"Navigate"));
        assert!(!labels(&sections).contains(&"Search"));
    }

    #[test]
    fn prefix_groups_never_leak_into_normal_mode() {
        let engine = engine();
        let sections = engine.suggest(&Signals::default(), 99);
        let normal = labels(&sections);

        for leaked in ["Goto", "Space", "Match", "Window", "View"] {
            assert!(!normal.contains(&leaked), "{leaked} leaked into normal mode");
        }
    }

    #[test]
    fn an_unrecognised_popup_falls_back_instead_of_going_blank() {
        let engine = engine();
        let sections = engine.suggest(&pending("registers"), 99);

        assert!(!sections.is_empty());
        assert!(labels(&sections).contains(&"Navigate"));
    }

    #[test]
    fn pinned_groups_are_always_offered_whatever_the_state() {
        let engine = engine();

        for signals in [
            Signals::default(),
            selected(true, true),
            Signals {
                mode: Mode::Insert,
                ..Signals::default()
            },
            Signals {
                lsp: true,
                modified: true,
                ..Signals::default()
            },
        ] {
            let pinned = engine.pinned(&signals);
            assert_eq!(labels(&pinned), vec!["Any time"]);
            assert!(pinned[0].items.iter().any(|item| item.keys == "␣a"));
            assert!(pinned[0].items.iter().any(|item| item.keys == "␣?"));
        }
    }

    #[test]
    fn pinned_groups_never_compete_in_the_ranked_list() {
        let engine = engine();
        let ranked = engine.suggest(&Signals::default(), 99);

        assert!(!labels(&ranked).contains(&"Any time"));
    }

    #[test]
    fn a_prefix_menu_takes_over_the_reserved_area_too() {
        assert!(engine().pinned(&pending("goto")).is_empty());
    }

    #[test]
    fn a_broken_catalog_degrades_to_empty_rather_than_panicking() {
        let engine = Engine::new("this is not valid toml {{{");
        assert!(engine.suggest(&Signals::default(), 99).is_empty());
    }
}
