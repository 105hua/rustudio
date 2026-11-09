//! The rustudio logo (original artwork); every other icon is a Tabler glyph (src/ti.rs).

use egui::{Painter, Rect, Sense, Vec2};

/// The same embedded artwork is used in the chrome and native window icon.
const LOGO_PNG: &[u8] = include_bytes!("../assets/icons/rustudio-256.png");

pub fn paint_logo(p: &Painter, r: Rect) {
    let id = egui::Id::new("rustudio-app-icon");
    let texture = p
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let rgba = logo_rgba(64);
            let image = egui::ColorImage::from_rgba_unmultiplied([64, 64], &rgba);
            let texture =
                p.ctx()
                    .load_texture("rustudio-app-icon", image, egui::TextureOptions::LINEAR);
            p.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    p.image(
        texture.id(),
        r,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

pub fn logo(ui: &mut egui::Ui, size: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_logo(ui.painter(), r);
}

/// RGBA pixels of the logo for the window icon.
pub fn logo_rgba(size: u32) -> Vec<u8> {
    let image = image::load_from_memory(LOGO_PNG)
        .expect("embedded Rustudio icon must be a valid PNG")
        .to_rgba8();
    image::imageops::resize(&image, size, size, image::imageops::FilterType::Lanczos3).into_raw()
}

/// Small icon for a class in the Explorer and dialogs: its own Tabler glyph
/// in its family's colour (src/class_icons.rs).
pub fn paint_class_icon(p: &Painter, r: Rect, class: &str) {
    let (name, color) = crate::class_icons::icon_for(class);
    crate::ti::paint(p, r.expand(1.0), name, color);
}

pub fn class_icon(ui: &mut egui::Ui, class: &str, size: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_class_icon(ui.painter(), r, class);
}
