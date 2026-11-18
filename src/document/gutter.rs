//! Колонка с номерами строк редактора.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::settings::AppSettings;

/// Настраивает колонку с номерами строк (Gutter).
///
/// Использует `GtkDrawingArea` и Cairo. Цвета и шрифт берутся из общих
/// настроек, поэтому при смене темы достаточно вызвать `queue_draw`.
pub(super) fn setup_line_numbers_gutter(
    text_view: &gtk4::TextView,
    settings: &Rc<RefCell<AppSettings>>,
) -> gtk4::DrawingArea {
    let gutter = gtk4::DrawingArea::new();
    gutter.set_content_width(48);

    // WeakRef разрывает цикл TextView -> gutter -> draw func -> TextView.
    // Иначе каждая закрытая вкладка навсегда удерживала бы свой TextView.
    let text_view_weak = text_view.downgrade();
    let settings_for_draw = settings.clone();

    gutter.set_draw_func(move |area, cr, width, height| {
        let Some(text_view) = text_view_weak.upgrade() else {
            return;
        };

        let buffer = text_view.buffer();
        let total_lines = buffer.line_count();

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

        // Метрики "0" нужны, чтобы выровнять номер по центру строки,
        // как обычный текст редактора.
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

            if let Ok(extents) = cr.text_extents(&num_str) {
                let x = (width as f64) - 8.0 - extents.width() - extents.x_bearing();
                let line_height = (h_buf as f64).max(font_size);
                let y = (win_y as f64)
                    + (line_height - glyph_height) * 0.5
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
