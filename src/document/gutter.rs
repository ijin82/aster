//! Колонка с номерами строк редактора.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::settings::AppSettings;

use super::diagnostics::{Diagnostic, Severity};

/// Настраивает колонку с номерами строк (Gutter).
///
/// Использует `GtkDrawingArea` и Cairo. Цвета и шрифт берутся из общих
/// настроек, поэтому при смене темы достаточно вызвать `queue_draw`.
///
/// Дополнительно gutter рисует цветную полосу у строк с диагностикой:
/// пустые строки (MD012, MD047) нельзя подчеркнуть в тексте, и без
/// маркера пользователь не увидел бы о них предупреждение.
pub(super) fn setup_line_numbers_gutter(
    text_view: &gtk4::TextView,
    settings: &Rc<RefCell<AppSettings>>,
    diagnostics: &Rc<RefCell<Vec<Diagnostic>>>,
) -> gtk4::DrawingArea {
    let gutter = gtk4::DrawingArea::new();
    gutter.set_content_width(48);

    // WeakRef разрывает цикл TextView -> gutter -> draw func -> TextView.
    // Иначе каждая закрытая вкладка навсегда удерживала бы свой TextView.
    let text_view_weak = text_view.downgrade();
    let settings_for_draw = settings.clone();
    let diagnostics_for_draw = diagnostics.clone();

    gutter.set_draw_func(move |area, cr, width, height| {
        let Some(text_view) = text_view_weak.upgrade() else {
            return;
        };

        let buffer = text_view.buffer();
        let total_lines = buffer.line_count();
        let diagnostics = diagnostics_for_draw.borrow();

        let settings = settings_for_draw.borrow();
        let theme = settings.theme.clone();
        let style_mgr = adw::StyleManager::default();
        let is_dark = style_mgr.is_dark();

        // CSS редактора задает размер в pt. Cairo работает в пикселях,
        // поэтому переводим пункты в пиксели по стандартным 96 DPI.
        let font_size = settings.editor_font_size as f64 * 96.0 / 72.0;
        let font_family = settings.editor_font_family.clone();
        cr.select_font_face(
            &font_family,
            gtk4::cairo::FontSlant::Normal,
            gtk4::cairo::FontWeight::Normal,
        );
        cr.set_font_size(font_size);

        // Ширина колонки зависит от количества цифр и реальной ширины
        // цифры в выбранном шрифте, а не от фиксированных 9 px.
        let digit_layout = text_view.create_pango_layout(Some("0"));
        let (digit_width_px, _) = digit_layout.pixel_size();
        let digit_width = (digit_width_px.max(1) as f64).max(font_size * 0.5);
        let digits = format!("{total_lines}").len().max(3);
        let needed_width = (digits as f64 * digit_width + 20.0).max(40.0) as i32;
        if area.content_width() != needed_width {
            area.set_content_width(needed_width);
        }

        let (bg_r, bg_g, bg_b, num_r, num_g, num_b) = match theme.as_str() {
            "dracula" => (0.15, 0.16, 0.21, 0.40, 0.42, 0.54),
            "kimbie_dark" => (0.11, 0.08, 0.05, 0.50, 0.42, 0.32),
            "solarized_dark" => (0.00, 0.14, 0.18, 0.35, 0.43, 0.46),
            "solarized_light" => (0.93, 0.90, 0.82, 0.58, 0.63, 0.63),
            "dark" => (0.04, 0.05, 0.07, 0.38, 0.42, 0.47),
            "light" => (0.97, 0.98, 0.99, 0.55, 0.58, 0.62),
            _ => {
                if is_dark {
                    (0.04, 0.05, 0.07, 0.38, 0.42, 0.47)
                } else {
                    (0.97, 0.98, 0.99, 0.55, 0.58, 0.62)
                }
            }
        };

        cr.set_source_rgb(bg_r, bg_g, bg_b);
        let _ = cr.paint();

        // Тонкая вертикальная линия между номерами и текстом.
        cr.set_source_rgba(num_r, num_g, num_b, 0.25);
        cr.set_line_width(1.0);
        cr.move_to((width as f64) - 0.5, 0.0);
        cr.line_to((width as f64) - 0.5, height as f64);
        let _ = cr.stroke();

        let (_, top_buf_y) =
            text_view.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, 0);
        let (_, bottom_buf_y) =
            text_view.window_to_buffer_coords(gtk4::TextWindowType::Left, 0, height);

        let top_buf_y = top_buf_y.max(0);
        let (mut iter, _) = text_view.line_at_y(top_buf_y);

        cr.set_source_rgb(num_r, num_g, num_b);

        // Метрики "0" нужны, чтобы выровнять номер по центру первого
        // визуального ряда. Высота всей логической строки здесь не
        // подходит: из-за мягкого переноса номер уезжал бы в середину
        // многострочного абзаца.
        let (glyph_height, glyph_y_bearing) = cr
            .text_extents("0")
            .map(|extents| (extents.height(), extents.y_bearing()))
            .unwrap_or((font_size, -font_size * 0.8));

        loop {
            let (y_buf, h_buf) = text_view.line_yrange(&iter);
            let (_, win_y) =
                text_view.buffer_to_window_coords(gtk4::TextWindowType::Left, 0, y_buf);

            let line_num = iter.line() + 1;
            let num_str = format!("{line_num}");

            // Высота одного визуального ряда, а не всей логической строки:
            // номер должен стоять у первой строки перенесенного текста.
            let visual_line_height = text_view.iter_location(&iter).height().max(1) as f64;

            // Цветная полоса слева от номера показывает строки с
            // диагностикой. Красный цвет — ошибка, янтарный — нарушение
            // стиля. Полоса не перекрывает цифры и не требует менять
            // ширину gutter.
            let line_index = iter.line();
            let has_error = diagnostics
                .iter()
                .any(|d| d.line == line_index && d.severity == Severity::Error);
            let has_warning = !has_error
                && diagnostics
                    .iter()
                    .any(|d| d.line == line_index && d.severity == Severity::Warning);

            if has_error || has_warning {
                let (r, g, b) = if has_error {
                    (0.90, 0.20, 0.20)
                } else {
                    (0.82, 0.55, 0.05)
                };
                cr.set_source_rgba(r, g, b, 0.9);
                cr.rectangle(
                    0.0,
                    win_y as f64 + 2.0,
                    3.0,
                    (visual_line_height - 4.0).max(2.0),
                );
                let _ = cr.fill();
                cr.set_source_rgb(num_r, num_g, num_b);
            }

            if let Ok(extents) = cr.text_extents(&num_str) {
                let x = (width as f64) - 8.0 - extents.width() - extents.x_bearing();
                let y = (win_y as f64)
                    + (visual_line_height - glyph_height) * 0.5
                    - glyph_y_bearing;
                cr.move_to(x, y);
                let _ = cr.show_text(&num_str);
            }

            if y_buf + h_buf > bottom_buf_y || !iter.forward_line() {
                break;
            }
        }
    });

    if settings.borrow().show_line_numbers {
        text_view.set_gutter(gtk4::TextWindowType::Left, Some(&gutter));
    } else {
        text_view.set_gutter(gtk4::TextWindowType::Left, None::<&gtk4::DrawingArea>);
    }

    gutter
}
