use helix_term::compositor::Component;
use helix_term::ui::overlay::Overlay;
use helix_view::graphics::Rect;

pub const WIDTH_PERCENT: u16 = 80;
pub const HEIGHT_PERCENT: u16 = 70;
pub const MAX_WIDTH: u16 = 160;
pub const MAX_HEIGHT: u16 = 40;

pub const DRAWER_WIDTH_PERCENT: u16 = 25;
pub const DRAWER_MIN_WIDTH: u16 = 24;

pub fn drawer_left<T: Component>(content: T) -> Overlay<T> {
    Overlay {
        content,
        calc_child_size: Box::new(|rect: Rect| {
            let width = scale(rect.width, DRAWER_WIDTH_PERCENT)
                .max(DRAWER_MIN_WIDTH)
                .min(rect.width);

            Rect {
                x: rect.x,
                y: rect.y,
                width,
                height: rect.height.saturating_sub(2),
            }
        }),
    }
}

pub fn centered<T: Component>(content: T) -> Overlay<T> {
    sized(content, WIDTH_PERCENT, HEIGHT_PERCENT)
}

pub fn sized<T: Component>(content: T, width_percent: u16, height_percent: u16) -> Overlay<T> {
    Overlay {
        content,
        calc_child_size: Box::new(move |rect: Rect| {
            let width = scale(rect.width, width_percent).min(MAX_WIDTH).min(rect.width);
            let height = scale(rect.height.saturating_sub(2), height_percent)
                .min(MAX_HEIGHT)
                .min(rect.height);

            Rect {
                x: rect.x + rect.width.saturating_sub(width) / 2,
                y: rect.y + rect.height.saturating_sub(height) / 2,
                width,
                height,
            }
        }),
    }
}

fn scale(size: u16, percent: u16) -> u16 {
    ((size as u32 * percent as u32) / 100) as u16
}
