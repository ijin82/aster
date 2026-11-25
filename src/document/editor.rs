//! Клавиатурные действия редактора, отступы и восстановление курсора.

use gtk4::prelude::*;
use gtk4::glib;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::settings::AppSettings;
use crate::shortcuts;

use super::get_file_canonical_key;

/// Восстанавливает позицию курсора для указанного файла.
pub(super) fn apply_restored_cursor_position(
    settings: &Rc<RefCell<AppSettings>>,
    buffer: &gtk4::TextBuffer,
    path: &Path,
) {
    let key = get_file_canonical_key(path);
    let (line, col) = settings
        .borrow()
        .file_cursor_positions
        .get(&key)
        .copied()
        .unwrap_or((0, 0));

    let line_count = buffer.line_count();
    let target_line = (line as i32).min(line_count.saturating_sub(1)).max(0);
    let mut iter = buffer
        .iter_at_line(target_line)
        .unwrap_or_else(|| buffer.start_iter());
    // `chars_in_line()` включает завершающий `\n`, поэтому ограничиваемся
    // длиной строки без перевода строки, иначе курсор уйдёт на следующую строку.
    let mut line_end = iter;
    line_end.forward_to_line_end();
    let valid_col = (col as i32).min(line_end.line_offset()).max(0);
    iter.set_line_offset(valid_col);
    buffer.place_cursor(&iter);
}

/// Прокручивает редактор к курсору в следующем цикле главного потока.
///
/// На момент загрузки файла окно еще может не иметь финальной разметки,
/// поэтому прокрутка выполняется через `idle_add_local_once`.
pub(super) fn schedule_cursor_scroll(
    text_view: &gtk4::TextView,
    editor_scroll: &gtk4::ScrolledWindow,
    soft_wrap: bool,
) {
    let text_view_clone = text_view.clone();
    let editor_scroll_clone = editor_scroll.clone();
    glib::idle_add_local_once(move || {
        let buf = text_view_clone.buffer();
        let insert_mark = buf.get_insert();
        let mut iter = buf.iter_at_mark(&insert_mark);
        if iter.line() == 0 {
            text_view_clone.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.0);
        } else {
            text_view_clone.scroll_to_iter(&mut iter, 0.0, true, 0.0, 0.5);
        }

        // В режиме переноса строк горизонтальная прокрутка не должна
        // сдвигать текст: это оставляет пустую полосу справа. Без переноса
        // горизонтальная прокрутка нужна для длинных строк.
        if soft_wrap {
            let adjustment = editor_scroll_clone.hadjustment();
            adjustment.set_value(0.0);
            glib::idle_add_local_once(move || adjustment.set_value(0.0));
        }
    });
}

/// Настраивает Tab/Shift+Tab и подавляет системный звук на границах текста.
pub(super) fn setup_editor_key_handling(
    text_view: &gtk4::TextView,
    editor_scroll: &gtk4::ScrolledWindow,
    settings: Rc<RefCell<AppSettings>>,
) {
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    // WeakRef разрывает цикл TextView -> controller -> handler -> TextView.
    let text_view_weak = text_view.downgrade();
    // ScrolledWindow владеет TextView. Сильная ссылка из controller в
    // ScrolledWindow создала бы цикл, поэтому храним слабую ссылку.
    let editor_scroll_weak = editor_scroll.downgrade();

    key_controller.connect_key_pressed(move |_, keyval, keycode, state| {
        let Some(text_view) = text_view_weak.upgrade() else {
            return glib::Propagation::Proceed;
        };

        let is_control = state.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
        let is_alt = state.contains(gtk4::gdk::ModifierType::ALT_MASK);
        let is_shift = state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);

        if is_control && !is_alt {
            let is_home =
                keyval == gtk4::gdk::Key::Home || keyval == gtk4::gdk::Key::KP_Home;
            let is_end = keyval == gtk4::gdk::Key::End || keyval == gtk4::gdk::Key::KP_End;

            // Ctrl+Home / Ctrl+End — начало и конец текущего документа.
            // Shift+Ctrl+Home/End не перехватываем, чтобы не ломать
            // стандартное выделение текста.
            if (is_home || is_end)
                && !state.contains(gtk4::gdk::ModifierType::SHIFT_MASK)
            {
                let Some(editor_scroll) = editor_scroll_weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let soft_wrap = settings.borrow().soft_wrap;
                let buffer = text_view.buffer();
                let mut iter = if is_home {
                    buffer.start_iter()
                } else {
                    buffer.end_iter()
                };
                buffer.place_cursor(&iter);
                scroll_to_edge(
                    &text_view,
                    &editor_scroll,
                    &mut iter,
                    is_home,
                    soft_wrap,
                );
                return glib::Propagation::Stop;
            }

            // Ctrl+Shift+Up / Ctrl+Shift+Down прокручивают редактор на одну
            // видимую строку, не перемещая курсор и не меняя выделение.
            // Обрабатываем здесь, а не через действия окна, потому что
            // прокрутка должна работать только при фокусе в редакторе и
            // учитывать реальную высоту строки с мягким переносом.
            let is_up = keyval == gtk4::gdk::Key::Up || keyval == gtk4::gdk::Key::KP_Up;
            let is_down =
                keyval == gtk4::gdk::Key::Down || keyval == gtk4::gdk::Key::KP_Down;
            if is_shift && (is_up || is_down) {
                if let Some(editor_scroll) = editor_scroll_weak.upgrade() {
                    scroll_editor_by_visual_line(&text_view, &editor_scroll, is_up);
                }
                return glib::Propagation::Stop;
            }

            return glib::Propagation::Proceed;
        }

        if is_alt {
            // Alt+D — дублировать текущую строку ниже. keycode переводится
            // в латинскую клавишу, поэтому комбинация работает и в русской
            // раскладке. Остальные Alt-комбинации не перехватываем.
            let shortcut_key = shortcuts::latin_char_for_keycode(keycode)
                .or_else(|| keyval.to_unicode().map(|ch| ch.to_ascii_lowercase()));
            if shortcut_key == Some('d') {
                duplicate_current_line(&text_view.buffer());
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }

        let is_forward_boundary = keyval == gtk4::gdk::Key::Down
            || keyval == gtk4::gdk::Key::KP_Down
            || keyval == gtk4::gdk::Key::Page_Down
            || keyval == gtk4::gdk::Key::KP_Page_Down
            || keyval == gtk4::gdk::Key::Right
            || keyval == gtk4::gdk::Key::KP_Right;
        let is_backward_boundary = keyval == gtk4::gdk::Key::Up
            || keyval == gtk4::gdk::Key::KP_Up
            || keyval == gtk4::gdk::Key::Page_Up
            || keyval == gtk4::gdk::Key::KP_Page_Up
            || keyval == gtk4::gdk::Key::Left
            || keyval == gtk4::gdk::Key::KP_Left;

        if is_forward_boundary || is_backward_boundary {
            let buffer = text_view.buffer();
            // При активном выделении (например, после Ctrl+A) стрелки
            // должны снимать выделение, а не блокироваться.
            if !buffer.has_selection() {
                let insert_mark = buffer.get_insert();
                let iter = buffer.iter_at_mark(&insert_mark);

                if is_forward_boundary && iter.is_end() {
                    return glib::Propagation::Stop;
                }
                if is_backward_boundary && iter.is_start() {
                    return glib::Propagation::Stop;
                }
            }
        }

        let is_tab = keyval == gtk4::gdk::Key::Tab;
        let is_iso_left_tab = keyval == gtk4::gdk::Key::ISO_Left_Tab;

        if !is_tab && !is_iso_left_tab {
            return glib::Propagation::Proceed;
        }

        let buffer = text_view.buffer();
        let indent_width = settings.borrow().indent_width.clamp(1, 8) as i32;

        if is_iso_left_tab || (is_tab && is_shift) {
            unindent_selection_or_cursor(&buffer, indent_width);
            glib::Propagation::Stop
        } else if is_tab && !is_shift {
            indent_selection_or_cursor(&buffer, indent_width);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });

    text_view.add_controller(key_controller);
}

/// Прокручивает редактор на одну видимую строку вверх или вниз.
///
/// Высота берется у символа первой видимой строки. В режиме мягкого
/// переноса `line_yrange` возвращает высоту всей логической строки, а
/// `iter_location` — высоту одного визуального ряда, поэтому прокрутка
/// остается плавной и не перескакивает через весь перенос.
fn scroll_editor_by_visual_line(
    text_view: &gtk4::TextView,
    editor_scroll: &gtk4::ScrolledWindow,
    up: bool,
) {
    let adjustment = editor_scroll.vadjustment();
    let visible = text_view.visible_rect();
    let probe_y = visible.y().max(0);

    let (iter, _) = text_view.line_at_y(probe_y);
    let line_height = text_view.iter_location(&iter).height().max(1) as f64;

    let lower = adjustment.lower();
    let upper = (adjustment.upper() - adjustment.page_size()).max(lower);
    let delta = if up { -line_height } else { line_height };
    let target = (adjustment.value() + delta).clamp(lower, upper);
    adjustment.set_value(target);
}

/// Прокручивает редактор к началу или концу документа.
///
/// После программной прокрутки горизонтальный adjustment возвращается
/// в ноль: иначе в режиме переноса строк справа появляется пустая полоса.
fn scroll_to_edge(
    text_view: &gtk4::TextView,
    editor_scroll: &gtk4::ScrolledWindow,
    iter: &mut gtk4::TextIter,
    is_home: bool,
    soft_wrap: bool,
) {
    let yalign = if is_home { 0.0 } else { 1.0 };
    let xalign = if is_home || soft_wrap { 0.0 } else { 1.0 };
    text_view.scroll_to_iter(iter, 0.0, true, xalign, yalign);

    if soft_wrap {
        let adjustment = editor_scroll.hadjustment();
        adjustment.set_value(0.0);
        glib::idle_add_local_once(move || adjustment.set_value(0.0));
    }
}

/// Дублирует строку под курсором и ставит курсор на копию.
///
/// Копия вставляется сразу после текущей строки. Если текущая строка
/// последняя и файл не заканчивается переводом строки, перед копией
/// добавляется `\n`. Курсор сохраняет колонку, поэтому повторный `Alt+D`
/// последовательно создает копии.
pub(super) fn duplicate_current_line(buffer: &gtk4::TextBuffer) {
    let insert_mark = buffer.get_insert();
    let cursor = buffer.iter_at_mark(&insert_mark);
    let line_index = cursor.line();
    let column = cursor.line_offset();

    let line_count = buffer.line_count();
    let Some(line_start) = buffer.iter_at_line(line_index) else {
        return;
    };
    let line_end = match buffer.iter_at_line(line_index + 1) {
        Some(mut next_line_start) => {
            next_line_start.backward_char();
            next_line_start
        }
        None => buffer.end_iter(),
    };
    let line_text = buffer.text(&line_start, &line_end, false);
    let line_len = line_text.chars().count() as i32;

    buffer.begin_user_action();

    // Для не последней строки вставляем копию перед началом следующей.
    // Для последней строки без завершающего перевода строки добавляем его
    // перед копией.
    let duplicate_start = if line_index + 1 < line_count {
        let Some(mut insert_at) = buffer.iter_at_line(line_index + 1) else {
            buffer.end_user_action();
            return;
        };
        let start_offset = insert_at.offset();
        buffer.insert(&mut insert_at, &format!("{line_text}\n"));
        start_offset
    } else {
        let mut insert_at = buffer.end_iter();
        let end_offset = insert_at.offset();
        buffer.insert(&mut insert_at, &format!("\n{line_text}"));
        end_offset + 1
    };

    let mut new_cursor = buffer.iter_at_offset(duplicate_start + column.min(line_len));
    buffer.place_cursor(&mut new_cursor);
    buffer.end_user_action();
}

/// Добавляет два пробела в начало каждой выделенной строки или в позицию курсора.
pub(super) fn indent_selection_or_cursor(buffer: &gtk4::TextBuffer, indent_width: i32) {
    let indent_width = indent_width.clamp(1, 8);
    let spaces = " ".repeat(indent_width as usize);
    if let Some((start_iter, end_iter)) = buffer.selection_bounds() {
        if start_iter != end_iter {
            let start_line = start_iter.line();
            let mut end_line = end_iter.line();
            let original_end_line = end_iter.line();
            if end_line > start_line && end_iter.starts_line() {
                end_line -= 1;
            }

            let start_col = start_iter.line_offset();
            let end_col = end_iter.line_offset();

            buffer.begin_user_action();

            for line_idx in start_line..=end_line {
                if let Some(mut line_iter) = buffer.iter_at_line(line_idx) {
                    buffer.insert(&mut line_iter, &spaces);
                }
            }

            let new_start_col = if start_col > 0 {
                start_col + indent_width
            } else {
                0
            };
            let new_end_col = if original_end_line == end_line && end_col > 0 {
                end_col + indent_width
            } else {
                end_col
            };

            if let Some(mut new_start) = buffer.iter_at_line(start_line) {
                let mut line_end = new_start.clone();
                line_end.forward_to_line_end();
                new_start.set_line_offset(new_start_col.min(line_end.line_offset()).max(0));

                if let Some(mut new_end) = buffer.iter_at_line(original_end_line) {
                    let mut end_line_end = new_end.clone();
                    end_line_end.forward_to_line_end();
                    new_end.set_line_offset(new_end_col.min(end_line_end.line_offset()).max(0));
                    buffer.select_range(&new_start, &new_end);
                }
            }

            buffer.end_user_action();
            return;
        }
    }

    buffer.begin_user_action();
    buffer.insert_at_cursor(&spaces);
    buffer.end_user_action();
}

/// Удаляет до двух начальных пробелов (или один Tab) с выделенных строк / строки курсора.
pub(super) fn unindent_selection_or_cursor(buffer: &gtk4::TextBuffer, indent_width: i32) {
    let indent_width = indent_width.clamp(1, 8);
    if let Some((start_iter, end_iter)) = buffer.selection_bounds() {
        if start_iter != end_iter {
            let start_line = start_iter.line();
            let mut end_line = end_iter.line();
            let original_end_line = end_iter.line();
            if end_line > start_line && end_iter.starts_line() {
                end_line -= 1;
            }

            let mut start_col = start_iter.line_offset();
            let mut end_col = end_iter.line_offset();

            buffer.begin_user_action();

            for line_idx in start_line..=end_line {
                if let Some(mut line_iter) = buffer.iter_at_line(line_idx) {
                    let chars_to_del = leading_indent_chars(&line_iter, indent_width);

                    if chars_to_del > 0 {
                        let mut del_end = line_iter.clone();
                        del_end.forward_chars(chars_to_del);
                        buffer.delete(&mut line_iter, &mut del_end);

                        if line_idx == start_line {
                            start_col = (start_col - chars_to_del).max(0);
                        }
                        if line_idx == original_end_line {
                            end_col = (end_col - chars_to_del).max(0);
                        }
                    }
                }
            }

            if let Some(mut new_start) = buffer.iter_at_line(start_line) {
                let mut line_end = new_start.clone();
                line_end.forward_to_line_end();
                new_start.set_line_offset(start_col.min(line_end.line_offset()).max(0));

                if let Some(mut new_end) = buffer.iter_at_line(original_end_line) {
                    let mut end_line_end = new_end.clone();
                    end_line_end.forward_to_line_end();
                    new_end.set_line_offset(end_col.min(end_line_end.line_offset()).max(0));
                    buffer.select_range(&new_start, &new_end);
                }
            }

            buffer.end_user_action();
            return;
        }
    }

    let insert_mark = buffer.get_insert();
    let cursor_iter = buffer.iter_at_mark(&insert_mark);
    let cur_line = cursor_iter.line();
    let cur_col = cursor_iter.line_offset();

    buffer.begin_user_action();

    let mut new_col = cur_col;
    if let Some(mut line_iter) = buffer.iter_at_line(cur_line) {
        let chars_to_del = leading_indent_chars(&line_iter, indent_width);

        if chars_to_del > 0 {
            let mut del_end = line_iter.clone();
            del_end.forward_chars(chars_to_del);
            buffer.delete(&mut line_iter, &mut del_end);
            new_col = (cur_col - chars_to_del).max(0);
        }
    }

    if let Some(mut new_iter) = buffer.iter_at_line(cur_line) {
        let mut line_end = new_iter.clone();
        line_end.forward_to_line_end();
        new_iter.set_line_offset(new_col.min(line_end.line_offset()).max(0));
        buffer.place_cursor(&new_iter);
    }

    buffer.end_user_action();
}


/// Сколько символов нужно удалить в начале строки для Shift+Tab.
///
/// Таб считается одним символом, пробелы — до `indent_width` подряд.
fn leading_indent_chars(start: &gtk4::TextIter, indent_width: i32) -> i32 {
    if start.char() == '\t' {
        return 1;
    }

    let mut iter = start.clone();
    let mut count = 0;
    for _ in 0..indent_width {
        if iter.char() != ' ' {
            break;
        }
        count += 1;
        if !iter.forward_char() {
            break;
        }
    }
    count
}

impl super::Document {
    /// Оборачивает выделение парой маркеров Markdown.
    ///
    /// Если выделения нет или открыт предпросмотр, ничего не делает.
    /// После вставки выделение остаётся на исходном тексте внутри
    /// маркеров, чтобы следующая команда форматирования применялась
    /// к тому же фрагменту.
    pub fn wrap_selection(&self, prefix: &str, suffix: &str) {
        if self.is_preview_active.get() {
            return;
        }

        let Some(text) = self.selected_text() else {
            return;
        };
        if text.is_empty() {
            return;
        }

        let replacement = format!("{prefix}{text}{suffix}");
        let select_start = prefix.chars().count();
        let select_len = text.chars().count();
        self.replace_selection(&replacement, select_start, select_len);
    }

    /// Меняет регистр выделения по кругу: строчные → заглавные →
    /// строчные. Для смешанного текста первое нажатие делает заглавные.
    pub fn cycle_selection_case(&self) {
        if self.is_preview_active.get() {
            return;
        }

        let Some(text) = self.selected_text() else {
            return;
        };
        if text.is_empty() {
            return;
        }

        let has_lowercase = text.chars().any(char::is_lowercase);
        let replacement = if has_lowercase {
            text.to_uppercase()
        } else {
            text.to_lowercase()
        };

        self.replace_selection(&replacement, 0, replacement.chars().count());
    }

    /// Возвращает текст текущего выделения или `None`.
    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.buffer.selection_bounds()?;
        Some(self.buffer.text(&start, &end, false).to_string())
    }

    /// Заменяет выделение и оставляет выделенным нужный фрагмент.
    ///
    /// Смещения считаются в символах: именно так работает
    /// `GtkTextIter::offset`, в отличие от байтовых индексов `&str`.
    fn replace_selection(&self, replacement: &str, select_start: usize, select_len: usize) {
        let Some((mut start, mut end)) = self.buffer.selection_bounds() else {
            return;
        };

        let start_offset = start.offset();

        self.buffer.begin_user_action();
        self.buffer.delete(&mut start, &mut end);

        let mut insert = self.buffer.iter_at_offset(start_offset);
        self.buffer.insert(&mut insert, replacement);

        let selection_start = self.buffer.iter_at_offset(start_offset + select_start as i32);
        let selection_end =
            self.buffer.iter_at_offset(start_offset + (select_start + select_len) as i32);
        self.buffer
            .select_range(&selection_start, &selection_end);

        self.buffer.end_user_action();
    }
}
