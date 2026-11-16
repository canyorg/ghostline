//! Falling-glyph "digital rain" background.

use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect};
use rand::Rng;

const CELL: f32 = 16.0;
const GLYPHS: &[char] = &[
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'K',
    'M', 'N', 'P', 'R', 'T', 'X', 'Z', '$', '#', '@', '%', '&', '*', '+', '=', '<', '>', '/', '\\',
    '|', ':', 'Ψ', 'Ω', 'Σ', 'Ж', 'Я', 'Ф', 'λ', 'π',
];

struct Column {
    head: f32,
    speed: f32,
    len: usize,
    glyphs: Vec<char>,
}

pub struct Rain {
    cols: Vec<Column>,
    rows: usize,
}

impl Rain {
    pub fn new() -> Self {
        Rain {
            cols: Vec::new(),
            rows: 0,
        }
    }

    fn new_column(rows: usize, rng: &mut impl Rng, scatter: bool) -> Column {
        let len = rng.gen_range(6..26);
        Column {
            head: if scatter {
                rng.gen_range(-(rows as f32)..rows as f32)
            } else {
                -(rng.gen_range(0..rows.max(1)) as f32)
            },
            speed: rng.gen_range(5.0..18.0),
            len,
            glyphs: (0..rows + 30)
                .map(|_| GLYPHS[rng.gen_range(0..GLYPHS.len())])
                .collect(),
        }
    }

    /// Advance the animation by `dt` seconds and paint into `rect`.
    /// `intensity` 0..1 scales brightness (use low values behind text).
    pub fn paint(&mut self, painter: &Painter, rect: Rect, dt: f32, intensity: f32) {
        let mut rng = rand::thread_rng();
        let ncols = (rect.width() / CELL).ceil() as usize;
        let rows = (rect.height() / CELL).ceil() as usize + 1;
        if ncols != self.cols.len() || rows != self.rows {
            self.rows = rows;
            self.cols = (0..ncols)
                .map(|_| Self::new_column(rows, &mut rng, true))
                .collect();
        }
        let font = FontId::monospace(CELL - 2.0);
        for (ci, col) in self.cols.iter_mut().enumerate() {
            col.head += col.speed * dt.min(0.1);
            if col.head - col.len as f32 > rows as f32 {
                *col = Self::new_column(rows, &mut rng, false);
            }
            // Occasionally mutate a glyph for the flicker effect.
            if rng.gen_bool(0.08) {
                let i = rng.gen_range(0..col.glyphs.len());
                col.glyphs[i] = GLYPHS[rng.gen_range(0..GLYPHS.len())];
            }
            let head = col.head.floor() as i32;
            for k in 0..col.len as i32 {
                let row = head - k;
                if row < 0 || row as usize >= rows {
                    continue;
                }
                let fade = 1.0 - k as f32 / col.len as f32;
                let color = if k == 0 {
                    Color32::from_rgba_unmultiplied(200, 255, 200, (230.0 * intensity) as u8)
                } else {
                    Color32::from_rgba_unmultiplied(
                        0,
                        (120.0 + 135.0 * fade) as u8,
                        (40.0 * fade) as u8,
                        (220.0 * fade * intensity) as u8,
                    )
                };
                let ch = col.glyphs[row as usize % col.glyphs.len()];
                painter.text(
                    Pos2::new(
                        rect.left() + ci as f32 * CELL,
                        rect.top() + row as f32 * CELL,
                    ),
                    Align2::LEFT_TOP,
                    ch,
                    font.clone(),
                    color,
                );
            }
        }
    }
}

/// Repaint cadence for the animation (~30 fps).
pub fn schedule(ctx: &egui::Context) {
    ctx.request_repaint_after(std::time::Duration::from_millis(33));
}
