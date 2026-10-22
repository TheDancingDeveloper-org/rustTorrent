//! Small painters shared by the screens: the piece bitfield, the state badge, a progress cell.

use egui::{Color32, Rect, Sense, Ui, Vec2, pos2, vec2};
use fluent2_egui::theme::{Theme, c};
use fluent2_egui::{Badge, BadgeAppearance, BadgeColor, BadgeSize};

use crate::data::{Piece, State};

/// Colour of each piece state in the current theme.
pub fn piece_colors(ui: &Ui) -> [Color32; 3] {
    let theme = Theme::get(ui.ctx());
    [
        c(theme.color.neutral_stroke_2),
        c(theme.color.status_warning_foreground_1),
        c(theme.color.brand_background),
    ]
}

fn piece_color(colors: &[Color32; 3], piece: Piece) -> Color32 {
    match piece {
        Piece::Missing => colors[0],
        Piece::Downloading => colors[1],
        Piece::Have => colors[2],
    }
}

/// The piece map: one square per piece, wrapped into rows, painted directly (no widgets).
/// Hovering a square shows its index and state.
pub fn pieces_map(ui: &mut Ui, pieces: &[Piece], piece_size: u64) {
    let colors = piece_colors(ui);
    let cell = 9.0;
    let gap = 1.0;
    let width = ui.available_width().max(cell);
    let per_row = ((width + gap) / (cell + gap)).floor().max(1.0) as usize;
    let rows = pieces.len().div_ceil(per_row);
    let height = rows as f32 * (cell + gap);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    for (i, piece) in pieces.iter().enumerate() {
        let (row, col) = (i / per_row, i % per_row);
        let min = rect.min + vec2(col as f32 * (cell + gap), row as f32 * (cell + gap));
        let square = Rect::from_min_size(min, Vec2::splat(cell));
        if ui.is_rect_visible(square) {
            painter.rect_filled(square, 1.5, piece_color(&colors, *piece));
        }
    }
    if let Some(pos) = response.hover_pos() {
        let rel = pos - rect.min;
        let (col, row) = (
            (rel.x / (cell + gap)) as usize,
            (rel.y / (cell + gap)) as usize,
        );
        let i = row * per_row + col;
        if col < per_row
            && let Some(piece) = pieces.get(i)
        {
            response.on_hover_text(format!(
                "Piece {i} of {} · {} · {}",
                pieces.len(),
                crate::data::bytes(piece_size),
                match piece {
                    Piece::Missing => "missing",
                    Piece::Downloading => "downloading",
                    Piece::Have => "verified",
                }
            ));
        }
    }
}

/// A one-line summary strip of the bitfield, as the overview tab shows above the numbers.
pub fn pieces_strip(ui: &mut Ui, pieces: &[Piece], height: f32) {
    let colors = piece_colors(ui);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, colors[0]);
    if pieces.is_empty() {
        return;
    }
    let buckets = (width as usize).max(1);
    for b in 0..buckets {
        let start = b * pieces.len() / buckets;
        let end = ((b + 1) * pieces.len() / buckets)
            .max(start + 1)
            .min(pieces.len());
        let slice = &pieces[start..end];
        let piece = if slice.iter().all(|p| *p == Piece::Have) {
            Piece::Have
        } else if slice.contains(&Piece::Downloading) {
            Piece::Downloading
        } else if slice.contains(&Piece::Have) {
            Piece::Have
        } else {
            continue;
        };
        let x = rect.left() + b as f32;
        painter.rect_filled(
            Rect::from_min_max(pos2(x, rect.top()), pos2(x + 1.0, rect.bottom())),
            0.0,
            piece_color(&colors, piece),
        );
    }
}

/// Legend under the piece map.
pub fn pieces_legend(ui: &mut Ui) {
    let colors = piece_colors(ui);
    ui.horizontal(|ui| {
        for (color, label) in colors.iter().zip(["Missing", "Downloading", "Verified"]) {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, *color);
            ui.label(label);
            ui.add_space(8.0);
        }
    });
}

pub fn state_color(state: State) -> BadgeColor {
    match state {
        State::Downloading => BadgeColor::Brand,
        State::Seeding => BadgeColor::Success,
        State::Paused => BadgeColor::Subtle,
        State::Queued => BadgeColor::Informative,
        State::Checking => BadgeColor::Warning,
        State::Stalled => BadgeColor::Severe,
        State::Error => BadgeColor::Danger,
    }
}

pub fn state_badge(ui: &mut Ui, state: State) -> egui::Response {
    ui.add(
        Badge::new(state.label())
            .appearance(BadgeAppearance::Tint)
            .color(state_color(state))
            .size(BadgeSize::Small),
    )
}

/// A thin progress bar with the percentage beside it, for the Progress column.
pub fn progress_cell(ui: &mut Ui, progress: f32, state: State) {
    let theme = Theme::get(ui.ctx());
    let fill = match state {
        State::Seeding => c(theme.color.status_success_foreground_1),
        State::Error => c(theme.color.status_danger_foreground_1),
        State::Paused | State::Queued => c(theme.color.neutral_foreground_3),
        _ => c(theme.color.brand_background),
    };
    let text = format!("{:.1}%", progress * 100.0);
    let width = ui.available_width();
    let bar_w = (width - 52.0).max(20.0);
    ui.horizontal_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(bar_w, 4.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.0, c(theme.color.neutral_stroke_2));
        let mut done = rect;
        done.set_width(rect.width() * progress.clamp(0.0, 1.0));
        ui.painter().rect_filled(done, 2.0, fill);
        ui.label(text);
    });
}

/// A raised Fluent surface: `neutral_background_1`, a thin stroke and a soft shadow.
///
/// fluent2-egui's `Card` paints its shadow after its content, which greys the content out, so
/// the mockup draws its own card with an `egui::Frame`.
pub fn card<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = Theme::get(ui.ctx());
    egui::Frame::new()
        .fill(c(theme.color.neutral_background_1))
        .stroke(egui::Stroke::new(1.0, c(theme.color.neutral_stroke_2)))
        .corner_radius(8.0)
        .inner_margin(16.0)
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(28),
        })
        .show(ui, content)
        .inner
}
