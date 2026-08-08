use std::path::Path;
use std::process::Command;

use helix_core::Rope;
use helix_vcs::Hunk;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Context,
    Removed,
    Added,
    Gap,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Cell {
    pub number: u32,
    pub text: String,
    pub kind: Kind,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    pub left: Option<Cell>,
    pub right: Option<Cell>,
}

impl Row {
    fn gap() -> Self {
        Self {
            left: None,
            right: None,
        }
    }

    fn context(before: u32, after: u32, text: String) -> Self {
        Self {
            left: Some(Cell {
                number: before,
                text: text.clone(),
                kind: Kind::Context,
            }),
            right: Some(Cell {
                number: after,
                text,
                kind: Kind::Context,
            }),
        }
    }

    pub fn is_gap(&self) -> bool {
        self.left.is_none() && self.right.is_none()
    }
}

pub fn rows(base: &Rope, doc: &Rope, hunks: &[Hunk], context: u32) -> Vec<Row> {
    let base_lines = base.len_lines() as u32;
    let doc_lines = doc.len_lines() as u32;

    let mut rows = Vec::new();
    let mut before = 0u32;
    let mut after = 0u32;

    for hunk in hunks {
        let lead = hunk.before.start.saturating_sub(context);
        if lead > before {
            if !rows.is_empty() {
                rows.push(Row::gap());
            }
            after += lead - before;
            before = lead;
        }

        while before < hunk.before.start && before < base_lines {
            rows.push(Row::context(before, after, line(base, before)));
            before += 1;
            after += 1;
        }

        let removed = hunk.before.end.saturating_sub(hunk.before.start);
        let added = hunk.after.end.saturating_sub(hunk.after.start);

        for step in 0..removed.max(added) {
            rows.push(Row {
                left: (step < removed).then(|| Cell {
                    number: hunk.before.start + step,
                    text: line(base, hunk.before.start + step),
                    kind: Kind::Removed,
                }),
                right: (step < added).then(|| Cell {
                    number: hunk.after.start + step,
                    text: line(doc, hunk.after.start + step),
                    kind: Kind::Added,
                }),
            });
        }

        before = hunk.before.end;
        after = hunk.after.end;

        let trail = (before + context).min(base_lines);
        while before < trail && after < doc_lines {
            rows.push(Row::context(before, after, line(base, before)));
            before += 1;
            after += 1;
        }
    }

    rows
}

fn line(rope: &Rope, index: u32) -> String {
    let index = index as usize;
    if index >= rope.len_lines() {
        return String::new();
    }

    rope.line(index)
        .to_string()
        .trim_end_matches(['\n', '\r'])
        .to_string()
}

pub fn stage(path: &Path) -> Result<(), String> {
    git(&["add", "--"], path)
}

pub fn unstage(path: &Path) -> Result<(), String> {
    git(&["restore", "--staged", "--"], path)
}

fn git(args: &[&str], path: &Path) -> Result<(), String> {
    let parent = path.parent().unwrap_or(Path::new("."));

    let output = Command::new("git")
        .args(args)
        .arg(path)
        .current_dir(parent)
        .output()
        .map_err(|err| format!("git: {err}"))?;

    match output.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(before: std::ops::Range<u32>, after: std::ops::Range<u32>) -> Hunk {
        Hunk { before, after }
    }

    fn ropes() -> (Rope, Rope) {
        let base = Rope::from_str("one\ntwo\nthree\nfour\nfive\n");
        let doc = Rope::from_str("one\nTWO\nthree\nfour\nfive\n");
        (base, doc)
    }

    #[test]
    fn a_one_line_change_pairs_removed_with_added() {
        let (base, doc) = ropes();
        let rows = rows(&base, &doc, &[hunk(1..2, 1..2)], 0);

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.left.as_ref().unwrap().text, "two");
        assert_eq!(row.left.as_ref().unwrap().kind, Kind::Removed);
        assert_eq!(row.right.as_ref().unwrap().text, "TWO");
        assert_eq!(row.right.as_ref().unwrap().kind, Kind::Added);
    }

    #[test]
    fn context_lines_appear_on_both_sides() {
        let (base, doc) = ropes();
        let rows = rows(&base, &doc, &[hunk(1..2, 1..2)], 1);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].left.as_ref().unwrap().kind, Kind::Context);
        assert_eq!(rows[0].left.as_ref().unwrap().text, "one");
        assert_eq!(rows[2].right.as_ref().unwrap().text, "three");
    }

    #[test]
    fn a_pure_insertion_leaves_the_left_side_empty() {
        let base = Rope::from_str("one\ntwo\n");
        let doc = Rope::from_str("one\nnew\ntwo\n");
        let rows = rows(&base, &doc, &[hunk(1..1, 1..2)], 0);

        assert_eq!(rows.len(), 1);
        assert!(rows[0].left.is_none());
        assert_eq!(rows[0].right.as_ref().unwrap().text, "new");
    }

    #[test]
    fn a_pure_deletion_leaves_the_right_side_empty() {
        let base = Rope::from_str("one\ngone\ntwo\n");
        let doc = Rope::from_str("one\ntwo\n");
        let rows = rows(&base, &doc, &[hunk(1..2, 1..1)], 0);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].left.as_ref().unwrap().text, "gone");
        assert!(rows[0].right.is_none());
    }

    #[test]
    fn uneven_hunks_pad_the_shorter_side() {
        let base = Rope::from_str("a\nb\nc\nd\n");
        let doc = Rope::from_str("a\nX\n");
        let rows = rows(&base, &doc, &[hunk(1..4, 1..2)], 0);

        assert_eq!(rows.len(), 3);
        assert!(rows[0].right.is_some());
        assert!(rows[1].right.is_none());
        assert!(rows[2].right.is_none());
        assert_eq!(rows[2].left.as_ref().unwrap().text, "d");
    }

    #[test]
    fn distant_hunks_are_separated_by_a_gap() {
        let base = Rope::from_str((0..40).map(|n| format!("line{n}\n")).collect::<String>().as_str());
        let doc = base.clone();
        let rows = rows(&base, &doc, &[hunk(1..2, 1..2), hunk(30..31, 30..31)], 1);

        assert!(rows.iter().any(Row::is_gap));
    }

    #[test]
    fn adjacent_hunks_do_not_duplicate_shared_context() {
        let base = Rope::from_str("a\nb\nc\nd\ne\n");
        let doc = base.clone();
        let rows = rows(&base, &doc, &[hunk(1..2, 1..2), hunk(3..4, 3..4)], 1);

        let numbers: Vec<u32> = rows
            .iter()
            .filter_map(|row| row.left.as_ref().map(|cell| cell.number))
            .collect();

        let mut sorted = numbers.clone();
        sorted.dedup();
        assert_eq!(numbers, sorted);
    }

    #[test]
    fn line_lookup_past_the_end_is_empty_rather_than_a_panic() {
        let rope = Rope::from_str("only\n");
        assert_eq!(line(&rope, 99), "");
    }

    #[test]
    fn no_hunks_means_no_rows() {
        let (base, doc) = ropes();
        assert!(rows(&base, &doc, &[], 3).is_empty());
    }
}
