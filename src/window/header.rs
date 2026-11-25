//! Заголовочная панель (`adw::HeaderBar`), её кнопки и синхронизация состояния.
//!
//! Модуль вынесен из `window.rs`, чтобы отделить создание и обновление
//! элементов заголовка от логики вкладок, сессии и закрытия окна.

use adw::prelude::*;
use gtk4::gio;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::document::Document;
use crate::i18n::Language;
use crate::settings::AppSettings;

use super::menus::{rebuild_main_menu, rebuild_tab_menu};
use super::tabs::active_document;

/// Виджеты заголовка и меню окна, требующие обновления при смене вкладки или языка.
#[derive(Clone)]
pub(super) struct HeaderWidgets {
    pub header_bar: adw::HeaderBar,
    pub open_btn: gtk4::Button,
    pub save_btn: gtk4::Button,
    pub edit_btn: gtk4::ToggleButton,
    pub preview_btn: gtk4::ToggleButton,
    pub search_btn: gtk4::ToggleButton,
    pub menu_btn: gtk4::MenuButton,
    pub menu: gio::Menu,
    pub new_tab_btn: gtk4::Button,
    pub tab_menu: gio::Menu,
}

/// Собирает `adw::HeaderBar`, кнопки управления и модели меню.
pub(super) fn build_header(lang: Language) -> HeaderWidgets {
    let header_bar = adw::HeaderBar::new();

    let open_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    open_btn.set_tooltip_text(Some(lang.tooltip_open()));
    header_bar.pack_start(&open_btn);

    let save_btn = gtk4::Button::from_icon_name("document-save-symbolic");
    save_btn.set_tooltip_text(Some(lang.tooltip_save()));
    header_bar.pack_start(&save_btn);

    // Переключатель «Редактор / Просмотр» применяется к активной вкладке.
    let edit_btn = gtk4::ToggleButton::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(lang.tooltip_edit_mode())
        .active(true)
        .build();
    let preview_btn = gtk4::ToggleButton::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text(lang.tooltip_preview_mode())
        .group(&edit_btn)
        .build();
    let switch_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    switch_box.add_css_class("linked");
    switch_box.append(&edit_btn);
    switch_box.append(&preview_btn);
    header_bar.set_title_widget(Some(&switch_box));

    let search_btn = gtk4::ToggleButton::builder()
        .icon_name("edit-find-symbolic")
        .tooltip_text(lang.tooltip_search())
        .build();
    header_bar.pack_end(&search_btn);

    let menu_btn = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(lang.tooltip_menu())
        .build();

    let menu = gio::Menu::new();
    rebuild_main_menu(&menu, lang);
    menu_btn.set_menu_model(Some(&menu));
    header_bar.pack_end(&menu_btn);

    let new_tab_btn = gtk4::Button::from_icon_name("tab-new-symbolic");
    new_tab_btn.add_css_class("flat");
    new_tab_btn.set_tooltip_text(Some(lang.tooltip_new_tab()));

    let tab_menu = gio::Menu::new();
    rebuild_tab_menu(&tab_menu, lang);

    HeaderWidgets {
        header_bar,
        open_btn,
        save_btn,
        edit_btn,
        preview_btn,
        search_btn,
        menu_btn,
        menu,
        new_tab_btn,
        tab_menu,
    }
}

/// Синхронизирует заголовок окна, акцент кнопки «Сохранить» и переключатели
/// режима/поиска с активной вкладкой.
pub(super) fn sync_header_with_active_tab(
    window: &adw::ApplicationWindow,
    header: &HeaderWidgets,
    tab_view: &adw::TabView,
    docs: &Rc<RefCell<Vec<Rc<Document>>>>,
    settings: &Rc<RefCell<AppSettings>>,
    suppress_mode_sync: &Rc<Cell<bool>>,
    suppress_search_sync: &Rc<Cell<bool>>,
) {
    let lang = Language::from_code(&settings.borrow().language);
    let Some(doc) = active_document(tab_view, docs) else {
        window.set_title(Some("Aster"));
        header.save_btn.set_sensitive(false);
        header.edit_btn.set_sensitive(false);
        header.preview_btn.set_sensitive(false);
        header.search_btn.set_sensitive(false);
        return;
    };

    header.save_btn.set_sensitive(true);
    header.edit_btn.set_sensitive(true);
    header.preview_btn.set_sensitive(true);
    header.search_btn.set_sensitive(true);

    window.set_title(Some(&format!("{} — Aster", doc.window_title(lang))));

    let is_modified = doc.is_modified.get();
    if is_modified {
        header.save_btn.add_css_class("suggested-action");
        header
            .save_btn
            .set_tooltip_text(Some(lang.tooltip_save_unsaved()));
    } else {
        header.save_btn.remove_css_class("suggested-action");
        header.save_btn.set_tooltip_text(Some(lang.tooltip_save()));
    }

    // Синхронизируем кнопки режима с состоянием активной вкладки.
    // Флаг подавляет обработчики `toggled`, чтобы программное
    // переключение не вызывало повторный рендеринг предпросмотра.
    suppress_mode_sync.set(true);
    header.edit_btn.set_active(!doc.is_preview_active.get());
    header.preview_btn.set_active(doc.is_preview_active.get());
    suppress_mode_sync.set(false);

    suppress_search_sync.set(true);
    header
        .search_btn
        .set_active(doc.search_bar.is_search_mode());
    suppress_search_sync.set(false);
}

/// Обновляет тултипы заголовка, меню и подписи всех открытых вкладок при смене языка.
pub(super) fn apply_header_language(
    header: &HeaderWidgets,
    docs: &Rc<RefCell<Vec<Rc<Document>>>>,
    lang: Language,
) {
    header.open_btn.set_tooltip_text(Some(lang.tooltip_open()));
    header.save_btn.set_tooltip_text(Some(lang.tooltip_save()));
    header
        .edit_btn
        .set_tooltip_text(Some(lang.tooltip_edit_mode()));
    header
        .preview_btn
        .set_tooltip_text(Some(lang.tooltip_preview_mode()));
    header
        .search_btn
        .set_tooltip_text(Some(lang.tooltip_search()));
    header.menu_btn.set_tooltip_text(Some(lang.tooltip_menu()));
    header
        .new_tab_btn
        .set_tooltip_text(Some(lang.tooltip_new_tab()));

    rebuild_main_menu(&header.menu, lang);
    rebuild_tab_menu(&header.tab_menu, lang);

    for doc in docs.borrow().iter() {
        doc.apply_language(lang);
    }
}
