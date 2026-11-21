//! Поиск и замена в редакторе и HTML-предпросмотре.

use adw::prelude::*;
use webkit6::prelude::*;
use gtk4::gio;
use gtk4::glib;
use std::rc::Rc;

use crate::highlighter;
use crate::i18n::Language;

use super::{Document, MAX_SEARCH_HIGHLIGHTS};

impl Document {
    /// Подключает поиск по буферу документа.
    ///
    /// Все совпадения подсвечиваются тегом `search-match`, текущее — более
    /// контрастным `search-current`. Состояние хранится в offset'ах буфера,
    /// поэтому после редактирования список пересчитывается, а не ссылается
    /// на устаревшие `TextIter`.
    pub(super) fn setup_search(self: &Rc<Self>) {
        let tag_table = self.tag_table.clone();

        let highlight_tag = gtk4::TextTag::builder()
            .name("search-match")
            .background("#f5a97f")
            .foreground("#181926")
            .build();
        tag_table.add(&highlight_tag);

        let current_tag = gtk4::TextTag::builder()
            .name("search-current")
            .background("#f97316")
            .foreground("#fff7ed")
            .weight(700)
            .build();
        tag_table.add(&current_tag);
        current_tag.set_priority(highlight_tag.priority() + 1);

        // Ввод запроса: пересчитываем совпадения и начинаем с первого.
        let weak_doc = Rc::downgrade(self);
        self.search_entry.connect_search_changed(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.refresh_search_matches(true);
            }
        });

        // Enter — следующее совпадение, Shift+Enter — предыдущее.
        let weak_doc = Rc::downgrade(self);
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key_controller.connect_key_pressed(move |_, keyval, _, state| {
            if keyval == gtk4::gdk::Key::Return || keyval == gtk4::gdk::Key::KP_Enter {
                if let Some(doc) = weak_doc.upgrade() {
                    let forward = !state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
                    doc.navigate_search(forward);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.search_entry.add_controller(key_controller);

        // Кнопки навигации по совпадениям.
        let weak_doc = Rc::downgrade(self);
        self.search_prev_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.navigate_search(false);
            }
        });
        let weak_doc = Rc::downgrade(self);
        self.search_next_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.navigate_search(true);
            }
        });

        // Показ/скрытие строки замены.
        let replace_row = self.replace_row.clone();
        let replace_entry = self.replace_entry.clone();
        let search_entry = self.search_entry.clone();
        let replace_btn = self.replace_btn.clone();
        let replace_all_btn = self.replace_all_btn.clone();
        self.replace_toggle.connect_toggled(move |button| {
            let visible = button.is_active();
            replace_row.set_visible(visible);
            replace_entry.set_visible(visible);
            replace_btn.set_visible(visible);
            replace_all_btn.set_visible(visible);
            if visible {
                if search_entry.text().is_empty() {
                    search_entry.grab_focus();
                } else {
                    replace_entry.grab_focus();
                }
            }
        });

        let weak_doc = Rc::downgrade(self);
        self.replace_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.replace_current();
            }
        });
        let weak_doc = Rc::downgrade(self);
        self.replace_all_btn.connect_clicked(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.replace_all();
            }
        });
        let weak_doc = Rc::downgrade(self);
        self.replace_entry.connect_activate(move |_| {
            if let Some(doc) = weak_doc.upgrade() {
                doc.replace_current();
            }
        });

        // Esc в поле замены должен закрывать всю панель, как и в поле поиска.
        let weak_doc = Rc::downgrade(self);
        let replace_key_controller = gtk4::EventControllerKey::new();
        replace_key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        replace_key_controller.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                if let Some(doc) = weak_doc.upgrade() {
                    doc.search_bar.set_search_mode(false);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.replace_entry.add_controller(replace_key_controller);

        // Закрытие панели убирает подсветку и прячет строку замены;
        // повторное открытие с тем же запросом восстанавливает поиск.
        let weak_doc = Rc::downgrade(self);
        self.search_bar
            .connect_search_mode_enabled_notify(move |bar| {
                if let Some(doc) = weak_doc.upgrade() {
                    if bar.is_search_mode() {
                        doc.refresh_search_matches(true);
                    } else {
                        doc.clear_search_highlights();
                        doc.replace_toggle.set_active(false);
                        doc.replace_row.set_visible(false);
                    }
                }
            });
    }

    /// Открывает панель поиска сразу в режиме замены (Ctrl+R).
    ///
    /// Замена работает по тексту редактора, поэтому из предпросмотра
    /// сначала переключаемся в редактор.
    pub fn open_replace_mode(self: &Rc<Self>) {
        if self.is_preview_active.get() {
            self.switch_to_editor();
        }

        self.search_bar.set_search_mode(true);
        self.replace_toggle.set_active(true);
        self.replace_row.set_visible(true);

        // Если запрос еще не введен, сначала нужен поиск, а не поле замены.
        if self.search_entry.text().is_empty() {
            self.search_entry.grab_focus();
        } else {
            self.replace_entry.grab_focus();
        }
    }

    /// Заменяет текущее совпадение и переходит к следующему.
    fn replace_current(self: &Rc<Self>) {
        if self.is_preview_active.get() {
            return;
        }
        let Some((match_start, match_end)) = self
            .search_matches
            .borrow()
            .get(self.search_index.get())
            .copied()
        else {
            return;
        };

        let replacement = self.replace_entry.text().to_string();
        let replacement_len = replacement.chars().count() as i32;

        self.suppress_search_refresh.set(true);
        self.bulk_editing.set(true);
        self.buffer.begin_user_action();

        let mut start_iter = self.buffer.iter_at_offset(match_start);
        let mut end_iter = self.buffer.iter_at_offset(match_end);
        self.buffer.delete(&mut start_iter, &mut end_iter);

        if !replacement.is_empty() {
            let mut insert_iter = self.buffer.iter_at_offset(match_start);
            self.buffer.insert(&mut insert_iter, &replacement);
        }

        self.buffer.end_user_action();
        self.suppress_search_refresh.set(false);
        self.end_bulk_edit();

        self.refresh_search_matches(false);

        let after = match_start + replacement_len;
        let matches = self.search_matches.borrow();
        if !matches.is_empty() {
            let next_index = next_match_index_after(&matches, after);
            drop(matches);
            self.search_index.set(next_index);
            self.update_search_status();
            self.apply_current_search_match();
        }
    }

    /// Заменяет все совпадения одним действием Undo.
    fn replace_all(self: &Rc<Self>) {
        if self.is_preview_active.get() {
            return;
        }
        let query = self.search_entry.text().to_string();
        if query.is_empty() {
            return;
        }
        let replacement = self.replace_entry.text().to_string();

        self.suppress_search_refresh.set(true);
        self.bulk_editing.set(true);
        let count = replace_all_matches(&self.buffer, &query, &replacement);
        self.suppress_search_refresh.set(false);
        self.end_bulk_edit();

        if count > 0 {
            self.refresh_search_matches(true);
        }
    }

    /// Завершает серию правок и один раз обновляет тяжёлые представления.
    ///
    /// Принимает `&Rc<Self>`, потому что для планирования отложенной
    /// проверки стиля нужен разделяемый владелец документа.
    fn end_bulk_edit(self: &Rc<Self>) {
        self.bulk_editing.set(false);
        highlighter::highlight_markdown_buffer(&self.buffer);
        self.schedule_dirty_check();
        self.schedule_diagnostics();
        self.gutter.queue_draw();
        self.update_tab_title();
    }

    /// Пересчитывает совпадения по текущему запросу.
    ///
    /// `reset_current` используется при смене запроса: начинаем с первого
    /// совпадения. При редактировании текста текущий индекс сохраняется,
    /// насколько это возможно.
    pub(super) fn refresh_search_matches(self: &Rc<Self>, reset_current: bool) {
        // Пока панель поиска закрыта, редактирование текста не должно
        // возвращать подсветку старого запроса.
        if !self.search_bar.is_search_mode() {
            self.clear_search_highlights();
            return;
        }

        if self.is_preview_active.get() {
            self.refresh_preview_search(reset_current);
            return;
        }

        let query = self.search_entry.text().to_string();
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();

        self.buffer.remove_tag_by_name("search-match", &start, &end);
        self.buffer.remove_tag_by_name("search-current", &start, &end);

        if query.is_empty() {
            self.search_matches.borrow_mut().clear();
            self.search_index.set(0);
            self.update_search_status();
            return;
        }

        let matches = find_matches(&self.buffer, &query);
        let total = matches.len();

        for &(match_start, match_end) in matches.iter().take(MAX_SEARCH_HIGHLIGHTS) {
            let start_iter = self.buffer.iter_at_offset(match_start);
            let end_iter = self.buffer.iter_at_offset(match_end);
            self.buffer
                .apply_tag_by_name("search-match", &start_iter, &end_iter);
        }

        let index = if reset_current || self.search_index.get() >= total {
            0
        } else {
            self.search_index.get()
        };

        *self.search_matches.borrow_mut() = matches;
        self.search_index.set(index);
        self.update_search_status();
        self.apply_current_search_match();
    }

    /// Переходит к следующему или предыдущему совпадению с заворотом.
    fn navigate_search(self: &Rc<Self>, forward: bool) {
        if self.is_preview_active.get() {
            self.navigate_preview_search(forward);
            return;
        }

        let total = self.search_matches.borrow().len();
        let Some(index) = wrapped_search_index(self.search_index.get(), total, forward) else {
            return;
        };

        self.search_index.set(index);
        self.update_search_status();
        self.apply_current_search_match();
    }

    /// Подсвечивает текущее совпадение и прокручивает к нему редактор.
    fn apply_current_search_match(&self) {
        let Some((match_start, match_end)) = self
            .search_matches
            .borrow()
            .get(self.search_index.get())
            .copied()
        else {
            return;
        };

        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer
            .remove_tag_by_name("search-current", &start, &end);

        let mut match_start_iter = self.buffer.iter_at_offset(match_start);
        let match_end_iter = self.buffer.iter_at_offset(match_end);
        self.buffer.apply_tag_by_name(
            "search-current",
            &match_start_iter,
            &match_end_iter,
        );

        self.buffer.place_cursor(&match_start_iter);
        self.scroll_editor_to_iter(&mut match_start_iter, 0.5);
    }

    /// Прокручивает редактор к итератору по вертикали, не сдвигая
    /// горизонтальную позицию.
    ///
    /// `GtkTextView::scroll_to_iter` с `xalign = 0.0` даже при включенном
    /// переносе строк пытается выровнять позицию по левому краю и может
    /// сдвинуть горизонтальный adjustment. Тогда справа появляется пустая
    /// полоса. После прокрутки возвращаем горизонтальную позицию в ноль.
    fn scroll_editor_to_iter(&self, iter: &mut gtk4::TextIter, yalign: f64) {
        self.text_view.scroll_to_iter(iter, 0.1, true, 0.0, yalign);
        self.reset_editor_horizontal_scroll();
    }

    /// Возвращает горизонтальную прокрутку редактора в начало.
    fn reset_editor_horizontal_scroll(&self) {
        // Без переноса строк горизонтальная прокрутка нужна, чтобы
        // добраться до конца длинной строки.
        if !self.settings.borrow().soft_wrap {
            return;
        }

        let adjustment = self.editor_scroll.hadjustment();
        adjustment.set_value(0.0);

        // Повторно в idle: GTK может пересчитать adjustment после текущей
        // итерации главного цикла.
        glib::idle_add_local_once(move || adjustment.set_value(0.0));
    }

    /// Убирает подсветку поиска при закрытии панели, сохраняя запрос.
    fn clear_search_highlights(&self) {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer.remove_tag_by_name("search-match", &start, &end);
        self.buffer.remove_tag_by_name("search-current", &start, &end);
        self.search_matches.borrow_mut().clear();
        self.search_index.set(0);
        self.clear_preview_search();
        self.update_search_status();
    }

    /// Снимает preview-подсветку и сбрасывает ее счетчики.
    pub(super) fn clear_preview_search(&self) {
        self.preview_search_generation
            .set(self.preview_search_generation.get() + 1);
        self.preview_search_count.set(0);
        self.preview_search_index.set(0);

        self.web_view.evaluate_javascript(
            "if (window.__asterSearch) { window.__asterSearch.clear(); }",
            None,
            None,
            Option::<&gio::Cancellable>::None,
            |_| {},
        );
    }

    /// Ищет видимый текст в HTML-предпросмотре через JavaScript.
    pub(super) fn refresh_preview_search(self: &Rc<Self>, _reset_current: bool) {
        let query = self.search_entry.text().to_string();
        let generation = self.preview_search_generation.get() + 1;
        self.preview_search_generation.set(generation);

        if query.is_empty() {
            self.clear_preview_search();
            self.update_search_status();
            return;
        }

        let query_json = serde_json::to_string(&query)
            .unwrap_or_else(|_| "\"\"".to_string());
        let script = format!(
            "window.__asterSearch ? window.__asterSearch.find({query_json}) : JSON.stringify({{count: 0, index: -1}});"
        );

        let weak_doc = Rc::downgrade(self);
        self.web_view.evaluate_javascript(
            &script,
            None,
            None,
            Option::<&gio::Cancellable>::None,
            move |result| {
                let Some(doc) = weak_doc.upgrade() else {
                    return;
                };
                if doc.preview_search_generation.get() != generation
                    || !doc.is_preview_active.get()
                {
                    return;
                }
                if let Ok(value) = result {
                    doc.apply_preview_search_result(&value.to_str());
                }
            },
        );
    }

    /// Переходит к следующему/предыдущему совпадению в предпросмотре.
    fn navigate_preview_search(self: &Rc<Self>, forward: bool) {
        let generation = self.preview_search_generation.get() + 1;
        self.preview_search_generation.set(generation);

        let script = format!(
            "window.__asterSearch ? window.__asterSearch.next({}) : JSON.stringify({{count: 0, index: -1}});",
            if forward { "true" } else { "false" }
        );

        let weak_doc = Rc::downgrade(self);
        self.web_view.evaluate_javascript(
            &script,
            None,
            None,
            Option::<&gio::Cancellable>::None,
            move |result| {
                let Some(doc) = weak_doc.upgrade() else {
                    return;
                };
                if doc.preview_search_generation.get() != generation
                    || !doc.is_preview_active.get()
                {
                    return;
                }
                if let Ok(value) = result {
                    doc.apply_preview_search_result(&value.to_str());
                }
            },
        );
    }

    /// Применяет JSON-ответ JavaScript-поиска к счетчику и статусу.
    fn apply_preview_search_result(&self, raw: &str) {
        let Some((count, index)) = parse_preview_search_result(raw) else {
            return;
        };

        self.preview_search_count.set(count);
        self.preview_search_index.set(index);
        self.update_search_status();
    }

    /// Обновляет счетчик и доступность кнопок навигации.
    pub(super) fn update_search_status(&self) {
        let lang = Language::from_code(&self.settings.borrow().language);
        let in_preview = self.is_preview_active.get();
        let (total, current) = if in_preview {
            (
                self.preview_search_count.get(),
                self.preview_search_index.get(),
            )
        } else {
            (self.search_matches.borrow().len(), self.search_index.get())
        };

        self.search_prev_btn.set_sensitive(total > 0);
        self.search_next_btn.set_sensitive(total > 0);
        self.replace_btn.set_sensitive(!in_preview && total > 0);
        self.replace_all_btn.set_sensitive(!in_preview && total > 0);

        if !self.search_bar.is_search_mode() || self.search_entry.text().is_empty() {
            self.search_status.set_text("");
        } else if total == 0 {
            self.search_status.set_text(lang.search_no_results());
        } else {
            self.search_status
                .set_text(&lang.search_counter(current + 1, total));
        }
    }
}

/// Разбирает JSON-ответ JavaScript-поиска.
///
/// Возвращает `(количество, индекс)`. При нулевом количестве индекс всегда 0.
pub(super) fn parse_preview_search_result(raw: &str) -> Option<(usize, usize)> {
    let result = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let count = result.get("count")?.as_u64()? as usize;
    let index = result
        .get("index")
        .and_then(|value| value.as_i64())
        .unwrap_or(-1);

    Some((count, if count == 0 { 0 } else { index.max(0) as usize }))
}

/// Индекс ближайшего совпадения, начинающегося не раньше `after`.
///
/// Если таких нет, возвращает 0: после замены поиск заворачивается на начало.
pub(super) fn next_match_index_after(matches: &[(i32, i32)], after: i32) -> usize {
    matches
        .iter()
        .position(|(start, _)| *start >= after)
        .unwrap_or(0)
}

/// Заменяет все вхождения `query` на `replacement` и возвращает их количество.
///
/// Правки идут с конца, чтобы offset'ы еще не обработанных совпадений
/// оставались корректными. Все изменения оборачиваются в одно user action,
/// поэтому `Ctrl+Z` отменяет замену целиком.
pub(super) fn replace_all_matches(buffer: &gtk4::TextBuffer, query: &str, replacement: &str) -> usize {
    let matches = find_matches(buffer, query);
    if matches.is_empty() {
        return 0;
    }

    let original = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    let mut result = String::with_capacity(original.len());
    let mut chars = original.char_indices();
    let mut char_index = 0usize;

    // Собираем новый текст в памяти: одна операция set_text вместо тысяч
    // delete/insert, каждое из которых поднимает changed и перестраивает
    // undo-стек GtkTextBuffer.
    for &(start, end) in &matches {
        let start = start.max(0) as usize;
        let end = end.max(0) as usize;

        while char_index < start {
            let Some((_, ch)) = chars.next() else {
                break;
            };
            result.push(ch);
            char_index += 1;
        }

        result.push_str(replacement);

        while char_index < end {
            if chars.next().is_none() {
                break;
            }
            char_index += 1;
        }
    }

    for (_, ch) in chars {
        result.push(ch);
    }

    // Одна группа Undo: удаляем старый текст и вставляем новый. Это
    // заметно быстрее тысяч отдельных delete/insert и, в отличие от
    // set_text, остается отменяемым действием.
    buffer.begin_user_action();
    let mut start = buffer.start_iter();
    let mut end = buffer.end_iter();
    buffer.delete(&mut start, &mut end);
    if !result.is_empty() {
        let mut insert_at = buffer.start_iter();
        buffer.insert(&mut insert_at, &result);
    }
    buffer.end_user_action();

    matches.len()
}

/// Находит все вхождения запроса и возвращает их диапазоны в offset'ах буфера.
pub(super) fn find_matches(buffer: &gtk4::TextBuffer, query: &str) -> Vec<(i32, i32)> {
    if query.is_empty() {
        return Vec::new();
    }

    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    let lower_text = text.to_lowercase();
    let lower_query = query.to_lowercase();

    // to_lowercase() может изменить длину строки (например, для некоторых
    // редких Unicode-символов). В таком случае byte offsets нельзя
    // сопоставить с исходным текстом, поэтому используем медленный,
    // но универсальный поиск GTK.
    if lower_query.is_empty() || lower_text.len() != text.len() {
        return find_matches_gtk(buffer, query);
    }

    // Сопоставление byte offset → character offset нужно, потому что
    // GtkTextBuffer и GtkTextIter работают с character offset.
    let mut boundaries = Vec::with_capacity(text.len().min(1_000_000));
    for (byte_index, _) in text.char_indices() {
        boundaries.push(byte_index as u32);
    }
    boundaries.push(text.len() as u32);

    lower_text
        .match_indices(&lower_query)
        .map(|(start, _matched)| {
            let end = start + lower_query.len();
            let start_char = boundaries
                .binary_search(&(start as u32))
                .unwrap_or_else(|index| index);
            let end_char = boundaries
                .binary_search(&(end as u32))
                .unwrap_or_else(|index| index);
            (start_char as i32, end_char as i32)
        })
        .collect()
}

/// Универсальный fallback поиска через GtkTextBuffer.
///
/// Используется, только если lowercase-преобразование меняет длину строки.
pub(super) fn find_matches_gtk(buffer: &gtk4::TextBuffer, query: &str) -> Vec<(i32, i32)> {
    let mut matches = Vec::new();
    let mut iter = buffer.start_iter();

    while let Some((match_start, match_end)) = iter.forward_search(
        query,
        gtk4::TextSearchFlags::CASE_INSENSITIVE,
        None,
    ) {
        matches.push((match_start.offset(), match_end.offset()));
        iter = match_end;
    }

    matches
}

/// Вычисляет индекс следующего/предыдущего совпадения с заворотом.
pub(super) fn wrapped_search_index(current: usize, total: usize, forward: bool) -> Option<usize> {
    if total == 0 {
        return None;
    }

    if forward {
        Some((current + 1) % total)
    } else {
        Some((current + total - 1) % total)
    }
}

