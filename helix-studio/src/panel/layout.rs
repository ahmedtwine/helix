use std::str::FromStr;

use helix_view::graphics::Rect;
use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    Viewport,
    Editor,
    Cursor,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
    Over,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    Fit,
    Fill,
    Cells(u16),
    Percent(u8),
}

impl FromStr for Extent {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        match raw {
            "fit" => Ok(Extent::Fit),
            "fill" => Ok(Extent::Fill),
            _ => match raw.strip_suffix('%') {
                Some(percent) => percent
                    .trim()
                    .parse()
                    .map(Extent::Percent)
                    .map_err(|_| format!("`{raw}` is not a percentage")),
                None => raw
                    .parse()
                    .map(Extent::Cells)
                    .map_err(|_| format!("`{raw}` is not a cell count")),
            },
        }
    }
}

impl<'de> Deserialize<'de> for Extent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Placement {
    pub anchor: Anchor,
    pub side: Side,
    pub align: Align,
    pub width: Extent,
    pub height: Extent,
    pub offset: (i16, i16),
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            anchor: Anchor::Viewport,
            side: Side::Bottom,
            align: Align::Center,
            width: Extent::Fill,
            height: Extent::Fit,
            offset: (0, 0),
        }
    }
}

impl Placement {
    pub fn resolve(&self, viewport: Rect, anchor: Rect, content: (u16, u16)) -> Rect {
        let width = extent(self.width, anchor.width, viewport.width, content.0);
        let height = extent(self.height, anchor.height, viewport.height, content.1);

        if width == 0 || height == 0 {
            return Rect::new(anchor.x, anchor.y, 0, 0);
        }

        let (x, y) = match self.side {
            Side::Top => (align(self.align, anchor.x, anchor.width, width), anchor.y),
            Side::Bottom => (
                align(self.align, anchor.x, anchor.width, width),
                anchor.bottom().saturating_sub(height),
            ),
            Side::Left => (anchor.x, align(self.align, anchor.y, anchor.height, height)),
            Side::Right => (
                anchor.right().saturating_sub(width),
                align(self.align, anchor.y, anchor.height, height),
            ),
            Side::Over => (
                align(Align::Center, anchor.x, anchor.width, width),
                align(Align::Center, anchor.y, anchor.height, height),
            ),
        };

        Rect::new(
            shift(x, self.offset.0, viewport.x, viewport.right(), width),
            shift(y, self.offset.1, viewport.y, viewport.bottom(), height),
            width,
            height,
        )
    }
}

fn extent(extent: Extent, anchor: u16, viewport: u16, content: u16) -> u16 {
    match extent {
        Extent::Fit => content.min(viewport),
        Extent::Fill => anchor.min(viewport),
        Extent::Cells(cells) => cells.min(viewport),
        Extent::Percent(percent) => {
            ((u32::from(anchor) * u32::from(percent.min(100))) / 100) as u16
        }
    }
}

fn align(align: Align, start: u16, available: u16, size: u16) -> u16 {
    let slack = available.saturating_sub(size);
    match align {
        Align::Start => start,
        Align::Center => start + slack / 2,
        Align::End => start + slack,
    }
}

fn shift(value: u16, offset: i16, min: u16, max: u16, size: u16) -> u16 {
    let ceiling = max.saturating_sub(size).max(min);
    (i32::from(value) + i32::from(offset)).clamp(i32::from(min), i32::from(ceiling)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };

    fn placement(side: Side, width: Extent, height: Extent) -> Placement {
        Placement {
            side,
            width,
            height,
            ..Placement::default()
        }
    }

    #[test]
    fn footer_hugs_the_bottom_edge() {
        let area = placement(Side::Bottom, Extent::Fill, Extent::Cells(1)).resolve(
            VIEWPORT,
            VIEWPORT,
            (0, 0),
        );
        assert_eq!(area, Rect::new(0, 39, 100, 1));
    }

    #[test]
    fn left_drawer_takes_a_percentage() {
        let area = placement(Side::Left, Extent::Percent(25), Extent::Fill).resolve(
            VIEWPORT,
            VIEWPORT,
            (0, 0),
        );
        assert_eq!(area, Rect::new(0, 0, 25, 40));
    }

    #[test]
    fn fit_uses_content_size_but_never_overflows() {
        let area = placement(Side::Over, Extent::Fit, Extent::Fit).resolve(VIEWPORT, VIEWPORT, (30, 5));
        assert_eq!(area.width, 30);
        assert_eq!(area.height, 5);

        let clamped =
            placement(Side::Over, Extent::Fit, Extent::Fit).resolve(VIEWPORT, VIEWPORT, (400, 90));
        assert_eq!(clamped.width, 100);
        assert_eq!(clamped.height, 40);
    }

    #[test]
    fn offset_lifts_the_panel_off_the_edge() {
        let lifted = Placement {
            offset: (0, -1),
            ..placement(Side::Bottom, Extent::Fill, Extent::Cells(1))
        }
        .resolve(VIEWPORT, VIEWPORT, (0, 0));
        assert_eq!(lifted.y, 38);
    }

    #[test]
    fn offset_never_escapes_the_viewport() {
        let pinned = Placement {
            offset: (0, 99),
            ..placement(Side::Bottom, Extent::Fill, Extent::Cells(1))
        }
        .resolve(VIEWPORT, VIEWPORT, (0, 0));
        assert_eq!(pinned.y, 39);
    }

    #[test]
    fn align_moves_along_the_cross_axis() {
        let start = Placement {
            align: Align::Start,
            ..placement(Side::Bottom, Extent::Cells(20), Extent::Cells(1))
        }
        .resolve(VIEWPORT, VIEWPORT, (0, 0));
        let end = Placement {
            align: Align::End,
            ..placement(Side::Bottom, Extent::Cells(20), Extent::Cells(1))
        }
        .resolve(VIEWPORT, VIEWPORT, (0, 0));

        assert_eq!(start.x, 0);
        assert_eq!(end.x, 80);
    }

    #[test]
    fn extent_parses_every_config_spelling() {
        assert_eq!("fit".parse::<Extent>().unwrap(), Extent::Fit);
        assert_eq!("fill".parse::<Extent>().unwrap(), Extent::Fill);
        assert_eq!("25%".parse::<Extent>().unwrap(), Extent::Percent(25));
        assert_eq!("12".parse::<Extent>().unwrap(), Extent::Cells(12));
        assert!("nonsense".parse::<Extent>().is_err());
    }

    #[test]
    fn fit_against_a_one_cell_cursor_anchor_uses_the_content_not_the_anchor() {
        let cursor = Rect::new(20, 10, 1, 1);
        let area = Placement {
            side: Side::Bottom,
            align: Align::Start,
            width: Extent::Fit,
            height: Extent::Fit,
            offset: (0, -1),
            ..Placement::default()
        }
        .resolve(VIEWPORT, cursor, (44, 3));

        assert_eq!(area.width, 44);
        assert_eq!(area.height, 3);
        assert_eq!(area.bottom(), 10);
    }

    #[test]
    fn a_popup_above_the_cursor_never_covers_the_cursor_row() {
        let cursor = Rect::new(5, 20, 1, 1);
        let area = Placement {
            side: Side::Bottom,
            width: Extent::Fit,
            height: Extent::Fit,
            offset: (0, -1),
            ..Placement::default()
        }
        .resolve(VIEWPORT, cursor, (30, 4));

        assert!(area.bottom() <= cursor.y);
    }

    #[test]
    fn fill_still_tracks_the_anchor_rather_than_the_viewport() {
        let anchor = Rect::new(0, 0, 40, 10);
        let area = placement(Side::Top, Extent::Fill, Extent::Cells(1)).resolve(
            VIEWPORT,
            anchor,
            (0, 0),
        );

        assert_eq!(area.width, 40);
    }

    #[test]
    fn anchor_smaller_than_viewport_still_clamps_inside_it() {
        let anchor = Rect::new(10, 5, 40, 20);
        let area =
            placement(Side::Bottom, Extent::Fill, Extent::Cells(2)).resolve(VIEWPORT, anchor, (0, 0));
        assert_eq!(area, Rect::new(10, 23, 40, 2));
    }
}
