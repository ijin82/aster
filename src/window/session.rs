//! Сохранение сессии, списка вкладок и геометрии окна.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::document::Document;
use crate::settings::AppSettings;

/// Сохраняет список открытых файлов, активную вкладку, курсоры и позиции
/// прокрутки всех документов в `settings.json`.
pub(crate) fn persist_session(
    settings: &Rc<RefCell<AppSettings>>,
    docs: &Rc<RefCell<Vec<Rc<Document>>>>,
    tab_view: &adw::TabView,
) {
    let snapshot: Vec<Rc<Document>> = docs.borrow().clone();
    let selected = tab_view.selected_page();
    let mut state = settings.borrow_mut();

    for doc in &snapshot {
        doc.export_state(&mut state);
    }

    let mut files = Vec::new();
    let mut active_index = 0usize;
    for doc in &snapshot {
        if let Some(key) = doc.canonical_key() {
            if selected
                .as_ref()
                .map(|page| doc.matches_page(page))
                .unwrap_or(false)
            {
                active_index = files.len();
            }
            files.push(key);
        }
    }

    state.session_files = files;
    state.session_active_document = active_index;
    state.save();
}

/// Сохраняет maximized-состояние и размеры окна.
pub(crate) fn save_window_geometry(
    settings: &Rc<RefCell<AppSettings>>,
    win: &adw::ApplicationWindow,
) {
    let is_max = win.is_maximized();
    {
        let mut state = settings.borrow_mut();
        state.window_maximized = is_max;
        if !is_max {
            state.window_width = win.default_width();
            state.window_height = win.default_height();
        }
    }
    settings.borrow().save();
}
