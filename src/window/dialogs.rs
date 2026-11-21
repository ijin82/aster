//! Диалоги сохранения, подтверждения несохраненных изменений и «О программе».

use adw::prelude::*;
use gtk4::gio;
use std::cell::RefCell;
use std::rc::Rc;

use crate::document::Document;
use crate::i18n::Language;

/// Сохраняет один документ. При необходимости открывает диалог «Сохранить как».
pub(crate) fn save_document(
    parent: &adw::ApplicationWindow,
    doc: &Rc<Document>,
    save_as: bool,
    on_done: Rc<dyn Fn(Result<bool, String>)>,
) {
    let has_path = doc.current_file.borrow().is_some();

    if !save_as && has_path {
        on_done(doc.save_existing().map(|()| true));
        return;
    }

    let lang = Language::from_code(&doc.settings.borrow().language);
    let initial_name = doc
        .current_file
        .borrow()
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("document.md")
        .to_string();

    let dialog = gtk4::FileDialog::builder()
        .title(lang.dialog_save_title())
        .initial_name(initial_name)
        .build();

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some(lang.filter_markdown()));
    filter.add_pattern("*.md");
    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));

    let doc = doc.clone();
    dialog.save(
        Some(parent),
        gio::Cancellable::NONE,
        move |res| {
            let result = match res {
                Ok(file) => match file.path() {
                    Some(path) => doc.save_to(&path).map(|()| true),
                    None => Ok(false),
                },
                // Закрытие диалога пользователем не является ошибкой.
                Err(_) => Ok(false),
            };
            on_done(result);
        },
    );
}

/// Последовательно сохраняет несколько документов (кнопка «Сохранить все»).
///
/// Если пользователь отменяет «Сохранить как» для одного из безымянных
/// документов, цепочка прерывается и окно не закрывается.
pub(crate) fn save_documents_sequentially(
    parent: adw::ApplicationWindow,
    docs: Vec<Rc<Document>>,
    index: usize,
    on_done: Rc<dyn Fn(Result<bool, String>)>,
) {
    if index >= docs.len() {
        on_done(Ok(true));
        return;
    }

    let doc = docs[index].clone();
    let parent_for_dialog = parent.clone();
    let parent_next = parent.clone();
    let docs_next = docs.clone();
    let on_done_next = on_done.clone();

    save_document(
        &parent_for_dialog,
        &doc,
        false,
        Rc::new(move |result| match result {
            Ok(true) => save_documents_sequentially(
                parent_next.clone(),
                docs_next.clone(),
                index + 1,
                on_done_next.clone(),
            ),
            Ok(false) => on_done_next(Ok(false)),
            Err(error) => on_done_next(Err(error)),
        }),
    );
}

/// Показывает диалог о несохраненных изменениях.
///
/// Три обработчика соответствуют ответам «Отмена», «Не сохранять» и «Сохранить».
pub(crate) fn show_unsaved_changes_dialog<F1, F2, F3>(
    parent: &adw::ApplicationWindow,
    heading: &str,
    body: &str,
    lang: Language,
    save_label: &str,
    on_cancel: F1,
    on_discard: F2,
    on_save: F3,
) where
    F1: FnOnce() + 'static,
    F2: FnOnce() + 'static,
    F3: FnOnce() + 'static,
{
    let dialog = adw::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .heading(heading)
        .body(body)
        .build();

    dialog.add_response("cancel", lang.btn_cancel());
    dialog.add_response("discard", lang.btn_discard());
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.add_response("save", save_label);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    let on_cancel = RefCell::new(Some(on_cancel));
    let on_discard = RefCell::new(Some(on_discard));
    let on_save = RefCell::new(Some(on_save));

    dialog.connect_response(None, move |_, response| match response {
        "discard" => {
            if let Some(callback) = on_discard.borrow_mut().take() {
                callback();
            }
        }
        "save" => {
            if let Some(callback) = on_save.borrow_mut().take() {
                callback();
            }
        }
        _ => {
            if let Some(callback) = on_cancel.borrow_mut().take() {
                callback();
            }
        }
    });

    dialog.present();
}

/// Выбор пользователя при конфликте локальных и внешних правок.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileConflictChoice {
    /// Загрузить версию с диска и потерять локальные правки.
    LoadExternal,
    /// Оставить локальные правки.
    KeepEditing,
}

/// Показывает обязательный диалог конфликта файла.
///
/// У диалога нет кнопки отмены: пользователь выбирает между загрузкой
/// внешней версии и продолжением редактирования. Esc и закрытие окна
/// трактуются как безопасный вариант «продолжить редактирование»,
/// поэтому локальные правки не теряются случайно.
pub(crate) fn show_file_conflict_dialog<F>(
    parent: &adw::ApplicationWindow,
    lang: Language,
    file_name: &str,
    on_choice: F,
) where
    F: FnOnce(FileConflictChoice) + 'static,
{
    let dialog = adw::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .heading(lang.file_changed_on_disk())
        .body(lang.file_conflict_body(file_name))
        .build();

    dialog.add_response("keep", lang.btn_keep_edits());
    dialog.add_response("load", lang.btn_load_external());
    dialog.set_response_appearance("load", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("keep"));
    dialog.set_close_response("keep");

    let on_choice = RefCell::new(Some(on_choice));
    dialog.connect_response(None, move |_, response| {
        let choice = if response == "load" {
            FileConflictChoice::LoadExternal
        } else {
            FileConflictChoice::KeepEditing
        };
        if let Some(callback) = on_choice.borrow_mut().take() {
            callback(choice);
        }
    });

    dialog.present();
}

/// Стандартное окно «О программе».
pub(crate) fn show_about_dialog(parent: &adw::ApplicationWindow, lang: Language) {
    let about = adw::AboutWindow::builder()
        .transient_for(parent)
        .modal(true)
        .application_name("Aster")
        .application_icon("io.github.aster")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Developer")
        .comments(lang.about_comments())
        .issue_url("https://github.com")
        .license_type(gtk4::License::MitX11)
        .build();
    about.present();
}
