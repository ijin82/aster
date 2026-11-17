//! Работа с вкладками: поиск активного документа, заворот Ctrl+Tab
//! и вспомогательные функции для позиций страниц.

use adw::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::document::Document;

/// Возвращает каталог файла для начальной папки диалога открытия.
///
/// Для относительного имени без каталога возвращает `None`: иначе GTK
/// попытался бы открыть пустой путь.
pub(crate) fn parent_directory(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// Находит документ, соответствующий выбранной вкладке.
pub(crate) fn active_document(
    tab_view: &adw::TabView,
    docs: &Rc<RefCell<Vec<Rc<Document>>>>,
) -> Option<Rc<Document>> {
    let page = tab_view.selected_page()?;
    docs.borrow()
        .iter()
        .find(|doc| doc.matches_page(&page))
        .cloned()
}

/// Выбирает следующую/предыдущую вкладку, заворачивая список по кругу.
///
/// `adw::TabView::select_next_page()` и `select_previous_page()` не
/// заворачиваются: на последней вкладке они просто возвращают `false`.
/// Поэтому позиция вычисляется вручную, а затем выбирается нужная страница.
pub(crate) fn select_tab_wrapped(tab_view: &adw::TabView, forward: bool) {
    let n_pages = tab_view.n_pages();
    if n_pages <= 0 {
        return;
    }

    let current = tab_view
        .selected_page()
        .map(|page| tab_view.page_position(&page))
        .unwrap_or(-1);

    let Some(target) = wrapped_tab_position(current, n_pages, forward) else {
        return;
    };

    if let Some(page) = tab_page_at_position(tab_view, target as u32) {
        tab_view.set_selected_page(&page);
    }
}

/// Вычисляет позицию следующей вкладки с заворотом.
///
/// Возвращает `None`, если вкладок нет. При некорректной текущей позиции
/// выбирает первую вкладку для движения вперед и последнюю — для движения назад.
pub(crate) fn wrapped_tab_position(current: i32, total: i32, forward: bool) -> Option<i32> {
    if total <= 0 {
        return None;
    }
    if current < 0 || current >= total {
        return Some(if forward { 0 } else { total - 1 });
    }

    if forward {
        Some((current + 1) % total)
    } else {
        Some((current - 1 + total) % total)
    }
}

/// Возвращает страницу вкладки по ее позиции в `adw::TabView`.
pub(crate) fn tab_page_at_position(tab_view: &adw::TabView, position: u32) -> Option<adw::TabPage> {
    let pages = tab_view.pages();
    pages
        .item(position)
        .and_then(|object| object.downcast::<adw::TabPage>().ok())
}
