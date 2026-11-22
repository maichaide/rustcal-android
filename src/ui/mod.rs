pub mod calendar_grid;
pub mod editors;
pub mod settings;
pub mod todo_panel;

use eframe::egui::Color32;

pub fn parse_hex_color(hex: &str) -> Color32 {
    let cleaned = hex.trim().trim_start_matches('#');
    if cleaned.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&cleaned[0..2], 16),
            u8::from_str_radix(&cleaned[2..4], 16),
            u8::from_str_radix(&cleaned[4..6], 16),
        ) {
            return Color32::from_rgb(r, g, b);
        }
    }
    Color32::from_rgb(37, 99, 235)
}

pub fn color_to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}
