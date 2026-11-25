//! Память прокрутки редактора и отложенное восстановление позиций.
//!
//! При переключении на предпросмотр `GtkScrolledWindow` редактора скрывается,
//! и его `GtkAdjustment` может сброситься в 0. Чтобы пользовательская позиция
//! не терялась, мы запоминаем её только при видимом редакторе и подавляем
//! запись промежуточных значений во время восстановления.

use gtk4::glib;
use gtk4::prelude::*;
use std::rc::Rc;

use super::Document;

impl Document {
    /// Подключает отслеживание вертикальной прокрутки редактора.
    pub(super) fn setup_editor_scroll_tracking(self: &Rc<Self>) {
        let weak_doc = Rc::downgrade(self);
        let suppress_editor_scroll_updates = self.suppress_editor_scroll_updates.clone();
        self.editor_scroll
            .vadjustment()
            .connect_value_changed(move |adj| {
                if suppress_editor_scroll_updates.get() {
                    return;
                }
                if let Some(doc) = weak_doc.upgrade() {
                    if !doc.is_preview_active.get() {
                        doc.editor_scroll_position.set(adj.value());
                    }
                }
            });
    }

    /// Прокручивает редактор к сохраненной позиции после перекомпоновки виджетов.
    pub(super) fn schedule_editor_scroll(&self) {
        self.schedule_editor_scroll_to(self.editor_scroll_position.get());
    }

    /// Прокручивает редактор к явно переданной позиции в следующем idle-цикле.
    ///
    /// Значение передается аргументом, а не читается из ячейки позже: за время
    /// до idle-цикла GTK может успеть пересчитать adjustment скрытого/только что
    /// показанного ScrolledWindow.
    pub(super) fn schedule_editor_scroll_to(&self, target: f64) {
        let adjustment = self.editor_scroll.vadjustment();
        let suppress_editor_scroll_updates = self.suppress_editor_scroll_updates.clone();
        self.suppress_editor_scroll_updates.set(true);

        glib::idle_add_local_once(move || {
            let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
            adjustment.set_value(target.clamp(0.0, max));
            suppress_editor_scroll_updates.set(false);
        });
    }
}
