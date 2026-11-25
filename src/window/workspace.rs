//! Единый контекст окна (`WindowContext`), стартовые вкладки, закрытие и recovery.
//!
//! Введение `WindowContext` уменьшает количество повторяющихся клонов `Rc`
//! и виджетов в замыканиях сигналов GTK, а вынесение `close-flow` и
//! восстановления сессии разгружает `window.rs`.

use adw::prelude::*;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use crate::document::Document;
use crate::i18n::Language;
use crate::recovery::{self, RecoveryDraft};
use crate::settings::AppSettings;

use super::dialogs::{save_document, save_documents_sequentially, show_unsaved_changes_dialog};
use super::session;
use super::tabs::{active_document, parent_directory};

/// Максимальная глубина стека закрытых вкладок для `Ctrl+Shift+T`.
pub(super) const MAX_CLOSED_TABS: usize = 50;

/// Разделяемое состояние главного окна.
///
/// Клонирование `WindowContext` дёшево: внутри только GObject-хэндлы и `Rc`.
#[derive(Clone)]
pub(super) struct WindowContext {
    pub window: adw::ApplicationWindow,
    pub tab_view: adw::TabView,
    pub docs: Rc<RefCell<Vec<Rc<Document>>>>,
    pub settings: Rc<RefCell<AppSettings>>,
    pub toast_overlay: adw::ToastOverlay,
    pub last_open_directory: Rc<RefCell<Option<PathBuf>>>,
    pub closed_tabs: Rc<RefCell<Vec<PathBuf>>>,
    pub recovery_scheduled: Rc<Cell<bool>>,
    pub suppress_mode_sync: Rc<Cell<bool>>,
    pub suppress_search_sync: Rc<Cell<bool>>,
    pub is_force_closing: Rc<Cell<bool>>,
    pub is_closing: Rc<Cell<bool>>,
}

impl WindowContext {
    /// Показывает неблокирующее toast-уведомление поверх окна.
    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(6);
        self.toast_overlay.add_toast(toast);
    }

    /// Сохраняет список открытых файлов, активную вкладку и позиции прокрутки.
    pub fn persist_session(&self) {
        session::persist_session(&self.settings, &self.docs, &self.tab_view);
    }

    /// Сохраняет maximized-состояние и размеры окна.
    pub fn save_window_geometry(&self) {
        session::save_window_geometry(&self.settings, &self.window);
    }

    /// Немедленно сохраняет безымянные черновики в `recovery.json`.
    pub fn save_recovery_now(&self) {
        recovery::save(&collect_recovery_drafts(&self.docs));
    }
}

/// Восстанавливает вкладки при запуске: прошлую сессию, переданные CLI-файлы
/// и несохранённые безымянные черновики после сбоя.
pub(super) fn restore_startup_tabs(
    ctx: &WindowContext,
    add_document: &Rc<dyn Fn(Option<PathBuf>, bool) -> Option<adw::TabPage>>,
    initial_files: Vec<PathBuf>,
    restore_session: bool,
) {
    let recovery_drafts = recovery::load();
    let mut opened_pages: Vec<adw::TabPage> = Vec::new();
    let mut selected_page: Option<adw::TabPage> = None;

    if restore_session {
        let (session_files, active_index) = {
            let state = ctx.settings.borrow();
            (state.session_files.clone(), state.session_active_document)
        };

        for (index, path) in session_files.iter().enumerate() {
            let path = PathBuf::from(path);
            if !path.is_file() {
                continue;
            }
            if let Some(page) = add_document(Some(path), true) {
                if index == active_index {
                    selected_page = Some(page.clone());
                }
                opened_pages.push(page);
            }
        }
    }

    if opened_pages.is_empty() {
        for path in initial_files {
            if !path.is_file() {
                continue;
            }
            if let Some(page) = add_document(Some(path), false) {
                opened_pages.push(page);
            }
        }
    }

    let mut recovered_count = 0usize;
    for draft in recovery_drafts {
        if let Some(page) = add_document(None, false) {
            if let Some(doc) = ctx
                .docs
                .borrow()
                .iter()
                .find(|doc| doc.matches_page(&page))
                .cloned()
            {
                doc.restore_draft(&draft.content, draft.cursor_line, draft.cursor_column);
                recovered_count += 1;
            }
            opened_pages.push(page.clone());
            selected_page = Some(page);
        }
    }
    if recovered_count > 0 {
        let lang = Language::from_code(&ctx.settings.borrow().language);
        ctx.show_toast(&lang.drafts_recovered(recovered_count));
    }

    if opened_pages.is_empty() {
        if let Some(page) = add_document(None, false) {
            opened_pages.push(page);
        }
    }

    // Активной делаем вкладку из сессии, иначе первую открытую.
    let active_page = selected_page.or_else(|| opened_pages.first().cloned());
    if let Some(page) = active_page {
        ctx.tab_view.set_selected_page(&page);
    }
    if let Some(doc) = active_document(&ctx.tab_view, &ctx.docs) {
        if let Some(directory) = doc
            .current_file
            .borrow()
            .as_ref()
            .and_then(|path| parent_directory(path))
        {
            *ctx.last_open_directory.borrow_mut() = Some(directory);
        }
    }
}

/// Подключает сигналы переключения/закрытия вкладок и закрытия окна.
pub(super) fn connect_tab_and_close_handlers(
    ctx: &WindowContext,
    refresh_active_ui: &Rc<dyn Fn()>,
) {
    // 1. Смена активной вкладки.
    {
        let ctx_cb = ctx.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        ctx.tab_view.connect_selected_page_notify(move |_| {
            if let Some(doc) = active_document(&ctx_cb.tab_view, &ctx_cb.docs) {
                if let Some(directory) = doc
                    .current_file
                    .borrow()
                    .as_ref()
                    .and_then(|path| parent_directory(path))
                {
                    *ctx_cb.last_open_directory.borrow_mut() = Some(directory);
                }
            }
            ctx_cb.persist_session();
            refresh_active_ui();
        });
    }

    // 2. Запрос закрытия вкладки (подтверждение при несохранённых правках).
    {
        let ctx_cb = ctx.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        ctx.tab_view.connect_close_page(move |view, page| {
            let doc = ctx_cb
                .docs
                .borrow()
                .iter()
                .find(|doc| doc.matches_page(page))
                .cloned();

            let Some(doc) = doc else {
                return glib::Propagation::Proceed;
            };

            // Чистая вкладка закрывается стандартным обработчиком TabView.
            if !doc.is_modified.get() {
                return glib::Propagation::Proceed;
            }

            let lang = Language::from_code(&ctx_cb.settings.borrow().language);
            let view_for_cancel = view.clone();
            let page_for_cancel = page.clone();
            let view_for_discard = view.clone();
            let page_for_discard = page.clone();
            let view_for_save = view.clone();
            let page_for_save = page.clone();
            let doc_for_discard = doc.clone();
            let doc_for_save = doc.clone();
            let ctx_for_discard = ctx_cb.clone();
            let ctx_for_save = ctx_cb.clone();
            let refresh_for_discard = refresh_active_ui.clone();
            let refresh_for_save = refresh_active_ui.clone();

            show_unsaved_changes_dialog(
                &ctx_cb.window,
                lang.tab_close_heading(),
                lang.tab_close_body(),
                lang,
                lang.btn_save(),
                move || {
                    view_for_cancel.close_page_finish(&page_for_cancel, false);
                },
                move || {
                    doc_for_discard.export_state(&mut ctx_for_discard.settings.borrow_mut());
                    view_for_discard.close_page_finish(&page_for_discard, true);
                    ctx_for_discard.persist_session();
                    refresh_for_discard();
                },
                move || {
                    let ctx_done = ctx_for_save.clone();
                    save_document(
                        &ctx_for_save.window,
                        &doc_for_save,
                        false,
                        Rc::new(move |result| match result {
                            Ok(true) => {
                                ctx_done.save_recovery_now();
                                view_for_save.close_page_finish(&page_for_save, true);
                                ctx_done.persist_session();
                                refresh_for_save();
                            }
                            Ok(false) => {
                                view_for_save.close_page_finish(&page_for_save, false);
                            }
                            Err(error) => {
                                view_for_save.close_page_finish(&page_for_save, false);
                                let lang =
                                    Language::from_code(&ctx_done.settings.borrow().language);
                                ctx_done
                                    .show_toast(&format!("{}: {error}", lang.error_save_file()));
                            }
                        }),
                    );
                },
            );

            glib::Propagation::Stop
        });
    }

    // 3. Отсоединение вкладки: стек закрытых файлов и закрытие окна на последней.
    {
        let ctx_cb = ctx.clone();
        let refresh_active_ui = refresh_active_ui.clone();
        ctx.tab_view.connect_page_detached(move |_, page, _| {
            // При закрытии окна вкладки отсоединяются автоматически; сессию
            // уже сохранил обработчик close_request, поэтому не затираем ее.
            if ctx_cb.is_closing.get() {
                return;
            }

            let removed = {
                let mut list = ctx_cb.docs.borrow_mut();
                list.iter()
                    .position(|doc| doc.matches_page(page))
                    .map(|index| list.remove(index))
            };

            if let Some(doc) = removed {
                if let Some(path) = doc.current_file.borrow().clone() {
                    let mut stack = ctx_cb.closed_tabs.borrow_mut();
                    if stack.len() >= MAX_CLOSED_TABS {
                        stack.remove(0);
                    }
                    stack.push(path);
                }
                doc.export_state(&mut ctx_cb.settings.borrow_mut());
            }

            ctx_cb.save_recovery_now();
            ctx_cb.persist_session();
            refresh_active_ui();

            // Закрытие последней вкладки закрывает окно (как в GNOME Text Editor).
            if ctx_cb.docs.borrow().is_empty() {
                ctx_cb.is_closing.set(true);
                ctx_cb.persist_session();
                ctx_cb.window.close();
            }
        });
    }

    // 4. Закрытие окна с проверкой всех изменённых вкладок.
    {
        let ctx_cb = ctx.clone();
        ctx.window.connect_close_request(move |win| {
            if ctx_cb.is_force_closing.get() {
                return glib::Propagation::Proceed;
            }

            let modified: Vec<Rc<Document>> = ctx_cb
                .docs
                .borrow()
                .iter()
                .filter(|doc| doc.is_modified.get())
                .cloned()
                .collect();

            if modified.is_empty() {
                ctx_cb.is_closing.set(true);
                ctx_cb.persist_session();
                ctx_cb.save_window_geometry();
                recovery::clear();
                return glib::Propagation::Proceed;
            }

            let lang = Language::from_code(&ctx_cb.settings.borrow().language);
            let ctx_discard = ctx_cb.clone();
            let ctx_save = ctx_cb.clone();

            show_unsaved_changes_dialog(
                win,
                lang.unsaved_window_heading(),
                lang.unsaved_window_body(),
                lang,
                lang.btn_save_all(),
                || {},
                move || {
                    ctx_discard.is_closing.set(true);
                    ctx_discard.persist_session();
                    ctx_discard.save_window_geometry();
                    recovery::clear();
                    ctx_discard.is_force_closing.set(true);
                    ctx_discard.window.close();
                },
                move || {
                    let ctx_done = ctx_save.clone();
                    save_documents_sequentially(
                        ctx_save.window.clone(),
                        modified.clone(),
                        0,
                        Rc::new(move |result| match result {
                            Ok(true) => {
                                ctx_done.is_closing.set(true);
                                ctx_done.persist_session();
                                ctx_done.save_window_geometry();
                                recovery::clear();
                                ctx_done.is_force_closing.set(true);
                                ctx_done.window.close();
                            }
                            Ok(false) => {}
                            Err(error) => {
                                let lang =
                                    Language::from_code(&ctx_done.settings.borrow().language);
                                ctx_done
                                    .show_toast(&format!("{}: {error}", lang.error_save_file()));
                            }
                        }),
                    );
                },
            );

            glib::Propagation::Stop
        });
    }
}

/// Собирает черновики всех безымянных документов.
pub(super) fn collect_recovery_drafts(
    docs: &Rc<RefCell<Vec<Rc<Document>>>>,
) -> Vec<RecoveryDraft> {
    docs.borrow()
        .iter()
        .filter(|doc| doc.current_file.borrow().is_none())
        .filter_map(|doc| {
            let text = doc
                .buffer
                .text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false);
            // Пустые безымянные вкладки не представляют ценности.
            if text.is_empty() {
                return None;
            }

            let insert_mark = doc.buffer.get_insert();
            let iter = doc.buffer.iter_at_mark(&insert_mark);
            Some(RecoveryDraft {
                content: text.to_string(),
                cursor_line: iter.line().max(0) as usize,
                cursor_column: iter.line_offset().max(0) as usize,
            })
        })
        .collect()
}

/// Планирует отложенную запись `recovery.json`.
///
/// `Weak` на список документов не создает цикл: буферный обработчик
/// документа не удерживает окно и все вкладки.
pub(super) fn schedule_recovery_save(
    docs: Weak<RefCell<Vec<Rc<Document>>>>,
    scheduled: Rc<Cell<bool>>,
) {
    if scheduled.replace(true) {
        return;
    }

    glib::timeout_add_seconds_local_once(1, move || {
        scheduled.set(false);
        if let Some(docs) = docs.upgrade() {
            recovery::save(&collect_recovery_drafts(&docs));
        }
    });
}
