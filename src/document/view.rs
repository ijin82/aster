//! Построение дерева виджетов одной вкладки документа.
//!
//! Вынесено из `document/mod.rs`, чтобы конструктор [`super::Document::new`]
//! отвечал за связывание состояния и загрузку файла, а не за 250 строк
//! создания кнопок, контейнеров и слоёв GTK.

use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use webkit6::UserContentManager;

use crate::highlighter;
use crate::i18n::Language;
use crate::settings::AppSettings;

use super::diagnostics::{self, Diagnostic};
use super::gutter::setup_line_numbers_gutter;
use super::preview::build_preview_web_view;
use super::whitespace::setup_whitespace_layer;

/// Набор виджетов, собираемых для одной вкладки документа.
pub(super) struct DocumentWidgets {
    pub root: gtk4::Box,
    pub view_stack: adw::ViewStack,
    pub search_bar: gtk4::SearchBar,
    pub search_entry: gtk4::SearchEntry,
    pub search_prev_btn: gtk4::Button,
    pub search_next_btn: gtk4::Button,
    pub search_status: gtk4::Label,
    pub replace_entry: gtk4::Entry,
    pub replace_toggle: gtk4::ToggleButton,
    pub replace_btn: gtk4::Button,
    pub replace_all_btn: gtk4::Button,
    pub replace_row: gtk4::Box,
    pub diagnostics: Rc<RefCell<Vec<Diagnostic>>>,
    pub lint_tooltip_overlay: gtk4::Overlay,
    pub lint_tooltip_layer: gtk4::Fixed,
    pub lint_tooltip: gtk4::Box,
    pub lint_tooltip_label: gtk4::Label,
    pub buffer: gtk4::TextBuffer,
    pub text_view: gtk4::TextView,
    pub editor_scroll: gtk4::ScrolledWindow,
    pub whitespace_area: gtk4::DrawingArea,
    pub show_whitespace: Rc<Cell<bool>>,
    pub gutter: gtk4::DrawingArea,
    pub tag_table: gtk4::TextTagTable,
    pub user_content_manager: UserContentManager,
    pub web_view: webkit6::WebView,
    pub preview_scroll: gtk4::ScrolledWindow,
}

/// Собирает все виджеты вкладки: редактор, gutter, слой пробелов,
/// собственный tooltip линтера, панель поиска/замены и предпросмотр WebKit.
pub(super) fn build_document_widgets(
    settings: &Rc<RefCell<AppSettings>>,
    lang: Language,
) -> DocumentWidgets {
    // --- Общие CSS-классы и тема редактора ---
    // Шрифт редактора задается CSS-правилом в `window/style.rs`
    // (`textview.editor-view text { font-family; font-size; }`), а не
    // текстовым тегом на диапазоне. Это принципиально: тег применялся
    // только к уже существующим символам, поэтому новый текст в пустом
    // документе получал системный шрифт по умолчанию.
    let tag_table = gtk4::TextTagTable::new();
    let style_mgr = adw::StyleManager::default();
    highlighter::register_highlight_tags(
        &tag_table,
        &settings.borrow().theme,
        style_mgr.is_dark(),
    );

    // Теги диагностики живут в той же таблице, что и подсветка
    // синтаксиса, но применяются отдельно: линтер не конфликтует
    // с `md-*` и может быть выключен без пересоздания буфера.
    diagnostics::register_tags(&tag_table);

    let buffer = gtk4::TextBuffer::builder()
        .tag_table(&tag_table)
        .enable_undo(true)
        .build();
    let text_view = gtk4::TextView::builder()
        .buffer(&buffer)
        .wrap_mode(gtk4::WrapMode::Word)
        .left_margin(18)
        .right_margin(18)
        .top_margin(14)
        .bottom_margin(14)
        .build();
    text_view.add_css_class("editor-view");

    // Список диагностик создаётся до gutter: колонка номеров строк
    // получает его копию и рисует маркеры найденных проблем.
    let diagnostics = Rc::new(RefCell::new(Vec::new()));

    let gutter = setup_line_numbers_gutter(&text_view, settings, &diagnostics);
    let editor_scroll = gtk4::ScrolledWindow::builder()
        .child(&text_view)
        .hexpand(true)
        .vexpand(true)
        .build();

    // Прозрачный слой поверх редактора для точек пробелов и линий табов.
    let whitespace_area = gtk4::DrawingArea::new();
    whitespace_area.set_hexpand(true);
    whitespace_area.set_vexpand(true);
    whitespace_area.set_halign(gtk4::Align::Fill);
    whitespace_area.set_valign(gtk4::Align::Fill);
    whitespace_area.set_can_target(false);
    let show_whitespace = Rc::new(Cell::new(settings.borrow().show_whitespace));
    setup_whitespace_layer(&whitespace_area, &text_view, show_whitespace.clone());

    let editor_overlay = gtk4::Overlay::new();
    editor_overlay.set_child(Some(&editor_scroll));
    editor_overlay.add_overlay(&whitespace_area);

    // Собственный tooltip вместо GtkTooltip. Штатный механизм GTK
    // перестаёт показывать подсказку после нескольких срабатываний
    // на одном и том же виджете, поэтому рисуем карточку сами.
    let lint_tooltip_label = gtk4::Label::new(None);
    lint_tooltip_label.set_xalign(0.0);
    lint_tooltip_label.set_wrap(false);
    lint_tooltip_label.set_margin_top(8);
    lint_tooltip_label.set_margin_bottom(8);
    lint_tooltip_label.set_margin_start(10);
    lint_tooltip_label.set_margin_end(10);

    let lint_tooltip = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    lint_tooltip.add_css_class("lint-tooltip");
    lint_tooltip.set_can_target(false);
    lint_tooltip.set_halign(gtk4::Align::Start);
    lint_tooltip.set_valign(gtk4::Align::Start);
    lint_tooltip.set_visible(false);
    lint_tooltip.append(&lint_tooltip_label);

    let lint_tooltip_layer = gtk4::Fixed::new();
    lint_tooltip_layer.set_hexpand(true);
    lint_tooltip_layer.set_vexpand(true);
    lint_tooltip_layer.set_can_target(false);
    lint_tooltip_layer.put(&lint_tooltip, 0.0, 0.0);
    editor_overlay.add_overlay(&lint_tooltip_layer);

    // Перерисовываем номера строк и слой пробелов при любой прокрутке.
    let gutter_for_scroll = gutter.clone();
    let whitespace_for_vscroll = whitespace_area.clone();
    editor_scroll.vadjustment().connect_value_changed(move |_| {
        gutter_for_scroll.queue_draw();
        whitespace_for_vscroll.queue_draw();
    });
    let whitespace_for_hscroll = whitespace_area.clone();
    editor_scroll.hadjustment().connect_value_changed(move |_| {
        whitespace_for_hscroll.queue_draw();
    });

    // --- Панель поиска ---
    let search_bar = gtk4::SearchBar::new();
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some(lang.search_placeholder()));

    let search_prev_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    search_prev_btn.add_css_class("flat");
    search_prev_btn.set_tooltip_text(Some(lang.tooltip_search_previous()));
    search_prev_btn.set_sensitive(false);

    let search_next_btn = gtk4::Button::from_icon_name("go-down-symbolic");
    search_next_btn.add_css_class("flat");
    search_next_btn.set_tooltip_text(Some(lang.tooltip_search_next()));
    search_next_btn.set_sensitive(false);

    let search_status = gtk4::Label::new(None);
    search_status.add_css_class("dim-label");
    search_status.add_css_class("numeric");
    search_status.set_width_chars(9);
    search_status.set_xalign(0.5);

    let replace_toggle = gtk4::ToggleButton::builder()
        .icon_name("edit-find-replace-symbolic")
        .tooltip_text(lang.tooltip_replace())
        .build();
    replace_toggle.add_css_class("flat");
    replace_toggle.set_active(false);

    let replace_entry = gtk4::Entry::new();
    replace_entry.set_hexpand(true);
    replace_entry.set_placeholder_text(Some(lang.replace_placeholder()));
    replace_entry.set_visible(false);

    let replace_btn = gtk4::Button::with_label(lang.btn_replace());
    replace_btn.add_css_class("flat");
    replace_btn.set_visible(false);
    replace_btn.set_sensitive(false);

    let replace_all_btn = gtk4::Button::with_label(lang.btn_replace_all());
    replace_all_btn.add_css_class("flat");
    replace_all_btn.set_visible(false);
    replace_all_btn.set_sensitive(false);

    let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    search_row.append(&search_entry);
    search_row.append(&search_status);
    search_row.append(&search_prev_btn);
    search_row.append(&search_next_btn);
    search_row.append(&replace_toggle);

    let replace_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    replace_row.append(&replace_entry);
    replace_row.append(&replace_btn);
    replace_row.append(&replace_all_btn);
    replace_row.set_visible(false);

    let search_rows = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    search_rows.append(&search_row);
    search_rows.append(&replace_row);
    search_bar.set_child(Some(&search_rows));
    search_bar.connect_entry(&search_entry);

    // --- Стек режимов документа ---
    let view_stack = adw::ViewStack::new();
    view_stack.add_titled(&editor_overlay, Some("editor"), lang.tab_editor());

    // --- Предпросмотр WebKit ---
    let (user_content_manager, web_view) = build_preview_web_view();
    let preview_scroll = gtk4::ScrolledWindow::builder()
        .child(&web_view)
        .hexpand(true)
        .vexpand(true)
        .build();
    view_stack.add_titled(&preview_scroll, Some("preview"), lang.tab_preview());

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.append(&search_bar);
    root.append(&view_stack);

    DocumentWidgets {
        root,
        view_stack,
        search_bar,
        search_entry,
        search_prev_btn,
        search_next_btn,
        search_status,
        replace_entry,
        replace_toggle,
        replace_btn,
        replace_all_btn,
        replace_row,
        diagnostics,
        lint_tooltip_overlay: editor_overlay,
        lint_tooltip_layer,
        lint_tooltip,
        lint_tooltip_label,
        buffer,
        text_view,
        editor_scroll,
        whitespace_area,
        show_whitespace,
        gutter,
        tag_table,
        user_content_manager,
        web_view,
        preview_scroll,
    }
}
