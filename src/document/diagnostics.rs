//! Проверка стиля Markdown — облегчённый аналог `markdownlint`.
//!
//! Модуль не меняет содержимое буфера: результат проверки — список
//! диагностических сообщений, которые накладываются на текст как
//! `GtkTextTag` с волнистым подчёркиванием. Поэтому линтер:
//! - не создаёт правок и не попадает в историю Undo;
//! - не выставляет флаг «есть несохранённые изменения»;
//! - может быть полностью выключен без изменения документа.
//!
//! Реализованы правила MD009, MD010, MD012, MD022, MD025, MD031, MD032,
//! MD040 и MD047. Это подмножество `markdownlint`, которое не требует
//! конфигурации и не даёт спорных предупреждений о вкусе разметки.
//!
//! # Почему колонки считаются в символах, а не в байтах
//!
//! `&str` в Rust индексируется байтами, а `GtkTextIter` измеряет позицию
//! количеством символов Unicode. На кириллице эти величины различаются:
//! «Привет» — 6 символов, но 12 байт. Если сохранить байтовый offset и
//! передать его в `forward_chars`, подчёркивание уедет вправо. Поэтому все
//! диапазоны в [`Diagnostic`] хранятся в символах строки, а байтовые
//! индексы преобразуются сразу при разборе строки.

use gtk4::prelude::*;
use gtk4::{gdk, glib, pango};
use std::rc::Rc;
use std::time::Duration;

use super::Document;
use crate::i18n::Language;

/// Задержка перед повторной проверкой после последней правки.
///
/// Полный проход по документу дешёвый, но на каждом нажатии клавиши он
/// всё равно не нужен: пока пользователь печатает, промежуточные
/// результаты никому не интересны. Небольшая пауза заодно избавляет от
/// мигания подчёркиваний.
const LINT_DELAY: Duration = Duration::from_millis(350);

/// Имена тегов для подчёркиваний. Теги регистрируются один раз на
/// `GtkTextTagTable` каждого документа.
const WARNING_TAG: &str = "lint-warning";
const ERROR_TAG: &str = "lint-error";

/// Важность диагностики.
///
/// Стилевые правила — предупреждения; ошибкой помечается только нарушение
/// структуры документа с несколькими заголовками первого уровня (MD025).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Severity {
    Warning,
    Error,
}

/// Идентификатор поддерживаемого правила стиля.
///
/// Номер совпадает с названием правила в `markdownlint`, поэтому
/// пользователь может найти подробное описание в документации линтера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rule {
    /// MD009 — пробелы в конце строки.
    Md009,
    /// MD010 — символ табуляции.
    Md010,
    /// MD012 — несколько пустых строк подряд.
    Md012,
    /// MD022 — заголовок не окружён пустыми строками.
    Md022,
    /// MD025 — больше одного заголовка первого уровня.
    Md025,
    /// MD031 — блок кода не окружён пустыми строками.
    Md031,
    /// MD032 — список не окружён пустыми строками.
    Md032,
    /// MD040 — у блока кода не указан язык.
    Md040,
    /// MD047 — файл не заканчивается одним переводом строки.
    Md047,
}

impl Rule {
    /// Возвращает идентификатор правила в виде `MDxxx`.
    pub(super) fn id(self) -> &'static str {
        match self {
            Rule::Md009 => "MD009",
            Rule::Md010 => "MD010",
            Rule::Md012 => "MD012",
            Rule::Md022 => "MD022",
            Rule::Md025 => "MD025",
            Rule::Md031 => "MD031",
            Rule::Md032 => "MD032",
            Rule::Md040 => "MD040",
            Rule::Md047 => "MD047",
        }
    }

    /// Важность нарушения.
    pub(super) fn severity(self) -> Severity {
        match self {
            // Несколько H1 ломают структуру документа и оглавление,
            // поэтому это единственная «ошибка» среди стилевых правил.
            Rule::Md025 => Severity::Error,
            _ => Severity::Warning,
        }
    }

    /// Имя `GtkTextTag`, которым рисуется подчёркивание.
    fn tag_name(self) -> &'static str {
        match self.severity() {
            Severity::Warning => WARNING_TAG,
            Severity::Error => ERROR_TAG,
        }
    }
}

/// Одна найденная проблема.
///
/// Позиция задана в координатах `GtkTextBuffer`: номер строки и смещение
/// в символах от начала строки. Такой формат позволяет применять тег без
/// пересчёта глобальных offset'ов после правок.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Diagnostic {
    /// Нарушенное правило.
    pub rule: Rule,
    /// Важность (влияет на цвет подчёркивания и маркера в gutter).
    pub severity: Severity,
    /// Номер строки, начиная с нуля.
    pub line: i32,
    /// Начало диапазона в символах от начала строки.
    pub start_col: i32,
    /// Конец диапазона (не включая) в символах от начала строки.
    pub end_col: i32,
}

impl Diagnostic {
    fn new(rule: Rule, line: usize, start_col: i32, end_col: i32) -> Self {
        Self {
            rule,
            severity: rule.severity(),
            line: line as i32,
            start_col,
            end_col,
        }
    }

    /// Проверяет, попадает ли позиция курсора в диапазон диагностики.
    ///
    /// Для пустых строк диапазон имеет нулевую ширину: подчёркивать в нём
    /// нечего, но подсказка при наведении всё равно должна работать.
    fn contains(&self, line: i32, column: i32) -> bool {
        self.line == line
            && column >= self.start_col
            && column < self.end_col.max(self.start_col + 1)
    }
}

/// Регистрирует теги диагностики в таблице тегов документа.
///
/// Pango не различает «жёлтую» и «красную» волну: единственный волнистый
/// вариант — `Underline::Error`. Цвет задаётся через `underline-rgba`,
/// поэтому важность передаётся цветом, а не формой линии.
pub(super) fn register_tags(tag_table: &gtk4::TextTagTable) {
    let colors = [
        (WARNING_TAG, gdk::RGBA::new(0.82, 0.55, 0.05, 1.0)),
        (ERROR_TAG, gdk::RGBA::new(0.90, 0.20, 0.20, 1.0)),
    ];

    for (name, color) in colors {
        let tag = gtk4::TextTag::builder()
            .name(name)
            .underline(pango::Underline::Error)
            .underline_rgba(&color)
            .build();
        tag_table.add(&tag);
    }
}

/// Накладывает подчёркивания на буфер.
///
/// Сначала снимаются только теги линтера — теги подсветки синтаксиса
/// (`md-*`) не затрагиваются. Затем каждый диапазон переводится в
/// `TextIter` по номеру строки и смещению в символах.
pub(super) fn apply_to_buffer(buffer: &gtk4::TextBuffer, diagnostics: &[Diagnostic]) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();

    for tag in [WARNING_TAG, ERROR_TAG] {
        buffer.remove_tag_by_name(tag, &start, &end);
    }

    for diagnostic in diagnostics {
        let Some(line_start) = buffer.iter_at_line(diagnostic.line) else {
            continue;
        };

        // `chars_in_line` включает завершающий перевод строки, поэтому
        // clamp защищает от попытки выйти за пределы строки.
        let line_chars = line_start.chars_in_line();
        let start_col = diagnostic.start_col.clamp(0, line_chars);
        let end_col = diagnostic.end_col.clamp(start_col, line_chars);

        // Нулевой диапазон (например, лишняя пустая строка) нарисовать
        // подчёркиванием нельзя — его показывает маркер в gutter.
        if start_col == end_col {
            continue;
        }

        let mut range_start = line_start;
        range_start.forward_chars(start_col);
        let mut range_end = line_start;
        range_end.forward_chars(end_col);

        buffer.apply_tag_by_name(diagnostic.rule.tag_name(), &range_start, &range_end);
    }
}

impl Document {
    /// Подключает подсказки при наведении на подчёркнутый фрагмент.
    ///
    /// Используется штатный механизм `query-tooltip` GTK: он сам решает,
    /// когда показать подсказку, и не мешает встроенным тултипам виджетов.
    /// Координаты события переводятся из окна виджета в координаты буфера.
    pub(super) fn setup_diagnostic_tooltip(self: &Rc<Self>) {
        self.text_view.set_has_tooltip(true);

        let weak_doc = Rc::downgrade(self);
        self.text_view
            .connect_query_tooltip(move |view, x, y, _keyboard_mode, tooltip| {
                let Some(doc) = weak_doc.upgrade() else {
                    return false;
                };

                let (buffer_x, buffer_y) =
                    view.window_to_buffer_coords(gtk4::TextWindowType::Widget, x, y);
                let Some(iter) = view.iter_at_location(buffer_x, buffer_y) else {
                    return false;
                };

                let line = iter.line();
                let column = iter.line_offset();

                let message = {
                    let diagnostics = doc.diagnostics.borrow();
                    diagnostics
                        .iter()
                        .find(|diagnostic| diagnostic.contains(line, column))
                        .map(|diagnostic| {
                            let lang = Language::from_code(&doc.settings.borrow().language);
                            format!(
                                "{} — {}",
                                diagnostic.rule.id(),
                                lang.lint_message(diagnostic.rule.id())
                            )
                        })
                };

                let Some(message) = message else {
                    return false;
                };

                tooltip.set_text(Some(message.as_str()));
                true
            });
    }

    /// Планирует проверку после последней правки.
    ///
    /// Предыдущий таймер снимается: важно проверять только актуальное
    /// состояние буфера. Слабая ссылка не продлевает жизнь документа,
    /// если вкладку закрыли до срабатывания таймера.
    pub(super) fn schedule_diagnostics(self: &Rc<Self>) {
        self.cancel_diagnostics();

        if !self.settings.borrow().markdown_lint {
            return;
        }

        let weak_doc = Rc::downgrade(self);
        let source_id = glib::timeout_add_local(LINT_DELAY, move || {
            if let Some(doc) = weak_doc.upgrade() {
                doc.diagnostics_timeout.set(None);
                doc.refresh_diagnostics();
            }
            glib::ControlFlow::Break
        });

        self.diagnostics_timeout.set(Some(source_id));
    }

    /// Снимает запланированную проверку, если она ещё не сработала.
    pub(super) fn cancel_diagnostics(&self) {
        if let Some(source_id) = self.diagnostics_timeout.take() {
            source_id.remove();
        }
    }

    /// Выполняет проверку немедленно и обновляет подчёркивания.
    ///
    /// Вызывается после загрузки файла, восстановления черновика и смены
    /// настройки. При обычном наборе текста используется дебаунс из
    /// [`Document::schedule_diagnostics`].
    pub(super) fn refresh_diagnostics(&self) {
        self.cancel_diagnostics();

        if !self.settings.borrow().markdown_lint {
            self.clear_diagnostics();
            return;
        }

        let text = self
            .buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), false);

        *self.diagnostics.borrow_mut() = validate(&text);

        // Блокируем ячейку только на время наложения тегов: draw-функция
        // gutter не должна увидеть частично обновлённый список.
        let diagnostics = self.diagnostics.borrow();
        apply_to_buffer(&self.buffer, &diagnostics);
        drop(diagnostics);

        self.gutter.queue_draw();
    }

    /// Убирает все подчёркивания и очищает список диагностик.
    pub(super) fn clear_diagnostics(&self) {
        self.diagnostics.borrow_mut().clear();

        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        for tag in [WARNING_TAG, ERROR_TAG] {
            self.buffer.remove_tag_by_name(tag, &start, &end);
        }

        self.gutter.queue_draw();
    }

    /// Применяет текущее значение настройки `markdown_lint`.
    ///
    /// Вызывается из окна настроек для всех открытых документов. При
    /// выключении проверки теги снимаются, но текст остаётся неизменным.
    pub fn apply_markdown_lint_setting(&self) {
        self.refresh_diagnostics();
    }
}

/// Данные открывающего fence-блока, нужные для MD031 и MD040.
struct FenceOpen {
    marker: char,
    length: usize,
    info_empty: bool,
}

/// Проверяет весь текст и возвращает найденные проблемы.
///
/// Функция намеренно не зависит от GTK: это позволяет тестировать правила
/// обычными unit-тестами, без создания окна и графического окружения.
pub(super) fn validate(text: &str) -> Vec<Diagnostic> {
    let mut result = Vec::new();

    // `split('\n')` сохраняет завершающую пустую строку, если текст
    // заканчивается переводом строки. Это совпадает с нумерацией строк
    // в GtkTextBuffer: для "a\n" существует и строка 0, и пустая строка 1.
    // `\r` отрезается, чтобы правила одинаково работали с CRLF-файлами.
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    let line_count = lines.len();

    // Первый проход: определяем, какие строки принадлежат fenced-блокам.
    // Внутри кода нельзя применять правила разметки — там `#` и `-`
    // являются обычными символами, а табы и пустые строки допустимы.
    let mut in_fence: Option<(char, usize)> = None;
    let mut is_code = vec![false; line_count];
    let mut opening: Vec<Option<FenceOpen>> = Vec::with_capacity(line_count);
    let mut closing = vec![false; line_count];
    opening.resize_with(line_count, || None);

    for (index, line) in lines.iter().enumerate() {
        if let Some((marker, min_length)) = in_fence {
            is_code[index] = true;
            if is_closing_fence(line, marker, min_length) {
                closing[index] = true;
                in_fence = None;
            }
            continue;
        }

        if let Some(fence) = parse_fence_open(line) {
            is_code[index] = true;
            in_fence = Some((fence.marker, fence.length));
            opening[index] = Some(fence);
        }
    }

    // --- MD009: пробелы в конце строки ---
    // Ровно два пробела в конце непустой строки — это разрешённый
    // Markdown-способ сделать жёсткий перенос, поэтому он не нарушение.
    for (index, line) in lines.iter().enumerate() {
        let without_spaces = line.trim_end_matches(' ');
        let trailing = line.len() - without_spaces.len();

        if trailing == 0 {
            continue;
        }

        let is_hard_break = trailing == 2 && !without_spaces.is_empty();
        if !is_hard_break {
            result.push(Diagnostic::new(
                Rule::Md009,
                index,
                char_len(without_spaces),
                char_len(line),
            ));
        }
    }

    // --- MD010: жёсткая табуляция ---
    // В блоках кода табы — нормальный способ выравнивания, поэтому
    // проверяются только строки обычного текста.
    for (index, line) in lines.iter().enumerate() {
        if is_code[index] {
            continue;
        }

        if let Some(byte_index) = line.find('\t') {
            let start = char_index(line, byte_index);
            result.push(Diagnostic::new(Rule::Md010, index, start, start + 1));
        }
    }

    // --- MD012: несколько пустых строк подряд ---
    // Пустые строки внутри fenced-блоков не считаются: это содержимое кода.
    let mut blank_run_start: Option<usize> = None;
    for index in 0..line_count {
        let is_blank = !is_code[index] && lines[index].trim().is_empty();

        if is_blank {
            if blank_run_start.is_none() {
                blank_run_start = Some(index);
            }
        } else if let Some(start) = blank_run_start.take() {
            push_blank_run(&mut result, start, index);
        }
    }
    if let Some(start) = blank_run_start.take() {
        push_blank_run(&mut result, start, line_count);
    }

    // --- MD022 и MD025: заголовки ATX ---
    let mut first_level_seen = false;
    for (index, line) in lines.iter().enumerate() {
        if is_code[index] {
            continue;
        }

        let Some(level) = parse_atx_heading(line) else {
            continue;
        };

        let missing_before = index > 0 && !lines[index - 1].trim().is_empty();
        let missing_after = index + 1 < line_count && !lines[index + 1].trim().is_empty();

        if missing_before || missing_after {
            result.push(Diagnostic::new(
                Rule::Md022,
                index,
                0,
                char_len(line).max(1),
            ));
        }

        if level == 1 {
            if first_level_seen {
                result.push(Diagnostic::new(
                    Rule::Md025,
                    index,
                    0,
                    char_len(line).max(1),
                ));
            } else {
                first_level_seen = true;
            }
        }
    }

    // --- MD031 и MD040: fenced-блоки ---
    for index in 0..line_count {
        if let Some(fence) = opening[index].as_ref() {
            if index > 0 && !lines[index - 1].trim().is_empty() {
                result.push(Diagnostic::new(
                    Rule::Md031,
                    index,
                    0,
                    char_len(lines[index]).max(1),
                ));
            }

            if fence.info_empty {
                result.push(Diagnostic::new(
                    Rule::Md040,
                    index,
                    0,
                    char_len(lines[index]).max(1),
                ));
            }
        }

        if closing[index] && index + 1 < line_count && !lines[index + 1].trim().is_empty() {
            result.push(Diagnostic::new(
                Rule::Md031,
                index,
                0,
                char_len(lines[index]).max(1),
            ));
        }
    }

    // --- MD032: списки ---
    // Обходим блоки списков, а не отдельные строки: после обработки блока
    // курсор сразу переносится за его конец. Иначе длинный список из
    // тысяч пунктов проверялся бы квадратично.
    let mut index = 0;
    while index < line_count {
        if is_code[index] {
            index += 1;
            continue;
        }

        let Some(indent) = list_item_indent(lines[index]) else {
            index += 1;
            continue;
        };

        // Список может быть вложенным; отдельным блоком считаем только
        // элементы верхнего уровня (отступ не больше трёх пробелов).
        if indent > 3 {
            index += 1;
            continue;
        }

        let previous_is_item = index > 0 && list_item_indent(lines[index - 1]).is_some();
        let previous_is_blank = index == 0 || lines[index - 1].trim().is_empty();

        // Строка перед первым элементом должна быть пустой. Исключение —
        // продолжение уже начатого списка (предыдущая строка тоже элемент)
        // и абзац с отступом, относящийся к тому же элементу списка.
        if index > 0
            && !previous_is_blank
            && !previous_is_item
            && indent_chars(lines[index - 1]) < indent + 2
        {
            result.push(Diagnostic::new(
                Rule::Md032,
                index,
                0,
                char_len(lines[index]).max(1),
            ));
        }

        // Ищем конец блока списка. Пустые строки внутри списка не
        // завершают блок, если дальше снова идёт элемент или продолжение
        // с отступом. Первая непустая строка без отступа — уже другой блок.
        let mut cursor = index + 1;
        while cursor < line_count {
            if is_code[cursor] {
                // Fenced-блок внутри элемента списка принадлежит списку,
                // если сам fence сдвинут на отступ элемента. Строки внутри
                // блока могут иметь любой отступ — это содержимое кода.
                if opening[cursor].is_some() {
                    if indent_chars(lines[cursor]) >= indent + 2 {
                        cursor += 1;
                        continue;
                    }
                    break;
                }
                cursor += 1;
                continue;
            }

            if lines[cursor].trim().is_empty() {
                let mut lookahead = cursor + 1;
                while lookahead < line_count && lines[lookahead].trim().is_empty() {
                    lookahead += 1;
                }

                let continues = lookahead < line_count
                    && (list_item_indent(lines[lookahead]).is_some()
                        || indent_chars(lines[lookahead]) >= indent + 2);

                if continues {
                    cursor += 1;
                    continue;
                }
                break;
            }

            if list_item_indent(lines[cursor]).is_some()
                || indent_chars(lines[cursor]) >= indent + 2
            {
                cursor += 1;
                continue;
            }

            break;
        }

        // Если блок закончился непустой строкой, после списка нет пустой
        // строки — сообщаем об этом на последней строке самого списка.
        if cursor < line_count && !lines[cursor].trim().is_empty() {
            let last = cursor.saturating_sub(1).max(index);
            result.push(Diagnostic::new(
                Rule::Md032,
                last,
                0,
                char_len(lines[last]).max(1),
            ));
        }

        // Вложенные элементы и продолжения уже проверены внутри блока.
        index = cursor.max(index + 1);
    }

    // --- MD047: ровно один перевод строки в конце файла ---
    if !text.is_empty() {
        let has_single_final_newline = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .map(|rest| !rest.ends_with('\n') && !rest.ends_with('\r'))
            .unwrap_or(false);

        if !has_single_final_newline {
            // Подчёркиваем последний видимый символ: пустую строку
            // подчеркнуть нельзя, а маркер в gutter укажет на неё.
            let last_line = lines
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .unwrap_or(0);
            let length = char_len(lines[last_line]);
            let (start_col, end_col) = if length > 0 {
                (length - 1, length)
            } else {
                (0, 0)
            };

            result.push(Diagnostic::new(Rule::Md047, last_line, start_col, end_col));
        }
    }

    // Разные правила могут указать на одну и ту же строку (например,
    // MD022 и MD032). Дубликаты не нужны ни в подсказках, ни в gutter.
    result.sort_by(|a, b| {
        (a.line, a.start_col, a.end_col, a.rule.id()).cmp(&(
            b.line,
            b.start_col,
            b.end_col,
            b.rule.id(),
        ))
    });
    result.dedup();
    result
}

/// Добавляет диагностики MD012 для лишних строк в серии пустых строк.
fn push_blank_run(result: &mut Vec<Diagnostic>, start: usize, end: usize) {
    if end.saturating_sub(start) <= 1 {
        return;
    }

    // Первая пустая строка допустима как разделитель, начиная со второй —
    // нарушение. Диапазон нулевой ширины: в пустой строке нет символов,
    // поэтому её отмечает только gutter.
    for line in start + 1..end {
        result.push(Diagnostic::new(Rule::Md012, line, 0, 0));
    }
}

/// Длина строки в символах Unicode (именно так считает позиции GTK).
fn char_len(text: &str) -> i32 {
    text.chars().count() as i32
}

/// Переводит байтовый индекс в смещение в символах.
fn char_index(text: &str, byte_index: usize) -> i32 {
    text[..byte_index].chars().count() as i32
}

/// Ширина начального отступа в символах.
fn indent_chars(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

/// Разбирает открывающий fence-блок.
///
/// Markdown допускает не больше трёх пробелов отступа, минимум три
/// символа `` ` `` или `~` и произвольную info-строку после них.
fn parse_fence_open(line: &str) -> Option<FenceOpen> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent > 3 {
        return None;
    }

    let rest = &line[indent..];
    let marker = rest.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }

    let length = rest.chars().take_while(|ch| *ch == marker).count();
    if length < 3 {
        return None;
    }

    let info = &rest[length..];

    // По правилам CommonMark info-строка backtick-блока не может
    // содержать обратных кавычек. Такой строкой закрывают inline-код,
    // а не открывают блок.
    if marker == '`' && info.contains('`') {
        return None;
    }

    Some(FenceOpen {
        marker,
        length,
        info_empty: info.trim().is_empty(),
    })
}

/// Проверяет, закрывает ли строка текущий fence-блок.
fn is_closing_fence(line: &str, marker: char, min_length: usize) -> bool {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent > 3 {
        return false;
    }

    let rest = &line[indent..];
    let length = rest.chars().take_while(|ch| *ch == marker).count();

    length >= min_length && rest[length..].trim().is_empty()
}

/// Возвращает уровень заголовка ATX (`#` … `######`) или `None`.
///
/// Заголовком считается только строка, где после решёток стоит пробел,
/// таб или конец строки. Благодаря этому `#hashtag` не принимается за
/// заголовок и не порождает ложных предупреждений.
fn parse_atx_heading(line: &str) -> Option<u8> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent > 3 {
        return None;
    }

    let rest = &line[indent..];
    let level = rest.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }

    let after = &rest[level..];
    if after.is_empty() || after.starts_with(' ') || after.starts_with('\t') {
        Some(level as u8)
    } else {
        None
    }
}

/// Возвращает отступ элемента списка в символах или `None`.
///
/// Поддерживаются маркеры `-`, `*`, `+` и упорядоченные `1.` / `1)`.
/// После маркера обязателен пробел или таб — иначе это не список.
fn list_item_indent(line: &str) -> Option<usize> {
    let indent = indent_chars(line);
    let prefix_bytes: usize = line.chars().take(indent).map(char::len_utf8).sum();
    let rest = &line[prefix_bytes..];

    for bullet in ['-', '*', '+'] {
        let mut chars = rest.chars();
        if chars.next() == Some(bullet) && matches!(chars.next(), Some(' ') | Some('\t')) {
            return Some(indent);
        }
    }

    let bytes = rest.as_bytes();
    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();

    if (1..=9).contains(&digits) && digits < bytes.len() {
        let marker = bytes[digits];
        let after = bytes.get(digits + 1).copied();
        if (marker == b'.' || marker == b')') && matches!(after, Some(b' ') | Some(b'\t')) {
            return Some(indent);
        }
    }

    None
}
