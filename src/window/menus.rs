//! Построение главного меню и контекстного меню вкладок.

use gtk4::gio;

use crate::i18n::Language;

/// Перестраивает главное меню при смене языка интерфейса.
pub(crate) fn rebuild_main_menu(menu: &gio::Menu, lang: Language) {
    menu.remove_all();
    menu.append(Some(lang.menu_new_tab()), Some("win.new_tab"));
    menu.append(Some(lang.menu_find_replace()), Some("win.find_replace"));
    menu.append(Some(lang.menu_save_as()), Some("win.save_as"));
    menu.append(Some(lang.menu_preferences()), Some("win.preferences"));
    menu.append(Some(lang.menu_about()), Some("win.about"));
    menu.append(Some(lang.menu_close_tab()), Some("win.close_tab"));
    menu.append(Some(lang.menu_close()), Some("win.close"));
}

/// Перестраивает контекстное меню вкладок.
pub(crate) fn rebuild_tab_menu(menu: &gio::Menu, lang: Language) {
    menu.remove_all();
    menu.append(Some(lang.menu_new_tab()), Some("win.new_tab"));
    menu.append(Some(lang.menu_close_tab()), Some("win.close_tab"));
}
