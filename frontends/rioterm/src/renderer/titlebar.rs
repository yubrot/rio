//! The title bar Rio draws in place of the OS caption (`window.decorations = "Transparent"`
//! on Windows): a band at the top of the window holding the tab strip and the caption buttons.

use crate::renderer::island::ISLAND_HEIGHT;
use rio_backend::sugarloaf::Sugarloaf;
use rio_window::window::CaptionButton;

/// Width of a caption button in logical px, as in Windows 11.
pub const CAPTION_BUTTON_WIDTH: f32 = 46.0;
/// Room kept between the tabs and the caption buttons to drag the window by.
const DRAG_GAP: f32 = 40.0;
const GLYPH_HALF: f32 = 5.0;
const GLYPH_STROKE: f32 = 1.0;
const RESTORE_OFFSET: f32 = 2.0;
const CLOSE_HOVER_FILL: [f32; 4] = [0.769, 0.169, 0.110, 1.0];
const CLOSE_PRESSED_FILL: [f32; 4] = [0.769, 0.169, 0.110, 0.9];
const INACTIVE_GLYPH_ALPHA: f32 = 0.45;
const ORDER: u8 = 0;

const BUTTONS: [CaptionButton; 3] = [
    CaptionButton::Minimize,
    CaptionButton::Maximize,
    CaptionButton::Close,
];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Titlebar {
    pub fullscreen: bool,
    pub maximized: bool,
    pub hovered: Option<CaptionButton>,
    pub pressed: Option<CaptionButton>,
}

impl Titlebar {
    /// Whether the band and the caption buttons are shown; fullscreen has no caption.
    #[inline]
    pub fn is_shown(&self) -> bool {
        !self.fullscreen
    }

    /// Logical width the tab strip must leave free at its right end.
    #[inline]
    pub fn reserved_right(&self) -> f32 {
        if self.is_shown() {
            CAPTION_BUTTON_WIDTH * BUTTONS.len() as f32 + DRAG_GAP
        } else {
            0.0
        }
    }

    /// Caption buttons with their logical left edge, right-aligned in the window.
    pub fn buttons(
        &self,
        logical_width: f32,
    ) -> impl Iterator<Item = (CaptionButton, f32)> + '_ {
        let left = logical_width - CAPTION_BUTTON_WIDTH * BUTTONS.len() as f32;
        BUTTONS
            .into_iter()
            .enumerate()
            .filter(|_| self.is_shown())
            .map(move |(i, button)| (button, left + CAPTION_BUTTON_WIDTH * i as f32))
    }

    pub fn render(
        &self,
        sugarloaf: &mut Sugarloaf,
        window_width: f32,
        scale_factor: f32,
        bg_color: [f32; 4],
        focused: bool,
    ) {
        let luminance =
            0.2126 * bg_color[0] + 0.7152 * bg_color[1] + 0.0722 * bg_color[2];
        let ink = if luminance > 0.5 { 0.0 } else { 1.0 };

        for (button, x) in self.buttons(window_width / scale_factor) {
            let hovered = self.hovered == Some(button);
            let pressed = hovered && self.pressed == Some(button);
            let fill = match (button, hovered, pressed) {
                (_, false, _) => None,
                (CaptionButton::Close, true, false) => Some(CLOSE_HOVER_FILL),
                (CaptionButton::Close, true, true) => Some(CLOSE_PRESSED_FILL),
                (_, true, false) => Some([ink, ink, ink, 0.08]),
                (_, true, true) => Some([ink, ink, ink, 0.05]),
            };
            if let Some(fill) = fill {
                sugarloaf.rect(
                    None,
                    x,
                    0.0,
                    CAPTION_BUTTON_WIDTH,
                    ISLAND_HEIGHT,
                    fill,
                    0.0,
                    ORDER,
                );
            }

            let glyph = if button == CaptionButton::Close && hovered {
                [1.0, 1.0, 1.0, 1.0]
            } else if focused || hovered {
                [ink, ink, ink, 1.0]
            } else {
                [ink, ink, ink, INACTIVE_GLYPH_ALPHA]
            };
            let cx = (x + CAPTION_BUTTON_WIDTH / 2.0).round();
            let cy = (ISLAND_HEIGHT / 2.0).round();
            draw_glyph(sugarloaf, button, self.maximized, cx, cy, glyph);
        }
    }
}

fn draw_glyph(
    sugarloaf: &mut Sugarloaf,
    button: CaptionButton,
    maximized: bool,
    cx: f32,
    cy: f32,
    color: [f32; 4],
) {
    let r = GLYPH_HALF;
    let mut line = |x1: f32, y1: f32, x2: f32, y2: f32| {
        sugarloaf.line(x1, y1, x2, y2, GLYPH_STROKE, 0.0, color, ORDER);
    };
    match button {
        CaptionButton::Minimize => line(cx - r, cy, cx + r, cy),
        CaptionButton::Maximize if maximized => {
            // Front window, then the corner of the one behind it.
            let o = RESTORE_OFFSET;
            stroke_square(&mut line, cx - r, cy - r + o, 2.0 * r - o);
            line(cx - r + o, cy - r, cx + r, cy - r);
            line(cx + r, cy - r, cx + r, cy + r - o);
        }
        CaptionButton::Maximize => stroke_square(&mut line, cx - r, cy - r, 2.0 * r),
        CaptionButton::Close => {
            line(cx - r, cy - r, cx + r, cy + r);
            line(cx - r, cy + r, cx + r, cy - r);
        }
    }
}

fn stroke_square(line: &mut impl FnMut(f32, f32, f32, f32), x: f32, y: f32, size: f32) {
    line(x, y, x + size, y);
    line(x + size, y, x + size, y + size);
    line(x + size, y + size, x, y + size);
    line(x, y + size, x, y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_right_aligned_in_order() {
        let titlebar = Titlebar::default();
        let buttons: Vec<_> = titlebar.buttons(800.0).collect();
        assert_eq!(
            buttons,
            vec![
                (CaptionButton::Minimize, 800.0 - CAPTION_BUTTON_WIDTH * 3.0),
                (CaptionButton::Maximize, 800.0 - CAPTION_BUTTON_WIDTH * 2.0),
                (CaptionButton::Close, 800.0 - CAPTION_BUTTON_WIDTH),
            ]
        );
        assert!(titlebar.reserved_right() > CAPTION_BUTTON_WIDTH * 3.0);
    }

    #[test]
    fn fullscreen_hides_the_caption() {
        let titlebar = Titlebar {
            fullscreen: true,
            ..Titlebar::default()
        };
        assert_eq!(titlebar.buttons(800.0).count(), 0);
        assert_eq!(titlebar.reserved_right(), 0.0);
    }
}
