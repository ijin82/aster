//! Проверка стиля Markdown — облегчённый аналог `markdownlint`.
//!
//! Модуль не меняет содержимое буфера: результат проверки — список
//! диагностических сообщений, которые накладываются на текст как
//! `GtkTextTag` с волнистым подчёркиванием. Поэтому линтер:
//! - не создаёт правок и не попадает в историю Undo;
//! - не выставляет флаг «есть несохранённые изменения»;
//! - может быть полностью выключен без изменения документа.
//!
//! Реализованные правила: MD007, MD009, MD010, MD012, MD018–MD026,
//! MD029–MD032, MD040, MD042, MD045, MD047, MD055, MD056 и MD060. Это подмножество `markdownlint`, которое не
//! требует конфигурации и не даёт спорных предупреждений о вкусе разметки.
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
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use super::Document;
use crate::external::open_uri;
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

/// Знаки препинания, которые нельзя оставлять в конце заголовка.
///
/// Это значение по умолчанию из markdownlint. Вопросительный знак
/// намеренно разрешён: заголовки в стиле FAQ — распространённое явление.
/// Полноширинные варианты нужны для документов на китайском и японском.
const MD026_PUNCTUATION: &str = ".,;:!。，；：！";

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
    /// MD007 — неверный отступ вложенного неупорядоченного списка.
    Md007,
    /// MD009 — пробелы в конце строки.
    Md009,
    /// MD010 — символ табуляции.
    Md010,
    /// MD012 — несколько пустых строк подряд.
    Md012,
    /// MD018 — нет пробела после решёток ATX-заголовка.
    Md018,
    /// MD019 — несколько пробелов после решёток ATX-заголовка.
    Md019,
    /// MD020 — нет пробела внутри решёток закрытого ATX-заголовка.
    Md020,
    /// MD021 — несколько пробелов внутри решёток закрытого ATX-заголовка.
    Md021,
    /// MD022 — заголовок не окружён пустыми строками.
    Md022,
    /// MD023 — заголовок начинается с отступа.
    Md023,
    /// MD024 — повторяющийся текст заголовка.
    Md024,
    /// MD025 — больше одного заголовка первого уровня.
    Md025,
    /// MD026 — знак препинания в конце заголовка.
    Md026,
    /// MD029 — неверная нумерация упорядоченного списка.
    Md029,
    /// MD030 — неверное число пробелов после маркера списка.
    Md030,
    /// MD031 — блок кода не окружён пустыми строками.
    Md031,
    /// MD032 — список не окружён пустыми строками.
    Md032,
    /// MD040 — у блока кода не указан язык.
    Md040,
    /// MD042 — ссылка без адреса.
    Md042,
    /// MD045 — у изображения нет альтернативного текста.
    Md045,
    /// MD047 — файл не заканчивается одним переводом строки.
    Md047,
    /// MD055 — непостоянный стиль pipe-символов в таблицах.
    Md055,
    /// MD056 — неверное число колонок в строке таблицы.
    Md056,
    /// MD060 — непостоянный стиль колонок таблицы.
    Md060,
    /// GHALERT — неизвестный или некорректный тип GitHub Alert.
    GhAlert,
}

impl Rule {
    /// Возвращает идентификатор правила в виде `MDxxx`.
    pub(super) fn id(self) -> &'static str {
        match self {
            Rule::Md007 => "MD007",
            Rule::Md009 => "MD009",
            Rule::Md010 => "MD010",
            Rule::Md012 => "MD012",
            Rule::Md018 => "MD018",
            Rule::Md019 => "MD019",
            Rule::Md020 => "MD020",
            Rule::Md021 => "MD021",
            Rule::Md022 => "MD022",
            Rule::Md023 => "MD023",
            Rule::Md024 => "MD024",
            Rule::Md025 => "MD025",
            Rule::Md026 => "MD026",
            Rule::Md029 => "MD029",
            Rule::Md030 => "MD030",
            Rule::Md031 => "MD031",
            Rule::Md032 => "MD032",
            Rule::Md040 => "MD040",
            Rule::Md042 => "MD042",
            Rule::Md045 => "MD045",
            Rule::Md047 => "MD047",
            Rule::Md055 => "MD055",
            Rule::Md056 => "MD056",
            Rule::Md060 => "MD060",
            Rule::GhAlert => "GHALERT",
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

    /// Ссылка на официальную страницу правила.
    ///
    /// Правый клик по диагностике открывает её сразу, без промежуточных
    /// меню и диалогов: в приложении остаётся только та документация,
    /// которую поддерживает markdownlint.
    fn docs_url(self) -> &'static str {
        match self {
            Rule::Md007 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md007.md",
            Rule::Md009 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md009.md",
            Rule::Md010 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md010.md",
            Rule::Md012 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md012.md",
            Rule::Md018 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md018.md",
            Rule::Md019 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md019.md",
            Rule::Md020 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md020.md",
            Rule::Md021 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md021.md",
            Rule::Md022 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md022.md",
            Rule::Md023 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md023.md",
            Rule::Md024 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md024.md",
            Rule::Md025 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md025.md",
            Rule::Md026 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md026.md",
            Rule::Md029 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md029.md",
            Rule::Md030 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md030.md",
            Rule::Md031 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md031.md",
            Rule::Md032 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md032.md",
            Rule::Md040 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md040.md",
            Rule::Md042 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md042.md",
            Rule::Md045 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md045.md",
            Rule::Md047 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md047.md",
            Rule::Md055 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md055.md",
            Rule::Md056 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md056.md",
            Rule::Md060 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md060.md",
            Rule::GhAlert => "https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#alerts",
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

/// Находит диагностику под указанной позицией.
///
/// Одна и та же функция используется подсказкой при наведении и
/// контекстным меню: так оба способа всегда указывают на одно правило.
pub(super) fn diagnostic_at(
    diagnostics: &[Diagnostic],
    line: i32,
    column: i32,
) -> Option<&Diagnostic> {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.contains(line, column))
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
    /// Делает подчёркнутые фрагменты «ссылками»: Ctrl+клик открывает
    /// документацию правила, а при наведении с зажатым Ctrl курсор
    /// меняется на «палец», как у ссылки в браузере.
    ///
    /// Правый клик не перехватывается вовсе: штатное контекстное меню
    /// редактора остаётся доступным, а линтер не создаёт ни одного
    /// дополнительного виджета.
    pub(super) fn setup_lint_links(self: &Rc<Self>) {
        let hover = Rc::new(LintHoverState::default());

        // --- Курсор при наведении ---
        let motion = gtk4::EventControllerMotion::new();
        {
            let weak_doc = Rc::downgrade(self);
            let hover = hover.clone();
            motion.connect_motion(move |_, x, y| {
                let Some(doc) = weak_doc.upgrade() else {
                    return;
                };

                let rule = {
                    let diagnostics = doc.diagnostics.borrow();
                    rule_at_position(&doc.text_view, &diagnostics, x, y)
                };

                hover.rule.set(rule);
                update_lint_cursor(&doc.text_view, &hover, ctrl_is_down(&doc.text_view));
                // Собственный tooltip-лейбл: не зависит от кэширования
                // GtkTooltip и показывается при каждом наведении.
                doc.update_lint_tooltip(rule, x, y);
            });
        }

        {
            let weak_doc = Rc::downgrade(self);
            let hover = hover.clone();
            motion.connect_leave(move |_| {
                if let Some(doc) = weak_doc.upgrade() {
                    hover.rule.set(None);
                    update_lint_cursor(&doc.text_view, &hover, false);
                    doc.hide_lint_tooltip();
                }
            });
        }
        self.text_view.add_controller(motion);

        // Нажатие/отпускание Ctrl обновляет курсор сразу, без движения
        // мыши. Если фокус не в редакторе, курсор обновится при
        // ближайшем движении — это незаметно на практике.
        let key = gtk4::EventControllerKey::new();
        {
            let weak_doc = Rc::downgrade(self);
            let hover = hover.clone();
            key.connect_key_pressed(move |_, keyval, _, _| {
                if matches!(keyval, gdk::Key::Control_L | gdk::Key::Control_R) {
                    if let Some(doc) = weak_doc.upgrade() {
                        update_lint_cursor(&doc.text_view, &hover, true);
                    }
                }
                glib::Propagation::Proceed
            });
        }
        {
            let weak_doc = Rc::downgrade(self);
            let hover = hover.clone();
            key.connect_key_released(move |_, keyval, _, _| {
                if matches!(keyval, gdk::Key::Control_L | gdk::Key::Control_R) {
                    if let Some(doc) = weak_doc.upgrade() {
                        update_lint_cursor(&doc.text_view, &hover, false);
                    }
                }
            });
        }
        self.text_view.add_controller(key);

        // --- Ctrl+клик по диагностике ---
        // Capture-фаза и Denied для обычных кликов: выделение текста и
        // установка курсора мышью работают как раньше.
        let click = gtk4::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk4::PropagationPhase::Capture);
        {
            let weak_doc = Rc::downgrade(self);
            click.connect_pressed(move |gesture, _n_press, x, y| {
                let Some(doc) = weak_doc.upgrade() else {
                    return;
                };

                if !ctrl_is_down(&doc.text_view) {
                    gesture.set_state(gtk4::EventSequenceState::Denied);
                    return;
                }

                let rule = {
                    let diagnostics = doc.diagnostics.borrow();
                    rule_at_position(&doc.text_view, &diagnostics, x, y)
                };

                if let Some(rule) = rule {
                    gesture.set_state(gtk4::EventSequenceState::Claimed);
                    open_uri(rule.docs_url());
                } else {
                    gesture.set_state(gtk4::EventSequenceState::Denied);
                }
            });
        }
        self.text_view.add_controller(click);

        // При уходе фокуса из редактора сбрасываем курсор: иначе он
        // останется «пальцем» над уже неактивным текстом.
        let focus = gtk4::EventControllerFocus::new();
        {
            let weak_doc = Rc::downgrade(self);
            focus.connect_leave(move |_| {
                if let Some(doc) = weak_doc.upgrade() {
                    hover.rule.set(None);
                    update_lint_cursor(&doc.text_view, &hover, false);
                    doc.hide_lint_tooltip();
                }
            });
        }
        self.text_view.add_controller(focus);
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
        self.hide_lint_tooltip();

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
        self.hide_lint_tooltip();
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

    /// Показывает собственный tooltip рядом с указателем.
    ///
    /// Используется не `GtkTooltip`, а обычная карточка поверх редактора:
    /// так подсказка гарантированно появляется при каждом наведении и не
    /// зависит от внутреннего кэширования GTK. Карточка не перехватывает
    /// события мыши.
    pub(super) fn update_lint_tooltip(&self, rule: Option<Rule>, x: f64, y: f64) {
        let Some(rule) = rule else {
            self.hide_lint_tooltip();
            return;
        };

        let lang = Language::from_code(&self.settings.borrow().language);
        self.lint_tooltip_label.set_text(&format!(
            "{} — {}\n{}",
            rule.id(),
            lang.lint_message(rule.id()),
            lang.lint_tooltip_hint()
        ));

        let Some((x, y)) = self
            .text_view
            .translate_coordinates(&self.lint_tooltip_overlay, x, y)
        else {
            self.hide_lint_tooltip();
            return;
        };

        let overlay_width = self.lint_tooltip_overlay.width().max(1);
        let overlay_height = self.lint_tooltip_overlay.height().max(1);
        let (_, natural_width, _, _) = self.lint_tooltip.measure(gtk4::Orientation::Horizontal, -1);
        let (_, natural_height, _, _) = self
            .lint_tooltip
            .measure(gtk4::Orientation::Vertical, natural_width.max(1));
        let natural_width = natural_width.max(1);
        let natural_height = natural_height.max(1);

        let mut tooltip_x = x + 14.0;
        let mut tooltip_y = y + 20.0;

        // Не даём подсказке выйти за пределы редактора: у правого края
        // показываем её слева от курсора, у нижнего — сверху.
        if tooltip_x + natural_width as f64 > overlay_width as f64 {
            tooltip_x = (x - natural_width as f64 - 14.0).max(0.0);
        }
        if tooltip_y + natural_height as f64 > overlay_height as f64 {
            tooltip_y = (y - natural_height as f64 - 8.0).max(0.0);
        }

        self.lint_tooltip
            .set_margin_start(tooltip_x.max(0.0) as i32);
        self.lint_tooltip.set_margin_top(tooltip_y.max(0.0) as i32);
        self.lint_tooltip.set_visible(true);
    }

    /// Скрывает собственный tooltip линтера.
    pub(super) fn hide_lint_tooltip(&self) {
        if self.lint_tooltip.is_visible() {
            self.lint_tooltip.set_visible(false);
        }
    }
}

/// Данные открывающего fence-блока, нужные для MD031 и MD040.
struct FenceOpen {
    marker: char,
    length: usize,
    info_empty: bool,
}

/// Состояние наведения на диагностику.
///
/// `rule` запоминается, чтобы Ctrl можно было нажать после того, как
/// курсор уже стоит над подчёркиванием: тогда курсор-«палец» включится
/// без дополнительного движения мыши.
#[derive(Default)]
struct LintHoverState {
    rule: Cell<Option<Rule>>,
    /// Текущий курсор уже является «пальцем».
    pointer: Cell<bool>,
}

/// Возвращает правило под координатами виджета или `None`.
fn rule_at_position(
    view: &gtk4::TextView,
    diagnostics: &[Diagnostic],
    x: f64,
    y: f64,
) -> Option<Rule> {
    let (buffer_x, buffer_y) =
        view.window_to_buffer_coords(gtk4::TextWindowType::Widget, x as i32, y as i32);
    let iter = view.iter_at_location(buffer_x, buffer_y)?;
    diagnostic_at(diagnostics, iter.line(), iter.line_offset()).map(|diagnostic| diagnostic.rule)
}

/// Проверяет текущее состояние клавиши Ctrl.
///
/// `EventControllerMotion` не передаёт модификаторы, поэтому состояние
/// запрашивается у клавиатуры напрямую. Для клика и клавиш это не нужно:
/// там модификаторы приходят вместе с событием.
fn ctrl_is_down(view: &gtk4::TextView) -> bool {
    view.display()
        .default_seat()
        .and_then(|seat| seat.keyboard())
        .map(|keyboard| {
            keyboard
                .modifier_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
        })
        .unwrap_or(false)
}

/// Включает или выключает курсор-«палец» над диагностикой.
fn update_lint_cursor(view: &gtk4::TextView, hover: &LintHoverState, ctrl: bool) {
    let pointer = ctrl && hover.rule.get().is_some();
    if hover.pointer.get() == pointer {
        return;
    }

    hover.pointer.set(pointer);
    view.set_cursor_from_name(Some(if pointer { "pointer" } else { "text" }));
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

    // --- MD018–MD026: заголовки ATX ---
    let mut first_level_seen = false;
    let mut heading_texts: HashSet<String> = HashSet::new();

    for (index, line) in lines.iter().enumerate() {
        if is_code[index] {
            continue;
        }

        let Some(heading) = inspect_atx_heading(line) else {
            continue;
        };

        let level = heading.level;
        let content = &line[heading.content_start..heading.content_end];

        // MD018: после решёток сразу идёт текст, без разделителя.
        if !line[heading.opening_end..].is_empty() && heading.opening_spaces == 0 {
            result.push(Diagnostic::new(
                Rule::Md018,
                index,
                heading.indent as i32,
                heading.opening_end as i32,
            ));
        }

        // MD019: больше одного пробела/таба после решёток.
        if heading.opening_spaces >= 2 {
            result.push(Diagnostic::new(
                Rule::Md019,
                index,
                heading.opening_end as i32,
                (heading.opening_end + heading.opening_spaces) as i32,
            ));
        }

        // MD020 и MD021 применяются только к закрытому ATX-стилю
        // (`# Заголовок #`), где решётки есть с обеих сторон.
        if let Some(closing_start) = heading.closing_start {
            let opening_missing_space =
                !line[heading.opening_end..].is_empty() && heading.opening_spaces == 0;
            let closing_missing_space = heading.closing_spaces == 0;

            if opening_missing_space || closing_missing_space {
                let (start_col, end_col) = if closing_missing_space {
                    (closing_start, heading.closing_end)
                } else {
                    (heading.indent, heading.opening_end)
                };
                result.push(Diagnostic::new(
                    Rule::Md020,
                    index,
                    start_col as i32,
                    end_col as i32,
                ));
            }

            if heading.opening_spaces >= 2 || heading.closing_spaces >= 2 {
                let (start_col, end_col) = if heading.closing_spaces >= 2 {
                    (closing_start - heading.closing_spaces, closing_start)
                } else {
                    (
                        heading.opening_end,
                        heading.opening_end + heading.opening_spaces,
                    )
                };
                result.push(Diagnostic::new(
                    Rule::Md021,
                    index,
                    start_col as i32,
                    end_col as i32,
                ));
            }
        }

        // MD023: заголовок сдвинут от начала строки.
        if heading.indent > 0 {
            result.push(Diagnostic::new(
                Rule::Md023,
                index,
                0,
                heading.indent as i32,
            ));
        }

        // MD024: повторяющийся текст заголовка. Номер уровня не важен:
        // по умолчанию markdownlint проверяет весь документ.
        let normalized = content.trim();
        if !normalized.is_empty() && !heading_texts.insert(normalized.to_string()) {
            result.push(Diagnostic::new(
                Rule::Md024,
                index,
                0,
                char_len(line).max(1),
            ));
        }

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

        // MD026: последний видимый символ заголовка не должен быть
        // знаком препинания. Закрывающие решётки уже отрезаны, поэтому
        // они не мешают.
        if heading.content_end > heading.content_start {
            let Some(last_char) = content.chars().next_back() else {
                continue;
            };

            if MD026_PUNCTUATION.contains(last_char)
                && !(last_char == ';' && ends_with_html_entity(content))
            {
                let punctuation_start = heading.content_end - last_char.len_utf8();
                let start_col = char_index(line, punctuation_start);
                result.push(Diagnostic::new(
                    Rule::Md026,
                    index,
                    start_col,
                    start_col + 1,
                ));
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

    // --- MD042 и MD045: ссылки и изображения ---
    check_link_and_image_rules(text, &mut result);

    // --- MD055, MD056 и MD060: таблицы ---
    check_table_rules(&lines, &is_code, &mut result);

    // --- GHALERT: корректность GitHub Alerts ---
    check_github_alerts(&lines, &is_code, &mut result);

    // --- MD007, MD029 и MD030: списки ---
    check_list_rules(&lines, &is_code, &mut result);

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

/// Данные ATX-заголовка, нужные правилам MD018–MD026.
struct AtxHeading {
    level: u8,
    /// Количество пробелов перед первой решёткой.
    indent: usize,
    /// Байтовый индекс сразу после открывающих решёток.
    opening_end: usize,
    /// Количество пробелов/табов между решётками и текстом.
    opening_spaces: usize,
    /// Начало закрывающих решёток, если заголовок закрытый.
    closing_start: Option<usize>,
    /// Конец закрывающих решёток.
    closing_end: usize,
    /// Количество пробелов/табов перед закрывающими решётками.
    closing_spaces: usize,
    /// Границы текста заголовка без решёток и окружающих пробелов.
    content_start: usize,
    content_end: usize,
}

/// Разбирает ATX-заголовок, включая некорректно оформленные варианты.
///
/// В отличие от строгого CommonMark, здесь `#Heading` и `# Heading #`
/// тоже считаются кандидатами: именно их проверяют MD018–MD021.
fn inspect_atx_heading(line: &str) -> Option<AtxHeading> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent > 3 {
        return None;
    }

    let rest = &line[indent..];
    let level = rest.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }

    let opening_end = indent + level;
    let after = &line[opening_end..];
    let opening_spaces = after
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count();

    let trimmed_end = line.trim_end().len();
    let mut closing_start = None;
    let mut closing_end = trimmed_end;
    let mut closing_spaces = 0;
    let mut content_start = opening_end + opening_spaces;
    let mut content_end = trimmed_end;

    if trimmed_end > opening_end && line[..trimmed_end].ends_with('#') {
        let closing_run = line[..trimmed_end]
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'#')
            .count();
        let run_start = trimmed_end - closing_run;

        // Если это та же серия решёток, что и открывающая, закрывающих
        // решёток нет: `####` — просто пустой заголовок.
        if run_start > opening_end {
            closing_start = Some(run_start);
            closing_end = trimmed_end;

            let mut spacing_start = run_start;
            while spacing_start > opening_end
                && matches!(line.as_bytes()[spacing_start - 1], b' ' | b'\t')
            {
                spacing_start -= 1;
            }

            closing_spaces = run_start - spacing_start;
            content_end = spacing_start;
        }
    }

    if content_start > content_end {
        content_start = content_end;
    }

    Some(AtxHeading {
        level: level as u8,
        indent,
        opening_end,
        opening_spaces,
        closing_start,
        closing_end,
        closing_spaces,
        content_start,
        content_end,
    })
}

/// Проверяет, что текст заканчивается HTML-сущностью (`&copy;`, `&#169;`).
///
/// markdownlint не считает точку с запятой в конце такой сущности
/// знаком препинания заголовка. Без этой проверки `## Copyright &copy;`
/// получал бы ложное предупреждение.
fn ends_with_html_entity(content: &str) -> bool {
    let Some(before_semicolon) = content.strip_suffix(';') else {
        return false;
    };
    let Some(ampersand) = before_semicolon.rfind('&') else {
        return false;
    };

    let entity = &before_semicolon[ampersand + 1..];
    if entity.is_empty() {
        return false;
    }

    if let Some(hex) = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"))
    {
        return !hex.is_empty() && hex.chars().all(|ch| ch.is_ascii_hexdigit());
    }

    if let Some(decimal) = entity.strip_prefix('#') {
        return !decimal.is_empty() && decimal.chars().all(|ch| ch.is_ascii_digit());
    }

    entity.chars().all(|ch| ch.is_ascii_alphanumeric())
}

/// Вид маркера списка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListMarkerKind {
    /// `-`, `*` или `+`.
    Unordered,
    /// `1.` или `1)`; хранит само число.
    Ordered(u32),
}

impl ListMarkerKind {
    fn is_unordered(self) -> bool {
        matches!(self, Self::Unordered)
    }
}

/// Разобранный элемент списка.
struct ListItem {
    indent: usize,
    kind: ListMarkerKind,
    marker_start: usize,
    marker_end: usize,
    spaces_after: usize,
    content_start: usize,
    content_end: usize,
}

impl ListItem {
    fn has_content(&self) -> bool {
        self.content_start < self.content_end
    }
}

/// Возвращает отступ элемента списка в символах или `None`.
fn list_item_indent(line: &str) -> Option<usize> {
    parse_list_item(line).map(|item| item.indent)
}

/// Разбирает маркер списка и позиции его частей.
///
/// Поддерживаются `-`, `*`, `+` и упорядоченные `1.` / `1)`. После
/// маркера обязателен пробел или таб — иначе это не элемент списка.
fn parse_list_item(line: &str) -> Option<ListItem> {
    let indent = indent_chars(line);
    let prefix_bytes: usize = line.chars().take(indent).map(char::len_utf8).sum();
    let rest = &line[prefix_bytes..];

    let first = rest.chars().next()?;
    let (kind, marker_len) = if matches!(first, '-' | '*' | '+')
        && matches!(rest.chars().nth(1), Some(' ') | Some('\t'))
    {
        (ListMarkerKind::Unordered, first.len_utf8())
    } else {
        parse_ordered_marker(rest)?
    };

    let marker_start = prefix_bytes;
    let marker_end = marker_start + marker_len;
    let after_marker = &line[marker_end..];
    let spaces_after = after_marker
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count();

    if spaces_after == 0 {
        return None;
    }

    let content_start = marker_end + spaces_after;
    let content_end = line.trim_end().len();

    Some(ListItem {
        indent,
        kind,
        marker_start,
        marker_end,
        spaces_after,
        content_start,
        content_end,
    })
}

/// Разбирает `1.` или `1)` в начале строки.
fn parse_ordered_marker(rest: &str) -> Option<(ListMarkerKind, usize)> {
    let bytes = rest.as_bytes();
    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();

    if digits == 0 || digits > 9 || digits >= bytes.len() {
        return None;
    }

    let marker = bytes[digits];
    let after = bytes.get(digits + 1).copied();
    if !(marker == b'.' || marker == b')') || !matches!(after, Some(b' ') | Some(b'\t')) {
        return None;
    }

    let number = rest[..digits].parse::<u32>().ok()?;
    Some((ListMarkerKind::Ordered(number), digits + 1))
}

/// Один упорядоченный элемент для MD029.
struct OrderedItem {
    line: usize,
    number: u32,
    marker_start: usize,
    marker_end: usize,
}

/// Проверяет отступы, нумерацию и пробелы после маркеров списков.
///
/// Это отдельный проход: правила MD007/MD029/MD030 смотрят на маркер,
/// а не на границы блока, как MD032. Состояние вложенности хранится в
/// стеке индексов отступов, поэтому вложенные списки не путаются с
/// соседними списками верхнего уровня.
fn check_list_rules(lines: &[&str], is_code: &[bool], result: &mut Vec<Diagnostic>) {
    let mut list_stack: Vec<(usize, bool)> = Vec::new();
    let mut ordered_runs: Vec<(usize, Vec<OrderedItem>)> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if is_code[index] {
            continue;
        }

        if line.trim().is_empty() {
            // Пустая строка не завершает список: элементы могут быть
            // разделены пустыми строками.
            continue;
        }

        let Some(item) = parse_list_item(line) else {
            // Непустая строка без маркера. Строка с отступом — это
            // продолжение элемента списка; строка без отступа — новый
            // блок, поэтому состояние вложенности сбрасывается.
            if indent_chars(line) == 0 {
                flush_ordered_runs(&mut ordered_runs, result);
                list_stack.clear();
            }
            continue;
        };

        // --- MD030: ровно один пробел после маркера ---
        if item.has_content() {
            let whitespace = &line[item.marker_end..item.content_start];
            if item.spaces_after != 1 || whitespace.starts_with('\t') {
                result.push(Diagnostic::new(
                    Rule::Md030,
                    index,
                    item.marker_end as i32,
                    item.content_start as i32,
                ));
            }
        }

        // --- MD007: отступ вложенного неупорядоченного списка ---
        while list_stack
            .last()
            .is_some_and(|(indent, _)| *indent > item.indent)
        {
            list_stack.pop();
        }

        let is_new_level = !list_stack
            .last()
            .is_some_and(|(indent, _)| *indent >= item.indent);
        if is_new_level {
            list_stack.push((item.indent, item.kind.is_unordered()));
        } else if let Some((_, unordered)) = list_stack.last_mut() {
            *unordered = item.kind.is_unordered();
        }

        let level = list_stack.len().saturating_sub(1);
        let parents_are_unordered = list_stack[..level].iter().all(|(_, unordered)| *unordered);

        if item.kind.is_unordered() && parents_are_unordered {
            let expected = level * 2;
            if item.indent != expected {
                result.push(Diagnostic::new(
                    Rule::Md007,
                    index,
                    0,
                    item.indent.max(1) as i32,
                ));
            }
        }

        // --- MD029: нумерация упорядоченных списков ---
        if let ListMarkerKind::Ordered(number) = item.kind {
            // Завершаем только более глубокие упорядоченные списки:
            // элемент на том же отступе продолжает текущий блок.
            while ordered_runs
                .last()
                .is_some_and(|(indent, _)| *indent > item.indent)
            {
                if let Some((_, run)) = ordered_runs.pop() {
                    flush_ordered_run(&run, result);
                }
            }

            if let Some((indent, run)) = ordered_runs.last_mut() {
                if *indent == item.indent {
                    run.push(OrderedItem {
                        line: index,
                        number,
                        marker_start: item.marker_start,
                        marker_end: item.marker_end,
                    });
                } else {
                    ordered_runs.push((
                        item.indent,
                        vec![OrderedItem {
                            line: index,
                            number,
                            marker_start: item.marker_start,
                            marker_end: item.marker_end,
                        }],
                    ));
                }
            } else {
                ordered_runs.push((
                    item.indent,
                    vec![OrderedItem {
                        line: index,
                        number,
                        marker_start: item.marker_start,
                        marker_end: item.marker_end,
                    }],
                ));
            }
        } else {
            // Неупорядоченный элемент завершает упорядоченные блоки
            // на том же или более глубоком уровне.
            while ordered_runs
                .last()
                .is_some_and(|(indent, _)| *indent >= item.indent)
            {
                if let Some((_, run)) = ordered_runs.pop() {
                    flush_ordered_run(&run, result);
                }
            }
        }
    }

    flush_ordered_runs(&mut ordered_runs, result);
}

/// Завершает все накопленные упорядоченные списки.
fn flush_ordered_runs(runs: &mut Vec<(usize, Vec<OrderedItem>)>, result: &mut Vec<Diagnostic>) {
    while let Some((_, run)) = runs.pop() {
        flush_ordered_run(&run, result);
    }
}

/// Проверяет один упорядоченный список.
///
/// По умолчанию markdownlint использует стиль `one_or_ordered`: либо
/// все номера равны 1, либо номера идут подряд. Нумерация начинается
/// с того числа, которое указал автор: `0, 1, 2` и `1, 2, 3` корректны.
fn flush_ordered_run(run: &[OrderedItem], result: &mut Vec<Diagnostic>) {
    if run.len() < 2 {
        return;
    }

    if run.iter().all(|item| item.number == 1) {
        return;
    }

    let first = run[0].number;
    for (offset, item) in run.iter().enumerate() {
        let expected = first + offset as u32;
        if item.number != expected {
            result.push(Diagnostic::new(
                Rule::Md029,
                item.line,
                item.marker_start as i32,
                item.marker_end as i32,
            ));
        }
    }
}

/// Проверяет пустые ссылки (MD042) и изображения без alt-текста (MD045).
///
/// Для инлайн-разметки используется `pulldown-cmark`: он уже знает, что
/// находится внутри кода или HTML, и отдаёт байтовые диапазоны событий.
/// Диапазоны переводятся в номера строк и колонки в символах.
fn check_link_and_image_rules(text: &str, result: &mut Vec<Diagnostic>) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(text, options).into_offset_iter();
    // Alt-текст изображения может состоять из нескольких событий
    // (текст и inline-код), поэтому накапливаем его в стеке.
    let mut images: Vec<(Range<usize>, String)> = Vec::new();

    for (event, range) in parser {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                let destination = dest_url.trim();
                if destination.is_empty() || destination == "#" {
                    push_range_diagnostic(text, result, Rule::Md042, range);
                }
            }
            Event::Start(Tag::Image { .. }) => {
                images.push((range, String::new()));
            }
            Event::Text(value) | Event::Code(value) => {
                if let Some((_, alt)) = images.last_mut() {
                    alt.push_str(&value);
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some((range, alt)) = images.pop() {
                    if alt.trim().is_empty() {
                        push_range_diagnostic(text, result, Rule::Md045, range);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Добавляет диагностику по байтовому диапазону события.
///
/// Если событие занимает несколько строк, подчёркиваем первый видимый
/// символ: диапазон `GtkTextTag` всегда должен быть в пределах строки.
fn push_range_diagnostic(
    text: &str,
    result: &mut Vec<Diagnostic>,
    rule: Rule,
    range: Range<usize>,
) {
    let (line, start_col) = position_at(text, range.start);
    let (end_line, end_col) = position_at(text, range.end);

    let end_col = if end_line == line && end_col > start_col {
        end_col
    } else {
        start_col + 1
    };

    result.push(Diagnostic::new(rule, line as usize, start_col, end_col));
}

/// Переводит байтовое смещение в номер строки и колонку в символах.
fn position_at(text: &str, byte_offset: usize) -> (i32, i32) {
    let offset = byte_offset.min(text.len());
    let before = &text[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() as i32;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let column = before[line_start..].chars().count() as i32;
    (line, column)
}

/// Разобранная строка таблицы.
struct TableRow {
    line: usize,
    leading_pipe: bool,
    trailing_pipe: bool,
    /// Содержимое ячеек между pipe-символами, без внешних разделителей.
    cells: Vec<String>,
    /// Позиции pipe-символов внутри обрезанной строки (в символах).
    pipes: Vec<usize>,
}

/// Проверяет стиль и структуру GFM-таблиц.
///
/// Таблица определяется по паре «строка заголовка + разделитель». Дальше
/// строки идут до пустой строки или начала другого блока. Текст сразу
/// после таблицы без пустой строки по правилам GFM считается её частью,
/// поэтому такие строки тоже проверяются.
fn check_table_rules(lines: &[&str], is_code: &[bool], result: &mut Vec<Diagnostic>) {
    // Стиль первой таблицы в документе, который должен сохраняться
    // во всех последующих таблицах (MD055, default `consistent`).
    let mut document_pipe_style: Option<(bool, bool)> = None;
    let mut index = 0;

    while index + 1 < lines.len() {
        if is_code[index] || is_code[index + 1] {
            index += 1;
            continue;
        }

        let Some(header) = parse_table_row(lines[index], index) else {
            index += 1;
            continue;
        };
        let Some(delimiter) = parse_table_row(lines[index + 1], index + 1) else {
            index += 1;
            continue;
        };

        if !is_delimiter_row(&delimiter) || header.cells.len() != delimiter.cells.len() {
            index += 1;
            continue;
        }

        let mut body: Vec<TableRow> = Vec::new();
        let mut end = index + 2;
        while end < lines.len() {
            if lines[end].trim().is_empty() || is_code[end] || starts_new_block(lines[end]) {
                break;
            }

            if let Some(row) = parse_table_row(lines[end], end) {
                body.push(row);
            } else {
                // Текст без pipe-символов — всё ещё часть таблицы по GFM.
                body.push(TableRow {
                    line: end,
                    leading_pipe: false,
                    trailing_pipe: false,
                    cells: vec![lines[end].trim().to_string()],
                    pipes: Vec::new(),
                });
            }
            end += 1;
        }

        let style = (header.leading_pipe, header.trailing_pipe);
        document_pipe_style.get_or_insert(style);

        let expected_columns = header.cells.len();
        let mut rows: Vec<&TableRow> = Vec::with_capacity(2 + body.len());
        rows.push(&header);
        rows.push(&delimiter);
        rows.extend(body.iter());

        for row in &rows {
            // MD055: единый стиль ведущих и замыкающих pipe.
            if let Some(expected_style) = document_pipe_style {
                if (row.leading_pipe, row.trailing_pipe) != expected_style {
                    result.push(Diagnostic::new(
                        Rule::Md055,
                        row.line,
                        0,
                        char_len(lines[row.line]).max(1),
                    ));
                }
            }

            // MD056: число колонок должно совпадать с заголовком.
            if row.cells.len() != expected_columns {
                result.push(Diagnostic::new(
                    Rule::Md056,
                    row.line,
                    0,
                    char_len(lines[row.line]).max(1),
                ));
            }
        }

        // MD060: таблица должна соответствовать хотя бы одному стилю —
        // выровненному, компактному или плотному. Иначе сообщаем один
        // раз на строке заголовка, а не на каждой ячейке.
        if !table_matches_aligned(&rows)
            && !table_matches_compact(&rows)
            && !table_matches_tight(&rows)
        {
            result.push(Diagnostic::new(
                Rule::Md060,
                header.line,
                0,
                char_len(lines[header.line]).max(1),
            ));
        }

        index = end;
    }
}

/// Разбирает строку таблицы, сохраняя внешние pipe-признаки и ячейки.
fn parse_table_row(line: &str, line_index: usize) -> Option<TableRow> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }

    let mut segments: Vec<String> = Vec::new();
    let mut pipes: Vec<usize> = Vec::new();
    let mut current = String::new();
    let mut escaped = false;

    for (char_index, ch) in trimmed.chars().enumerate() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            current.push(ch);
            escaped = true;
        } else if ch == '|' {
            segments.push(std::mem::take(&mut current));
            pipes.push(char_index);
        } else {
            current.push(ch);
        }
    }
    segments.push(current);

    let leading_pipe = segments.first().is_some_and(String::is_empty);
    let trailing_pipe = segments.last().is_some_and(String::is_empty);

    if leading_pipe {
        segments.remove(0);
    }
    if trailing_pipe {
        segments.pop();
    }

    if segments.is_empty() {
        return None;
    }

    Some(TableRow {
        line: line_index,
        leading_pipe,
        trailing_pipe,
        cells: segments,
        pipes,
    })
}

/// Проверяет, что все ячейки разделителя выглядят как `---`, `:---` и т.п.
fn is_delimiter_row(row: &TableRow) -> bool {
    !row.cells.is_empty()
        && row.cells.iter().all(|cell| {
            let trimmed = cell.trim();
            let without_left = trimmed.strip_prefix(':').unwrap_or(trimmed);
            let without_both = without_left.strip_suffix(':').unwrap_or(without_left);
            !without_both.is_empty() && without_both.chars().all(|ch| ch == '-')
        })
}

/// Строка начинает новый блок и поэтому завершает таблицу.
fn starts_new_block(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#')
        || trimmed.starts_with('>')
        || trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || parse_list_item(line).is_some()
}

/// Проверяет стиль «выровненные pipe-символы».
fn table_matches_aligned(rows: &[&TableRow]) -> bool {
    let Some(first) = rows.first() else {
        return false;
    };
    !first.pipes.is_empty() && rows.iter().all(|row| row.pipes == first.pipes)
}

/// Проверяет стиль «один пробел вокруг содержимого каждой ячейки».
fn table_matches_compact(rows: &[&TableRow]) -> bool {
    rows.iter().all(|row| {
        let cell_count = row.cells.len();
        row.cells.iter().enumerate().all(|(index, cell)| {
            let need_left = row.leading_pipe || index > 0;
            let need_right = row.trailing_pipe || index + 1 < cell_count;
            let content = cell.trim();

            if content.is_empty() {
                // Пустая ячейка в компактном стиле — ровно один пробел,
                // если хотя бы с одной стороны есть pipe.
                return if need_left || need_right {
                    cell == " "
                } else {
                    cell.is_empty()
                };
            }

            let mut core = cell.as_str();
            if need_left {
                let Some(stripped) = core.strip_prefix(' ') else {
                    return false;
                };
                core = stripped;
            }
            if need_right {
                let Some(stripped) = core.strip_suffix(' ') else {
                    return false;
                };
                core = stripped;
            }

            core == content
        })
    })
}

/// Проверяет плотный стиль: вокруг содержимого ячеек нет пробелов.
fn table_matches_tight(rows: &[&TableRow]) -> bool {
    rows.iter()
        .all(|row| row.cells.iter().all(|cell| cell.trim() == cell.as_str()))
}

/// Проверяет типы GitHub Alerts.
///
/// Это не правило markdownlint, а собственная диагностика Aster: она
/// помогает заметить `[!alert!]`, неизвестные типы и текст после типа,
/// которые GitHub не превратит в callout. Валидные типы рендерер
/// поддерживает независимо от регистра.
fn check_github_alerts(lines: &[&str], is_code: &[bool], result: &mut Vec<Diagnostic>) {
    for (index, line) in lines.iter().enumerate() {
        if is_code[index] {
            continue;
        }
        if let Some((start_col, end_col)) = invalid_alert_marker(line) {
            result.push(Diagnostic::new(Rule::GhAlert, index, start_col, end_col));
        }
    }
}

/// Возвращает границы `[!TYPE]`, если alert-маркер некорректен.
fn invalid_alert_marker(line: &str) -> Option<(i32, i32)> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent > 3 {
        return None;
    }

    let mut rest = &line[indent..];
    let mut offset = indent;
    let mut in_blockquote = false;

    // Пропускаем маркеры вложенных цитат: `> > [!NOTE]`.
    while let Some(after_marker) = rest.strip_prefix('>') {
        in_blockquote = true;
        offset += 1;
        rest = after_marker;
        if let Some(after_space) = rest.strip_prefix(' ') {
            offset += 1;
            rest = after_space;
        }
    }

    if !in_blockquote || !rest.starts_with("[!") {
        return None;
    }

    let close = rest.find(']')?;
    let marker = &rest[..=close];
    let kind = &rest[2..close];
    let trailing = rest[close + 1..].trim();

    let valid_kind = matches!(
        kind.to_ascii_lowercase().as_str(),
        "note" | "tip" | "important" | "warning" | "caution"
    );

    if valid_kind && trailing.is_empty() {
        return None;
    }

    let start_col = offset as i32;
    let end_col = start_col + marker.chars().count() as i32;
    Some((start_col, end_col))
}
