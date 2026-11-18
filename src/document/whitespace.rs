//! Отрисовка пробелов и табуляций поверх редактора.
//!
//! GTK TextView не умеет показывать пробельные символы. Рисуем их отдельным
//! прозрачным слоем поверх ScrolledWindow: точки для пробелов и горизонтальные
//! линии для табов. Слой не перехватывает мышь и не меняет текст буфера.

use gtk4::cairo::Context;
use gtk4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Подключает функцию рисования к прозрачному слою.
pub(super) fn setup_whitespace_layer(
    area: &gtk4::DrawingArea,
    text_view: &gtk4::TextView,
    visible: Rc<Cell<bool>>,
) {
    let text_view_weak = text_view.downgrade();
    area.set_draw_func(move |area, cr, _width, _height| {
        if !visible.get() {
            return;
        }
        let Some(text_view) = text_view_weak.upgrade() else {
            return;
        };
        draw_whitespace(area, cr, &text_view);
    });
}

/// Рисует пробелы и табы только в видимой области буфера.
fn draw_whitespace(area: &gtk4::DrawingArea, cr: &Context, text_view: &gtk4::TextView) {
    let visible = text_view.visible_rect();
    if visible.width() <= 0 || visible.height() <= 0 {
        return;
    }

    // `visible_rect()` может начинаться выше нуля из-за top_margin
    // редактора, а iter_at_location в координатах margin возвращает None.
    // Поэтому берем первую видимую строку через line_at_y, а не итератор
    // по координате.
    let (mut iter, _) = text_view.line_at_y(visible.y().max(0));
    let (end_line_iter, _) = text_view.line_at_y(visible.y() + visible.height());
    let end_line = end_line_iter.line();
    let visible_bottom = (visible.y() + visible.height()) as f64;

    // Смещение TextView внутри Overlay. buffer_to_window_coords уже учитывает
    // прокрутку, а это смещение — положение самого TextView.
    let origin = text_view
        .compute_point(area, &gtk4::graphene::Point::new(0.0, 0.0))
        .unwrap_or_else(|| gtk4::graphene::Point::new(0.0, 0.0));
    let origin_x = origin.x() as f64;
    let origin_y = origin.y() as f64;

    let mut color = text_view.color();
    color.set_alpha(0.45);
    cr.set_source_rgba(
        color.red() as f64,
        color.green() as f64,
        color.blue() as f64,
        color.alpha() as f64,
    );
    cr.set_line_width(1.0);

    while iter.line() <= end_line {
        let ch = iter.char();
        if ch == ' ' || ch == '\t' {
            let location = text_view.iter_location(&iter);
            if (location.y() as f64) > visible_bottom {
                break;
            }
            let mut next = iter.clone();
            next.forward_char();
            let next_location = text_view.iter_location(&next);

            // Пробел в конце перенесенной строки может оказаться уже на
            // следующей строке; такие позиции пропускаем.
            if location.y() == next_location.y() {
                // Widget, а не Text: слой лежит поверх всего TextView,
                // включая gutter. Text-координаты не содержат его ширину,
                // из-за чего маркеры уезжали влево на текст.
                let (x, y) = text_view.buffer_to_window_coords(
                    gtk4::TextWindowType::Widget,
                    location.x(),
                    location.y(),
                );
                let (next_x, _) = text_view.buffer_to_window_coords(
                    gtk4::TextWindowType::Widget,
                    next_location.x(),
                    next_location.y(),
                );

                let x = x as f64 + origin_x;
                let y = y as f64 + origin_y;
                let next_x = next_x as f64 + origin_x;
                let width = (next_x - x).max(0.0);
                let height = location.height().max(1) as f64;

                if ch == ' ' {
                    let dot_size = (height * 0.12).clamp(1.5, 2.5);
                    cr.rectangle(
                        x + width * 0.5 - dot_size * 0.5,
                        y + height * 0.55 - dot_size * 0.5,
                        dot_size,
                        dot_size,
                    );
                    let _ = cr.fill();
                } else {
                    let line_y = y + height * 0.6;
                    let line_end = (x + width - 2.0).max(x + 2.0);
                    cr.move_to(x + 2.0, line_y);
                    cr.line_to(line_end, line_y);
                    // Небольшая вертикальная засечка на конце таба.
                    cr.line_to(line_end, line_y - 3.0);
                    let _ = cr.stroke();
                }
            }
        }

        if !iter.forward_char() {
            break;
        }
    }

}
