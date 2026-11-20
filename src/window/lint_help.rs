//! Окно встроенной справки по правилам проверки стиля Markdown.
//!
//! Справка состоит из двух уровней:
//! - каталог всех правил с поиском (корневая страница `adw::NavigationView`);
//! - страница конкретного правила с объяснением и примерами.
//!
//! Окно открывается из контекстного меню диагностики сразу на нужном
//! правиле, по `F1` — на каталоге, а также из окна настроек. Содержимое
//! не загружается из сети: тексты поставляются вместе с приложением.

use adw::prelude::*;
use std::rc::Rc;

use crate::document::{Rule, Severity};
use crate::external::open_uri;
use crate::i18n::Language;
use crate::lint_rules::{docs_url, matches_query, rule_help, severity_label};

/// Открывает окно справки.
///
/// `focus` задаёт правило, которое нужно показать сразу. Если он равен
/// `None`, открывается каталог всех правил.
pub(crate) fn show_lint_help_window(
    parent: &impl IsA<gtk4::Window>,
    lang: Language,
    focus: Option<Rule>,
) {
    build_lint_help_window(parent, lang, focus).present();
}

/// Создаёт окно справки, но не показывает его.
///
/// Отделено от [`show_lint_help_window`], чтобы окно можно было собрать
/// в тесте без запуска главного цикла GTK и без показа пользователю.
fn build_lint_help_window(
    parent: &impl IsA<gtk4::Window>,
    lang: Language,
    focus: Option<Rule>,
) -> adw::Window {
    let window = adw::Window::builder()
        .transient_for(parent)
        .modal(false)
        .title(lang.lint_help_window_title())
        .default_width(640)
        .default_height(640)
        .build();

    // Справка не должна переживать главное окно: иначе после закрытия
    // Aster останется висящее окно с правилами.
    window.set_destroy_with_parent(true);

    let navigation = adw::NavigationView::new();
    window.set_content(Some(&navigation));

    let catalog = build_catalog_page(&navigation, lang);
    navigation.push(&catalog);

    if let Some(rule) = focus {
        let detail = build_detail_page(lang, rule);
        navigation.push(&detail);
    }

    window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gtk4::test]
    fn test_help_window_builds_catalog_and_detail() {
        let parent = adw::Window::new();

        // Проверяем оба сценария: каталог по F1 и сразу открытое правило
        // из контекстного меню. Ошибка построения страниц проявилась бы
        // паникой или пустым содержимым окна.
        let catalog = build_lint_help_window(&parent, Language::Russian, None);
        assert!(catalog.content().is_some());

        let detail = build_lint_help_window(&parent, Language::English, Some(Rule::Md022));
        assert!(detail.content().is_some());
    }
}

/// Создаёт корневую страницу с поиском и списком всех правил.
fn build_catalog_page(navigation: &adw::NavigationView, lang: Language) -> adw::NavigationPage {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let search = gtk4::SearchEntry::builder()
        .placeholder_text(lang.lint_help_search_placeholder())
        .margin_top(12)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&search);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .build();

    let list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    list.add_css_class("boxed-list");
    scrolled.set_child(Some(&list));
    content.append(&scrolled);

    // Перестроение списка при каждом изменении запроса: правил всего
    // девять, поэтому проще пересобрать строки, чем поддерживать
    // отдельную модель фильтрации и синхронизировать её с виджетами.
    let rebuild: Rc<dyn Fn(&str)> = {
        let list = list.clone();
        let navigation = navigation.clone();

        Rc::new(move |query: &str| {
            // В gtk4-rs у ListBox нет remove_all, поэтому снимаем строки
            // по одной. Правил немного, и пересборка не заметна.
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }

            let mut found = false;
            for rule in Rule::ALL {
                if !matches_query(rule, lang, query) {
                    continue;
                }
                found = true;

                let row = adw::ActionRow::builder()
                    .title(format!("{} · {}", rule.id(), lang.lint_message(rule.id())))
                    .subtitle(severity_label(rule.severity(), lang))
                    .activatable(true)
                    .build();
                row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));

                let navigation_for_row = navigation.clone();
                row.connect_activated(move |_| {
                    let detail = build_detail_page(lang, rule);
                    navigation_for_row.push(&detail);
                });

                list.append(&row);
            }

            if !found {
                let empty = gtk4::Label::new(Some(lang.lint_help_empty()));
                empty.add_css_class("dim-label");
                empty.set_margin_top(24);
                empty.set_margin_bottom(24);
                empty.set_wrap(true);

                let row = gtk4::ListBoxRow::new();
                row.set_activatable(false);
                row.set_selectable(false);
                row.set_child(Some(&empty));
                list.append(&row);
            }
        })
    };

    rebuild("");

    let rebuild_for_search = rebuild.clone();
    search.connect_search_changed(move |entry| {
        rebuild_for_search(entry.text().as_str());
    });

    build_page_shell(lang.lint_help_all_rules(), &content)
}

/// Создаёт страницу одного правила с объяснением и примерами.
fn build_detail_page(lang: Language, rule: Rule) -> adw::NavigationPage {
    let help = rule_help(rule, lang);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    content.set_margin_top(18);
    content.set_margin_bottom(24);
    content.set_margin_start(18);
    content.set_margin_end(18);

    // Номер правила и важность — отдельной строкой над заголовком.
    let meta = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let id_label = gtk4::Label::new(Some(rule.id()));
    id_label.add_css_class("dim-label");
    meta.append(&id_label);

    let severity = gtk4::Label::new(Some(severity_label(rule.severity(), lang)));
    severity.add_css_class("caption");
    severity.add_css_class(match rule.severity() {
        Severity::Warning => "warning",
        Severity::Error => "error",
    });
    meta.append(&severity);
    content.append(&meta);

    let title = gtk4::Label::new(Some(lang.lint_message(rule.id())));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    title.set_wrap(true);
    content.append(&title);

    content.append(&section_label(lang.lint_help_why()));
    content.append(&body_label(help.why));

    content.append(&section_label(lang.lint_help_bad()));
    content.append(&example_box(help.bad));

    content.append(&section_label(lang.lint_help_good()));
    content.append(&example_box(help.good));

    // Поясняем обозначения невидимых символов, которые используются
    // в примерах MD009, MD010 и MD047.
    let legend = gtk4::Label::new(Some(lang.lint_help_legend()));
    legend.add_css_class("caption");
    legend.add_css_class("dim-label");
    legend.set_xalign(0.0);
    legend.set_wrap(true);
    content.append(&legend);

    let docs_button = gtk4::Button::with_label(lang.lint_open_docs());
    docs_button.set_halign(gtk4::Align::Start);
    let uri = docs_url(rule);
    docs_button.connect_clicked(move |_| open_uri(uri));
    content.append(&docs_button);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    scrolled.set_child(Some(&content));

    build_page_shell(rule.id(), &scrolled)
}

/// Собирает страницу `adw::NavigationView` с заголовком.
///
/// Кнопку «Назад» не добавляем вручную: `AdwNavigationView` сам показывает
/// её в `AdwHeaderBar` для любой страницы, кроме корневой, и связывает с
/// `pop()`. Это заодно даёт штатную обработку Esc и жестов.
fn build_page_shell(title: &str, content: &impl IsA<gtk4::Widget>) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(title, "")));

    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(content));

    adw::NavigationPage::new(&toolbar, title)
}

/// Заголовок раздела справки.
fn section_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("heading");
    label.set_xalign(0.0);
    label.set_margin_top(6);
    label
}

/// Обычный абзац пояснения.
fn body_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    label
}

/// Блок с примером разметки.
///
/// Используется карточка Adwaita и моноширинный шрифт: так пример
/// визуально отличается от пояснительного текста и его можно выделить.
fn example_box(text: &str) -> gtk4::Box {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.add_css_class("card");

    let label = gtk4::Label::new(Some(text));
    label.add_css_class("monospace");
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    label.set_margin_top(10);
    label.set_margin_bottom(10);
    label.set_margin_start(12);
    label.set_margin_end(12);

    container.append(&label);
    container
}
