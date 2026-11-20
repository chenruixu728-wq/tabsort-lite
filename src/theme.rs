use eframe::egui::{self, Color32, RichText};

pub const INK: Color32 = Color32::from_rgb(7, 26, 49);
pub const MUTED: Color32 = Color32::from_rgb(95, 109, 119);
pub const CANVAS: Color32 = Color32::from_rgb(252, 251, 248);
pub const STONE: Color32 = Color32::from_rgb(245, 243, 238);
pub const BLUE: Color32 = Color32::from_rgb(195, 223, 254);
pub const MINT: Color32 = Color32::from_rgb(186, 240, 236);
pub const LILAC: Color32 = Color32::from_rgb(212, 194, 255);

pub fn configure(context: &egui::Context) {
    context.set_theme(egui::Theme::Light);
    let mut style = (*context.style()).clone();
    style.visuals = egui::Visuals::light();
    let visuals = &mut style.visuals;
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = CANVAS;
    visuals.window_fill = CANVAS;
    visuals.faint_bg_color = STONE;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.selection.bg_fill = BLUE;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, INK);
    visuals.window_corner_radius = 24.into();
    visuals.window_stroke = egui::Stroke::NONE;
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 36,
        spread: 0,
        color: Color32::from_black_alpha(24),
    };
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = 12.into();
        widget.fg_stroke.color = INK;
    }
    visuals.widgets.inactive.bg_fill = STONE;
    visuals.widgets.inactive.weak_bg_fill = STONE;
    visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(225, 227, 228));
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(231, 239, 247);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(231, 239, 247);
    visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.5_f32, Color32::from_rgb(107, 177, 255));
    visuals.widgets.active.bg_fill = BLUE;
    visuals.widgets.active.weak_bg_fill = BLUE;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5_f32, INK);
    visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(216, 220, 224));
    style.spacing.item_spacing = egui::vec2(10.0, 12.0);
    style.spacing.button_padding = egui::vec2(16.0, 10.0);
    style.spacing.interact_size.y = 36.0;
    style.spacing.window_margin = 24.into();
    style.animation_time = 0.15;
    for (text_style, size) in [
        (egui::TextStyle::Body, 14.0),
        (egui::TextStyle::Button, 14.0),
        (egui::TextStyle::Small, 12.0),
        (egui::TextStyle::Heading, 23.0),
    ] {
        style
            .text_styles
            .insert(text_style, egui::FontId::proportional(size));
    }
    context.set_style(style);
}

pub fn modal_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CANVAS)
        .corner_radius(24)
        .inner_margin(24)
        .shadow(egui::epaint::Shadow {
            offset: [0, 10],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha(28),
        })
}

pub fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(CANVAS).size(15.0))
        .fill(INK)
        .stroke(egui::Stroke::NONE)
        .corner_radius(16)
        .min_size(egui::vec2(118.0, 44.0))
}

pub fn badge(ui: &mut egui::Ui, label: &str, fill: Color32) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(12.0).color(INK));
        });
}

pub fn choice(ui: &mut egui::Ui, label: &str, selected: bool, size: egui::Vec2) -> egui::Response {
    let (fill, text, stroke) = if selected {
        (INK, CANVAS, egui::Stroke::NONE)
    } else {
        (STONE, INK, egui::Stroke::NONE)
    };
    ui.add_sized(
        size,
        egui::Button::new(RichText::new(label).color(text))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(12),
    )
}

pub fn card(ui: &mut egui::Ui, render: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .corner_radius(22)
        .inner_margin(20)
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(237, 237, 233)))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            render(ui);
        });
}

pub fn atmosphere(rect: egui::Rect) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    const STEPS: u32 = 24;
    for y in 0..=STEPS {
        for x in 0..=STEPS {
            let u = x as f32 / STEPS as f32;
            let v = y as f32 / STEPS as f32;
            let distance = (v * rect.height()).min((1.0 - v) * rect.height());
            let inset = if distance < 24.0 {
                24.0 - (24.0_f32.powi(2) - (24.0 - distance).powi(2)).sqrt()
            } else {
                0.0
            };
            let weight = ((u - 0.55).powi(2) + (v - 0.8).powi(2)).sqrt().min(1.0);
            let bottom = MINT.lerp_to_gamma(LILAC, u);
            let color = BLUE.lerp_to_gamma(bottom, (1.0 - weight) * 0.75);
            mesh.colored_vertex(
                egui::pos2(
                    rect.left() + inset + (rect.width() - 2.0 * inset) * u,
                    rect.top() + rect.height() * v,
                ),
                color,
            );
        }
    }
    for y in 0..STEPS {
        for x in 0..STEPS {
            let i = y * (STEPS + 1) + x;
            mesh.add_triangle(i, i + 1, i + STEPS + 1);
            mesh.add_triangle(i + 1, i + STEPS + 2, i + STEPS + 1);
        }
    }
    mesh.into()
}
