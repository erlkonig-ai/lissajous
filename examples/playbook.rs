#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = ".." }
//! egui = "0.33"
//! egui-theme-switch = "0.4"
//! ```

use egui::Color32;
use egui::{self};
use lissajous::md;
use lissajous::notebook;
use lissajous::widgets;
use lissajous::NotebookCtx;

fn to_hex(c: Color32) -> String {
    let r = c.r();
    let g = c.g();
    let b = c.b();
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn ral_lookup(code: u16) -> Option<(&'static str, Color32)> {
    lissajous::themes::ral::RAL_COLORS
        .iter()
        .find(|(num, _, _)| *num == code)
        .map(|(_, name, color)| (*name, *color))
}

fn ral_codes() -> &'static [u16] {
    static CODES: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    CODES.get_or_init(|| {
        let mut codes: Vec<u16> = lissajous::themes::ral::RAL_COLORS
            .iter()
            .map(|(code, _, _)| *code)
            .collect();
        codes.sort_unstable();
        codes
    })
}

fn closest_ral_code(current: u16, proposed: u16) -> u16 {
    let codes = ral_codes();
    if codes.is_empty() {
        return proposed;
    }

    match codes.binary_search(&proposed) {
        Ok(index) => codes[index],
        Err(insertion) => match proposed.cmp(&current) {
            std::cmp::Ordering::Less => {
                if insertion == 0 {
                    codes[0]
                } else {
                    codes[insertion - 1]
                }
            }
            std::cmp::Ordering::Greater => {
                if insertion >= codes.len() {
                    codes[codes.len() - 1]
                } else {
                    codes[insertion]
                }
            }
            std::cmp::Ordering::Equal => {
                if insertion == 0 {
                    return codes[0];
                }
                if insertion >= codes.len() {
                    return codes[codes.len() - 1];
                }

                let lower = codes[insertion - 1];
                let upper = codes[insertion];
                let dist_lower = (proposed as i32 - lower as i32).abs();
                let dist_upper = (upper as i32 - proposed as i32).abs();
                if dist_lower <= dist_upper {
                    lower
                } else {
                    upper
                }
            }
        },
    }
}

fn closest_ral_from_rgb(rgb: [u8; 3]) -> u16 {
    let (r, g, b) = (rgb[0] as i32, rgb[1] as i32, rgb[2] as i32);
    lissajous::themes::ral::RAL_COLORS
        .iter()
        .map(|(code, _, color)| {
            let dr = r - color.r() as i32;
            let dg = g - color.g() as i32;
            let db = b - color.b() as i32;
            let dist2 = (dr * dr + dg * dg + db * db) as u32;
            (*code, dist2)
        })
        .min_by_key(|(_, dist2)| *dist2)
        .map(|(code, _)| code)
        .unwrap_or(0)
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GiB", b / GB)
    } else if b >= MB {
        format!("{:.2} MiB", b / MB)
    } else if b >= KB {
        format!("{:.2} KiB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn bucket_label(exp: u32) -> String {
    let start = 1u64 << exp;
    if start >= (1u64 << 30) {
        format!("{}G", start >> 30)
    } else if start >= (1u64 << 20) {
        format!("{}M", start >> 20)
    } else if start >= (1u64 << 10) {
        format!("{}K", start >> 10)
    } else {
        format!("{start}B")
    }
}

fn color_chip(ui: &mut egui::Ui, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, color);
        painter.rect_stroke(
            rect,
            0.0,
            ui.visuals().window_stroke,
            egui::StrokeKind::Inside,
        );
    }

    response
}

fn ral_cell(ui: &mut egui::Ui, code: u16) {
    let Some((name, color)) = ral_lookup(code) else {
        ui.monospace(format!("RAL {code}"));
        return;
    };

    ui.horizontal(|ui| {
        let hex = to_hex(color);
        let tooltip = format!("RAL {code} — {name}\n{hex}");
        color_chip(ui, color).on_hover_text(tooltip);
        ui.monospace(format!("RAL {code}"));
    });
}

fn hex_cell(ui: &mut egui::Ui, color: Color32) {
    let hex = to_hex(color);
    ui.horizontal(|ui| {
        color_chip(ui, color).on_hover_text(hex.clone());
        ui.monospace(hex);
    });
}

// local blend util for this playbook so it's independent from themes internals
fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let r = (a.r() as f32 * (1.0 - t) + b.r() as f32 * t).round() as u8;
    let g = (a.g() as f32 * (1.0 - t) + b.g() as f32 * t).round() as u8;
    let bch = (a.b() as f32 * (1.0 - t) + b.b() as f32 * t).round() as u8;
    Color32::from_rgb(r, g, bch)
}

fn paint_hatching(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let spacing = 8.0;
    let stroke = egui::Stroke::new(1.0, color);

    let h = rect.height();
    let mut x = rect.left() - h;
    while x < rect.right() + h {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x + h, rect.bottom())],
            stroke,
        );
        x += spacing;
    }
}

fn rgb_histogram_editor_size(ui: &egui::Ui) -> egui::Vec2 {
    let desired_width = 240.0;
    let plot_height = 72.0;
    let font_id = egui::TextStyle::Small.resolve(ui.style());
    let tick_len = 4.0;
    let tick_pad = 2.0;
    let text_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
    let label_row_h = tick_len + tick_pad + text_height;
    egui::vec2(desired_width, plot_height + label_row_h)
}

#[derive(Clone, Copy, Debug, Default)]
struct RgbHistogramEditorResult {
    changed: bool,
    interaction_ended: bool,
}

fn rgb_histogram_editor(ui: &mut egui::Ui, rgb: &mut [u8; 3]) -> RgbHistogramEditorResult {
    let desired_width = rgb_histogram_editor_size(ui).x;
    let plot_height = 72.0;
    let y_segments = 5_u64;
    let y_max = 100_u64;
    let max_x_labels = 3_usize;

    let font_id = egui::TextStyle::Small.resolve(ui.style());
    let tick_len = 4.0;
    let tick_pad = 2.0;
    let text_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
    let label_row_h = tick_len + tick_pad + text_height;

    let total_h = plot_height + label_row_h;
    let (outer_rect, response) =
        ui.allocate_exact_size(egui::vec2(desired_width, total_h), egui::Sense::hover());
    if !ui.is_rect_visible(outer_rect) {
        return RgbHistogramEditorResult::default();
    }

    let visuals = ui.visuals();
    let background = visuals.window_fill;
    let outline = visuals.widgets.noninteractive.bg_stroke.color;
    let ink = visuals.widgets.noninteractive.fg_stroke.color;
    let stroke = egui::Stroke::new(1.0, outline);
    let grid_color = blend(background, ink, 0.22);

    let y_ticks: Vec<u64> = (0..=y_segments)
        .map(|i| (y_max / y_segments).saturating_mul(i))
        .collect();

    let y_label_width = ui.fonts_mut(|fonts| {
        y_ticks
            .iter()
            .map(|value| {
                fonts
                    .layout_no_wrap(format!("{value}"), font_id.clone(), ink)
                    .size()
                    .x
            })
            .fold(0.0, f32::max)
    });
    let y_axis_w = (y_label_width + 10.0).clamp(24.0, 80.0);
    let y_axis_pad = 6.0;

    let plot_rect = egui::Rect::from_min_max(
        egui::pos2(
            (outer_rect.left() + y_axis_w + y_axis_pad).min(outer_rect.right()),
            outer_rect.top(),
        ),
        egui::pos2(outer_rect.right(), outer_rect.bottom() - label_row_h),
    );
    let plot_area = plot_rect.shrink(4.0);

    let painter = ui.painter().with_clip_rect(outer_rect);
    painter.rect_stroke(plot_rect, 0.0, stroke, egui::StrokeKind::Inside);

    for value in &y_ticks {
        let frac = (*value as f64 / y_max as f64) as f32;
        let y = plot_area.bottom() - frac * plot_area.height();

        painter.line_segment(
            [
                egui::pos2(plot_area.left(), y),
                egui::pos2(plot_area.right(), y),
            ],
            egui::Stroke::new(1.0, grid_color),
        );
        painter.text(
            egui::pos2(plot_rect.left() - 4.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{value}"),
            font_id.clone(),
            ink,
        );
    }

    if !plot_area.is_positive() {
        return RgbHistogramEditorResult::default();
    }

    let channel_colors = [
        lissajous::themes::ral(3020),
        lissajous::themes::ral(6024),
        lissajous::themes::ral(5005),
    ];
    let channel_names = ["R", "G", "B"];

    let bucket_count = 3_usize;
    let gap = 2.0;
    let bar_w = ((plot_area.width() - gap * (bucket_count.saturating_sub(1) as f32))
        / bucket_count as f32)
        .max(1.0);

    let mut changed = false;
    let mut interaction_ended = false;

    for i in 0..bucket_count {
        let x0 = plot_area.left() + i as f32 * (bar_w + gap);
        let x1 = (x0 + bar_w).min(plot_area.right());

        let column_rect = egui::Rect::from_min_max(
            egui::pos2(x0, plot_area.top()),
            egui::pos2(x1, plot_area.bottom()),
        );
        if !column_rect.is_positive() {
            continue;
        }

        let id = response.id.with(("rgb_histogram_bar", i));
        let resp = ui.interact(column_rect, id, egui::Sense::click_and_drag());
        let hovered = resp.hovered();
        let dragged = resp.dragged();
        let drag_stopped = resp.drag_stopped();
        let clicked = resp.clicked();

        if hovered || dragged {
            ui.output_mut(|output| output.cursor_icon = egui::CursorIcon::ResizeVertical);
        }

        if clicked || dragged || drag_stopped {
            if let Some(pointer) = resp.interact_pointer_pos() {
                let t = ((plot_area.bottom() - pointer.y) / plot_area.height()).clamp(0.0, 1.0);
                let next = (t * 255.0).round() as u8;
                if rgb[i] != next {
                    rgb[i] = next;
                    changed = true;
                }
            }
        }

        interaction_ended |= clicked || drag_stopped;

        let value = rgb[i] as u64;
        let pct = ((value as f64 / 255.0) * 100.0).round() as u64;
        let tooltip = format!("{}: {value} / 255 ({pct}%)", channel_names[i]);
        let _ = resp.on_hover_text(tooltip);

        if value == 0 {
            continue;
        }

        let bar_h = ((value as f32 / 255.0) * plot_area.height()).clamp(1.0, plot_area.height());
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(x0, plot_area.bottom() - bar_h),
            egui::pos2(x1, plot_area.bottom()),
        );

        let stroke_color = if hovered || dragged {
            ui.visuals().selection.stroke.color
        } else {
            outline
        };
        let bar_stroke = egui::Stroke::new(1.0, stroke_color);

        let hatch_rect = bar_rect.shrink(1.0);
        if hatch_rect.is_positive() {
            paint_hatching(
                &painter.with_clip_rect(hatch_rect),
                hatch_rect,
                channel_colors[i],
            );
        }
        painter.rect_stroke(bar_rect, 0.0, bar_stroke, egui::StrokeKind::Inside);
    }

    if max_x_labels > 0 {
        let tick_top = plot_rect.bottom();

        for (i, name) in channel_names.iter().enumerate().take(bucket_count) {
            let x = plot_area.left() + i as f32 * (bar_w + gap) + bar_w * 0.5;
            painter.line_segment(
                [egui::pos2(x, tick_top), egui::pos2(x, tick_top + tick_len)],
                egui::Stroke::new(1.0, outline),
            );
            painter.text(
                egui::pos2(x, tick_top + tick_len + tick_pad),
                egui::Align2::CENTER_TOP,
                *name,
                font_id.clone(),
                ink,
            );
        }
    }

    RgbHistogramEditorResult {
        changed,
        interaction_ended,
    }
}

#[derive(Debug)]
struct PaletteState {
    ral_code: u16,
    rgb: [u8; 3],
}

impl Default for PaletteState {
    fn default() -> Self {
        let color = lissajous::themes::ral(7047);
        Self {
            ral_code: 7047_u16,
            rgb: [color.r(), color.g(), color.b()],
        }
    }
}

#[derive(Debug)]
struct WidgetPlaybookState {
    progress: f32,
    level_percent: f64,
    toggle_on: bool,
    metric_bytes: bool,
    radio_choice: RadioChoice,
    line_text: String,
    multi_text: String,
    focus_target: FocusTarget,
}

impl Default for WidgetPlaybookState {
    fn default() -> Self {
        Self {
            progress: 0.5,
            level_percent: 50.0,
            toggle_on: false,
            metric_bytes: false,
            radio_choice: RadioChoice::default(),
            line_text: "Short prompt".to_string(),
            multi_text: "First line\nSecond line\nThird line".to_string(),
            focus_target: FocusTarget::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RadioChoice {
    Alpha,
    Beta,
    Gamma,
}

impl Default for RadioChoice {
    fn default() -> Self {
        Self::Alpha
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    None,
    NumberField,
    SingleLine,
    Multiline,
}

impl Default for FocusTarget {
    fn default() -> Self {
        Self::None
    }
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    let padding = lissajous::cards::DEFAULT_CARD_PADDING;
    nb.view(move |ctx| {
        // Introduction
        md!(
            ctx,
            "# Palette Playbook\n\nBase tokens map semantic roles → RAL paint chips. Derived colors are small blends on top."
        );
    });

    nb.view(move |ctx| {
        ctx.with_padding(padding, |ctx| {
            let light_foreground = lissajous::themes::ral(9011);
            let light_background = lissajous::themes::ral(7047);
            let light_surface = lissajous::themes::ral(7047);

            let dark_foreground = lissajous::themes::ral(9003);
            let dark_background = lissajous::themes::ral(7046);
            let dark_surface = lissajous::themes::ral(7047);

            // Derived samples (same rules used in `themes::industrial`)
            let light_surface_muted = blend(light_surface, light_background, 0.2);
            let light_border = blend(light_foreground, light_background, 0.4);
            let light_control_fill_hover = blend(light_background, light_foreground, 0.05);

            let dark_surface_muted = blend(dark_surface, dark_background, 0.2);
            let dark_border = blend(dark_foreground, dark_background, 0.4);
            let dark_control_fill_hover = blend(dark_background, dark_foreground, 0.05);

            ctx.label(egui::RichText::new("TOKENS").monospace().strong());
            egui::Grid::new("palette_tokens")
                .num_columns(3)
                .spacing(egui::vec2(16.0, 6.0))
                .show(ctx.ui_mut(), |ui| {
                    ui.label("");
                    ui.monospace("LIGHT");
                    ui.monospace("DARK");
                    ui.end_row();

                    ui.monospace("FOREGROUND");
                    ral_cell(ui, 9011);
                    ral_cell(ui, 9003);
                    ui.end_row();

                    ui.monospace("BACKGROUND");
                    ral_cell(ui, 7047);
                    ral_cell(ui, 7046);
                    ui.end_row();

                    ui.monospace("SURFACE");
                    ral_cell(ui, 7047);
                    ral_cell(ui, 7047);
                    ui.end_row();

                    ui.monospace("ACCENT");
                    ral_cell(ui, 2009);
                    ral_cell(ui, 2009);
                    ui.end_row();
                });

            ctx.ui_mut()
                .collapsing(egui::RichText::new("DERIVED").monospace(), |ui| {
                    egui::Grid::new("palette_derived")
                        .num_columns(3)
                        .spacing(egui::vec2(16.0, 6.0))
                        .show(ui, |ui| {
                            ui.label("");
                            ui.monospace("LIGHT");
                            ui.monospace("DARK");
                            ui.end_row();

                            ui.monospace("BORDER (FG/BG 0.4)");
                            hex_cell(ui, light_border);
                            hex_cell(ui, dark_border);
                            ui.end_row();

                            ui.monospace("MUTED SURFACE (S/BG 0.2)");
                            hex_cell(ui, light_surface_muted);
                            hex_cell(ui, dark_surface_muted);
                            ui.end_row();

                            ui.monospace("HOVER (BG/FG 0.05)");
                            hex_cell(ui, light_control_fill_hover);
                            hex_cell(ui, dark_control_fill_hover);
                            ui.end_row();
                        });
                });

            ctx.add_space(12.0);
            ctx.label(egui::RichText::new("SUBWAY LINES").monospace().strong());
            ctx.add_space(6.0);
            let subway_palette = [1003, 2010, 3001, 4008, 5005, 6032, 3014];
            ctx.horizontal_wrapped(|ctx| {
                for code in subway_palette {
                    ral_cell(ctx, code);
                }
            });
        });
    });

    let _palette_state = nb.state(
        "palette_state",
        PaletteState::default(),
        move |ctx, state| {
            ctx.with_padding(padding, |ctx| {
                ctx.label(egui::RichText::new("RAL PICKER").monospace().strong());
                ctx.add_space(12.0);

                let histogram_size = rgb_histogram_editor_size(ctx);
                let preview_size = egui::vec2(histogram_size.y, histogram_size.y);

                ctx.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ctx| {
                    let (preview_rect, preview_resp) =
                        ctx.allocate_exact_size(preview_size, egui::Sense::hover());
                    ctx.add_space(16.0);

                    let rgb_edit = rgb_histogram_editor(ctx, &mut state.rgb);
                    if rgb_edit.changed || rgb_edit.interaction_ended {
                        state.ral_code = closest_ral_from_rgb(state.rgb);
                    }
                    if rgb_edit.interaction_ended {
                        if let Some((_, color)) = ral_lookup(state.ral_code) {
                            state.rgb = [color.r(), color.g(), color.b()];
                        }
                    }

                    ctx.add_space(24.0);
                    ctx.ui_mut().vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.monospace("RAL");
                            let ral_response = ui.add(
                                widgets::NumberField::new(&mut state.ral_code)
                                    .constrain_value(&|current, proposed| {
                                        let proposed = proposed.clamp(0u16, 9999u16);
                                        closest_ral_code(current, proposed)
                                    })
                                    .update_while_editing(false)
                                    .speed(0.25),
                            );

                            if ral_response.changed() {
                                if let Some((_, color)) = ral_lookup(state.ral_code) {
                                    state.rgb = [color.r(), color.g(), color.b()];
                                }
                            }
                        });

                        ui.add_space(8.0);

                        let code = state.ral_code;
                        if let Some((name, ral_color)) = ral_lookup(code) {
                            ui.label(name);
                            ui.monospace(to_hex(ral_color));
                        } else {
                            ui.label(egui::RichText::new("Unknown RAL code").monospace());
                        }
                    });

                    let color = Color32::from_rgb(state.rgb[0], state.rgb[1], state.rgb[2]);
                    let hex = to_hex(color);
                    if ctx.is_rect_visible(preview_rect) {
                        ctx.painter().rect_filled(preview_rect, 0.0, color);
                        let window_stroke = ctx.visuals().window_stroke;
                        ctx.painter().rect_stroke(
                            preview_rect,
                            0.0,
                            window_stroke,
                            egui::StrokeKind::Inside,
                        );
                    }

                    let code = state.ral_code;
                    let name = ral_lookup(code)
                        .map(|(name, _)| name)
                        .unwrap_or("Unknown RAL");
                    let tooltip = format!("RAL {code} — {name}\n{hex}");
                    let _ = preview_resp.on_hover_text(tooltip);
                });
            });
        },
    );

    nb.view(move |ctx| {
        md!(
            ctx,
            "## Widget Playbook\n\nA quick showcase of our custom widgets. The value is normalized to `[0, 1]`."
        );
    });

    let widget_state = nb.state(
        "widget_state",
        WidgetPlaybookState::default(),
        move |ctx, state| {
            ctx.with_padding(padding, |ctx| {
                ctx.label(egui::RichText::new("BUTTONS").monospace().strong());
                ctx.horizontal(|ctx| {
                    let _ = ctx.add(widgets::Button::new("BUTTON"));
                    let _ = ctx.add(widgets::Button::new("SMALL"));
                    ctx.add_enabled(false, widgets::Button::new("DISABLED"));
                    let _ = ctx.add(widgets::Button::new("SELECTED").selected(true));
                    let _ = ctx.add(widgets::Button::new("TOGGLE").on(&mut state.toggle_on));
                });

                ctx.add_space(12.0);
                ctx.label(egui::RichText::new("CHOICE TOGGLE").monospace().strong());
                ctx.horizontal(|ctx| {
                    ctx.add(widgets::ChoiceToggle::binary(
                        &mut state.metric_bytes,
                        "COUNT",
                        "BYTES",
                    ));
                });

                ctx.add_space(12.0);
                ctx.label(egui::RichText::new("RADIO").monospace().strong());
                ctx.horizontal(|ctx| {
                    ctx.add(widgets::RadioButton::new(
                        &mut state.radio_choice,
                        RadioChoice::Alpha,
                        "ALPHA",
                    ));
                    ctx.add(widgets::RadioButton::new(
                        &mut state.radio_choice,
                        RadioChoice::Beta,
                        "BETA",
                    ));
                    ctx.add(widgets::RadioButton::new(
                        &mut state.radio_choice,
                        RadioChoice::Gamma,
                        "GAMMA",
                    ));
                });

                ctx.add_space(12.0);
                ctx.label(egui::RichText::new("ROW ALIGNMENT").monospace().strong());
                let mut number_id = None;
                ctx.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ctx| {
                    let _ = widgets::row_label(ctx, egui::RichText::new("LEVEL").monospace());
                    let slider_response = ctx.add(
                        widgets::Slider::new(&mut state.progress, 0.0..=1.0).show_value(false),
                    );
                    if slider_response.changed() {
                        state.level_percent = state.progress as f64 * 100.0;
                    }
                    ctx.add_space(8.0);
                    let number_response = ctx.add(
                        widgets::NumberField::new(&mut state.level_percent)
                            .suffix("%")
                            .min_decimals(0)
                            .max_decimals(1)
                            .update_while_editing(false)
                            .constrain_value(&|_, proposed| proposed.clamp(0.0, 100.0)),
                    );
                    if number_response.changed() {
                        state.progress = (state.level_percent / 100.0) as f32;
                    }
                    number_id = Some(number_response.id);
                    ctx.add_space(8.0);
                    let _ = ctx.add(widgets::Button::new("APPLY"));
                });

                ctx.add_space(12.0);
                ctx.label(egui::RichText::new("FOCUS").monospace().strong());
                ctx.horizontal(|ctx| {
                    ctx.add(
                        widgets::ChoiceToggle::new(&mut state.focus_target)
                            .choice(FocusTarget::None, "NONE")
                            .choice(FocusTarget::NumberField, "NUMBER")
                            .choice(FocusTarget::SingleLine, "LINE")
                            .choice(FocusTarget::Multiline, "MULTI"),
                    );
                });

                ctx.add_space(12.0);
                ctx.label(egui::RichText::new("TEXT FIELDS").monospace().strong());
                let line_response = ctx.add(widgets::TextField::singleline(&mut state.line_text));
                ctx.add_space(8.0);
                let multi_response = ctx.add(widgets::TextField::multiline(&mut state.multi_text));

                let focus_id = match state.focus_target {
                    FocusTarget::None => None,
                    FocusTarget::NumberField => number_id,
                    FocusTarget::SingleLine => Some(line_response.id),
                    FocusTarget::Multiline => Some(multi_response.id),
                };
                if let Some(id) = focus_id {
                    ctx.memory_mut(|mem| mem.request_focus(id));
                }
            });
        },
    );

    nb.view(move |ctx| {
        let mut state = widget_state.read_mut(ctx);
        ctx.with_padding(padding, |ctx| {
            ctx.label(egui::RichText::new("SLIDER + METERS").monospace().strong());

            let slider_response =
                ctx.add(widgets::Slider::new(&mut state.progress, 0.0..=1.0).text("LEVEL"));
            if slider_response.changed() {
                state.level_percent = state.progress as f64 * 100.0;
            }
            let progress = state.progress;

            ctx.monospace(format!("Value: {progress:.3}"));
            ctx.add(
                widgets::ProgressBar::new(progress)
                    .text("OUTPUT")
                    .scale_percent(),
            );

            let green = lissajous::themes::ral(6024);
            let yellow = lissajous::themes::ral(1023);
            let red = lissajous::themes::ral(3020);

            ctx.add(
                widgets::ProgressBar::new(progress)
                    .text("SIGNAL")
                    .segments(60)
                    .scale_labels([(0.0, "0 (off)"), (0.7, "70!"), (0.9, "90"), (1.0, "100")])
                    .zone(0.0..=0.7, green)
                    .zone(0.7..=0.9, yellow)
                    .zone(0.9..=1.0, red),
            );
        });
    });

    nb.view(move |ctx| {
        let state = widget_state.read(ctx);
        let (progress, metric_bytes) = (state.progress, state.metric_bytes);
        ctx.with_padding(padding, |ctx| {
            ctx.label(egui::RichText::new("HISTOGRAM").monospace().strong());
            ctx.monospace("Uses COUNT/BYTES + slider to shift the synthetic distribution.");

            let y_axis = if metric_bytes {
                widgets::HistogramYAxis::Bytes
            } else {
                widgets::HistogramYAxis::Count
            };

            let min_exp = 6u32;
            let max_exp = 24u32;
            let exp_span = (max_exp - min_exp).max(1) as f32;
            let center = min_exp as f32 + progress * exp_span;

            let mut buckets = Vec::new();
            for exp in min_exp..=max_exp {
                let dist = (exp as f32 - center).abs();
                let t = (1.0 - dist / exp_span).clamp(0.0, 1.0);
                let count = (180.0 * (t * t)) as u64;
                let bytes = count.saturating_mul(1u64 << exp);
                let value = if metric_bytes { bytes } else { count };
                let label = bucket_label(exp);
                buckets.push(
                    widgets::HistogramBucket::new(value, label.clone()).tooltip(format!(
                        "bucket: {label}\ncount: {count}\nbytes: {}",
                        format_bytes(bytes)
                    )),
                );
            }

            ctx.push_id("histogram-demo", |ctx| {
                ctx.add(widgets::Histogram::new(&buckets, y_axis).plot_height(96.0));
            });
        });
    });
}
