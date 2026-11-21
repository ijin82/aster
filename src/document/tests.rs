//! Тесты документа, поиска, редактора и политики ссылок.

use super::diagnostics::{
    apply_to_buffer, diagnostic_at, register_tags, validate, Rule, Severity,
};
use super::editor::{
    apply_restored_cursor_position, duplicate_current_line, indent_selection_or_cursor,
    setup_editor_key_handling, unindent_selection_or_cursor,
};
use super::gutter::setup_line_numbers_gutter;
use super::links::{classify_link_uri, LinkTarget};
use super::search::{
    find_matches, next_match_index_after, parse_preview_search_result, replace_all_matches,
    wrapped_search_index,
};
use super::*;

    #[gtk4::test]
    fn test_indent_and_unindent() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("line 1\nline 2\nline 3\n");

        let start = buffer.iter_at_line(0).unwrap();
        let end = buffer.iter_at_line_offset(1, 3).unwrap();
        buffer.select_range(&start, &end);

        indent_selection_or_cursor(&buffer, 2);
        let full_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(full_text.as_str(), "  line 1\n  line 2\nline 3\n");

        unindent_selection_or_cursor(&buffer, 2);
        let text_after_unindent = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text_after_unindent.as_str(), "line 1\nline 2\nline 3\n");

        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("line\n");
        buffer.place_cursor(&buffer.start_iter());
        indent_selection_or_cursor(&buffer, 4);
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            "    line\n"
        );
        unindent_selection_or_cursor(&buffer, 4);
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            "line\n"
        );
    }

    #[gtk4::test]
    fn test_find_matches_is_case_insensitive() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("Foo foo food\nbar\n");

        let matches = find_matches(&buffer, "foo");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].0, 0);
        assert_eq!(matches[1].0, 4);
        assert_eq!(matches[2].0, 8);

        assert!(find_matches(&buffer, "xyz").is_empty());
        assert!(find_matches(&buffer, "").is_empty());

        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("Привет привет ПРИВЕТ");
        let matches = find_matches(&buffer, "привет");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0], (0, 6));
        assert_eq!(matches[1], (7, 13));
    }

    #[test]
    fn test_wrapped_search_index() {
        assert_eq!(wrapped_search_index(0, 3, true), Some(1));
        assert_eq!(wrapped_search_index(2, 3, true), Some(0));
        assert_eq!(wrapped_search_index(0, 3, false), Some(2));
        assert_eq!(wrapped_search_index(2, 3, false), Some(1));
        assert_eq!(wrapped_search_index(0, 0, true), None);
    }

    #[test]
    fn test_classify_link_uri() {
        assert_eq!(
            classify_link_uri("#section-1", None),
            LinkTarget::Anchor("section-1".to_string())
        );
        assert_eq!(
            classify_link_uri("#", None),
            LinkTarget::Anchor(String::new())
        );
        assert_eq!(
            classify_link_uri("https://example.com", None),
            LinkTarget::External("https://example.com".to_string())
        );
        assert_eq!(
            classify_link_uri("MAILTO:test@example.com", None),
            LinkTarget::External("MAILTO:test@example.com".to_string())
        );
        assert_eq!(
            classify_link_uri("file:///tmp/other.md", None),
            LinkTarget::External("file:///tmp/other.md".to_string())
        );
        assert_eq!(
            classify_link_uri("javascript:alert(1)", None),
            LinkTarget::Blocked
        );
        assert_eq!(
            classify_link_uri("data:text/html,hi", None),
            LinkTarget::Blocked
        );
        assert_eq!(
            classify_link_uri("relative/file.md", None),
            LinkTarget::Blocked
        );

        // Регрессия: base href="file:///tmp/" превращает #anchor
        // в file:///tmp/#anchor. Это внутренний якорь, а не ссылка на /tmp.
        let current = std::path::Path::new("/tmp/aster-links.md");
        assert_eq!(
            classify_link_uri("file:///tmp/#target", Some(current)),
            LinkTarget::Anchor("target".to_string())
        );
        assert_eq!(
            classify_link_uri("file:///tmp/aster-links.md#target", Some(current)),
            LinkTarget::Anchor("target".to_string())
        );
        assert_eq!(
            classify_link_uri("https://example.com#target", Some(current)),
            LinkTarget::External("https://example.com#target".to_string())
        );
    }

    #[gtk4::test]
    fn test_gutter_does_not_keep_text_view_alive() {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let text_view = gtk4::TextView::new();
        let weak_text_view = text_view.downgrade();

        // Держим gutter отдельно: если draw-функция захватит TextView
        // сильной ссылкой, цикл не даст TextView освободиться.
        let diagnostics = Rc::new(RefCell::new(Vec::new()));
        let _gutter = setup_line_numbers_gutter(&text_view, &settings, &diagnostics);

        drop(text_view);
        assert!(
            weak_text_view.upgrade().is_none(),
            "gutter draw func keeps TextView alive"
        );
    }

    #[gtk4::test]
    fn test_key_controller_does_not_keep_text_view_alive() {
        let text_view = gtk4::TextView::new();
        let weak_text_view = text_view.downgrade();

        let editor_scroll = gtk4::ScrolledWindow::new();
        editor_scroll.set_child(Some(&text_view));
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        setup_editor_key_handling(&text_view, &editor_scroll, settings);

        drop(text_view);
        // ScrolledWindow владеет TextView, поэтому сначала освобождаем
        // контейнер, иначе weak-проверка сработает на дочерний виджет.
        drop(editor_scroll);
        assert!(
            weak_text_view.upgrade().is_none(),
            "key controller keeps TextView alive"
        );
    }

    #[gtk4::test]
    fn test_editor_options_are_applied() {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let doc = Document::new(settings.clone(), None, false).expect("create document");

        assert!(doc.show_whitespace.get());
        assert!(doc.text_view.tabs().is_some());

        settings.borrow_mut().soft_wrap = false;
        settings.borrow_mut().show_whitespace = false;
        doc.apply_editor_options();

        assert_eq!(doc.text_view.wrap_mode(), gtk4::WrapMode::None);
        assert!(!doc.show_whitespace.get());

        settings.borrow_mut().soft_wrap = true;
        doc.apply_editor_options();
        assert_eq!(doc.text_view.wrap_mode(), gtk4::WrapMode::Word);
    }

    #[gtk4::test]
    fn test_restore_draft_sets_content_and_modified_state() {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let doc = Document::new(settings, None, false).expect("create document");

        doc.restore_draft("first\nsecond line", 1, 3);

        let text = doc
            .buffer
            .text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false);
        assert_eq!(text.as_str(), "first\nsecond line");
        assert!(doc.is_modified.get());

        let cursor = doc.buffer.iter_at_mark(&doc.buffer.get_insert());
        assert_eq!(cursor.line(), 1);
        assert_eq!(cursor.line_offset(), 3);
    }

    #[gtk4::test]
    fn test_document_is_freed_after_drop() {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let doc = Document::new(settings, None, false).expect("create document");
        let weak_doc = Rc::downgrade(&doc);
        let weak_web_view = doc.web_view.downgrade();
        let weak_text_view = doc.text_view.downgrade();

        drop(doc);
        assert!(
            weak_doc.upgrade().is_none(),
            "Document is kept alive by internal signal handlers"
        );
        assert!(
            weak_web_view.upgrade().is_none(),
            "WebView is kept alive by its own load handler"
        );
        assert!(
            weak_text_view.upgrade().is_none(),
            "TextView is kept alive by editor signal handlers"
        );
    }

    #[gtk4::test]
    fn test_preview_search_script_finds_visible_text() {
        use std::time::{Duration, Instant};

        let manager = UserContentManager::new();
        manager.add_script(&UserScript::new(
            SCROLL_TRACKER_SCRIPT,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::End,
            &[],
            &[],
        ));

        let web_view = webkit6::WebView::builder()
            .user_content_manager(&manager)
            .build();

        let loaded = Rc::new(Cell::new(false));
        let loaded_for_signal = loaded.clone();
        web_view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                loaded_for_signal.set(true);
            }
        });

        web_view.load_html(
            "<!DOCTYPE html><html><body>             <p>Hello world</p><p>hello again</p>             <script>Hello hidden</script>             </body></html>",
            None,
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while !loaded.get() && Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(loaded.get(), "preview did not load");

        let evaluate = |script: &str| -> String {
            let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
            let result_for_callback = result.clone();
            web_view.evaluate_javascript(
                script,
                None,
                None,
                Option::<&gio::Cancellable>::None,
                move |value| {
                    if let Ok(value) = value {
                        *result_for_callback.borrow_mut() = Some(value.to_str().to_string());
                    }
                },
            );

            let deadline = Instant::now() + Duration::from_secs(5);
            while result.borrow().is_none() && Instant::now() < deadline {
                while glib::MainContext::default().iteration(false) {}
                std::thread::sleep(Duration::from_millis(10));
            }

            let output = result.borrow().clone().expect("JS result");
            output
        };

        let first = evaluate("window.__asterSearch.find('hello')");
        assert_eq!(parse_preview_search_result(&first), Some((2, 0)));

        let second = evaluate("window.__asterSearch.next(true)");
        assert_eq!(parse_preview_search_result(&second), Some((2, 1)));

        let mark_count = evaluate(
            "document.querySelectorAll('mark.aster-search-match, mark.aster-search-current').length",
        );
        assert_eq!(mark_count, "2");
    }

    #[test]
    fn test_parse_preview_search_result() {
        assert_eq!(parse_preview_search_result(r#"{"count":3,"index":1}"#), Some((3, 1)));
        assert_eq!(parse_preview_search_result(r#"{"count":0,"index":-1}"#), Some((0, 0)));
        assert_eq!(parse_preview_search_result(r#"{"count":2,"index":-1}"#), Some((2, 0)));
        assert_eq!(parse_preview_search_result("not json"), None);
    }

    #[test]
    fn test_next_match_index_after_replacement() {
        let matches = [(0, 3), (10, 13), (20, 23)];
        assert_eq!(next_match_index_after(&matches, 0), 0);
        assert_eq!(next_match_index_after(&matches, 4), 1);
        assert_eq!(next_match_index_after(&matches, 11), 2);
        assert_eq!(next_match_index_after(&matches, 21), 0);
        assert_eq!(next_match_index_after(&[], 0), 0);
    }

    #[gtk4::test]
    fn test_replace_all_matches() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("Foo foo food\nbar\n");

        buffer.set_enable_undo(true);
        let original = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let count = replace_all_matches(&buffer, "foo", "baz");
        assert_eq!(count, 3);
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text.as_str(), "baz baz bazd\nbar\n");

        assert!(buffer.can_undo());
        buffer.undo();
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            original.as_str()
        );

        // Пустая строка замены удаляет совпадения, но не ломает текст.
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("a-a-a");
        assert_eq!(replace_all_matches(&buffer, "a", ""), 3);
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .as_str(),
            "--"
        );

        assert_eq!(replace_all_matches(&buffer, "", "x"), 0);
    }

    #[gtk4::test]
    fn test_duplicate_current_line() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("first\nsecond\nthird\n");

        let cursor = buffer.iter_at_line_offset(1, 3).expect("line offset");
        buffer.place_cursor(&cursor);
        duplicate_current_line(&buffer);

        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text.as_str(), "first\nsecond\nsecond\nthird\n");

        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        assert_eq!(cursor.line(), 2);
        assert_eq!(cursor.line_offset(), 3);
    }

    #[gtk4::test]
    fn test_duplicate_last_line_without_trailing_newline() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("one\ntwo");

        let cursor = buffer.iter_at_line_offset(1, 1).expect("line offset");
        buffer.place_cursor(&cursor);
        duplicate_current_line(&buffer);

        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text.as_str(), "one\ntwo\ntwo");

        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        assert_eq!(cursor.line(), 2);
        assert_eq!(cursor.line_offset(), 1);
    }

    #[gtk4::test]
    fn test_cursor_position_restore() {
        let buffer = gtk4::TextBuffer::new(None);
        buffer.set_text("First line\nSecond line\nThird line\n");

        let test_path = std::path::PathBuf::from("/test/sample_doc.md");
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let key = get_file_canonical_key(&test_path);

        let target_iter = buffer.iter_at_line_offset(1, 5).unwrap();
        buffer.place_cursor(&target_iter);

        let insert_mark = buffer.get_insert();
        let iter = buffer.iter_at_mark(&insert_mark);
        settings
            .borrow_mut()
            .file_cursor_positions
            .insert(key, (iter.line() as usize, iter.line_offset() as usize));

        buffer.place_cursor(&buffer.start_iter());
        apply_restored_cursor_position(&settings, &buffer, &test_path);

        let restored_iter = buffer.iter_at_mark(&buffer.get_insert());
        assert_eq!(restored_iter.line(), 1);
        assert_eq!(restored_iter.line_offset(), 5);
    }

    #[gtk4::test]
    fn test_preview_render_cache_reuses_unchanged_html() {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let doc = Document::new(settings, None, false).expect("create document");
        doc.buffer.set_text("# Title\n\nBody");

        // Первый вызов загружает HTML в WebView, повторный с тем же
        // содержимым должен переиспользовать уже отрисованную страницу.
        assert!(doc.refresh_preview());
        assert!(!doc.refresh_preview());

        // Изменение Markdown делает кэш недействительным.
        doc.buffer.set_text("# Title\n\nChanged body");
        assert!(doc.refresh_preview());
    }

// --- Проверка стиля Markdown ---
//
// Правила проверяются на чистом тексте, без GTK: `validate` не зависит от
// виджетов, поэтому тесты быстрые и не требуют графической сессии.

#[test]
fn test_lint_clean_document_has_no_diagnostics() {
    let text = "# Title\n\nText with a hard break  \n\n## Section\n\n```rust\nfn main() {}\n```\n\n- item\n- item\n\nEnd.\n";
    assert!(
        validate(text).is_empty(),
        "clean document produced diagnostics: {:?}",
        validate(text)
    );
}

#[test]
fn test_lint_md009_allows_two_trailing_spaces() {
    let diagnostics = validate("Text \nText  \nText   \n");
    let md009: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == Rule::Md009)
        .collect();

    assert_eq!(md009.len(), 2, "only one and three spaces are violations");
    assert_eq!(md009[0].line, 0);
    assert_eq!(md009[1].line, 2);
    assert_eq!(md009[1].start_col, 4);
    assert_eq!(md009[1].end_col, 7);
}

#[test]
fn test_lint_columns_are_characters_for_cyrillic() {
    let diagnostics = validate("Привет   \n");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];

    // «Привет» — 6 символов, хотя в UTF-8 это 12 байт.
    assert_eq!(diagnostic.rule, Rule::Md009);
    assert_eq!(diagnostic.start_col, 6);
    assert_eq!(diagnostic.end_col, 9);
}

#[test]
fn test_lint_md010_ignores_tabs_in_code_blocks() {
    let diagnostics = validate("a\tb\n\n```rust\n\tcode\n```\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule, Rule::Md010);
    assert_eq!(diagnostics[0].line, 0);
    assert_eq!(diagnostics[0].start_col, 1);
}

#[test]
fn test_lint_md012_multiple_blank_lines() {
    let diagnostics = validate("a\n\n\nb\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule, Rule::Md012);
    assert_eq!(diagnostics[0].line, 2);
    assert_eq!(diagnostics[0].start_col, diagnostics[0].end_col);
}

#[test]
fn test_lint_md022_headings_need_blank_lines() {
    let diagnostics = validate("# Title\nText\n## Section\n");
    let md022: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == Rule::Md022)
        .collect();

    assert_eq!(md022.len(), 2);
    assert_eq!(md022[0].line, 0);
    assert_eq!(md022[1].line, 2);
}

#[test]
fn test_lint_md025_reports_second_h1_as_error() {
    let diagnostics = validate("# One\n\n# Two\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule, Rule::Md025);
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert_eq!(diagnostics[0].line, 2);
}

#[test]
fn test_lint_md031_fences_need_blank_lines() {
    let diagnostics = validate("Text\n```rust\ncode\n```\nText\n");
    let md031: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == Rule::Md031)
        .collect();

    assert_eq!(md031.len(), 2);
    assert_eq!(md031[0].line, 1);
    assert_eq!(md031[1].line, 3);
}

#[test]
fn test_lint_md040_fence_without_language() {
    let diagnostics = validate("```\ncode\n```\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule, Rule::Md040);
    assert_eq!(diagnostics[0].line, 0);
}

#[test]
fn test_lint_md032_lists_need_blank_lines() {
    let diagnostics = validate("Text\n- item\n- item\nText\n");
    let md032: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == Rule::Md032)
        .collect();

    assert_eq!(md032.len(), 2);
    assert_eq!(md032[0].line, 1);
    assert_eq!(md032[1].line, 2);
}

#[test]
fn test_lint_md047_requires_single_final_newline() {
    let missing = validate("Text");
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].rule, Rule::Md047);
    assert_eq!(missing[0].line, 0);

    // Два перевода строки — это и лишняя пустая строка, и неверное
    // завершение файла; оба правила честно срабатывают.
    let extra = validate("Text\n\n");
    assert!(extra.iter().any(|diagnostic| diagnostic.rule == Rule::Md047));
    assert!(extra.iter().any(|diagnostic| diagnostic.rule == Rule::Md012));
}

#[test]
fn test_lint_ignores_markdown_inside_code_blocks() {
    let diagnostics = validate("```md\n# Not a heading\n\n\n\n```\n");
    assert!(
        diagnostics.is_empty(),
        "code block content was linted: {:?}",
        diagnostics
    );
}

#[gtk4::test]
fn test_lint_tags_use_character_columns() {
    let tag_table = gtk4::TextTagTable::new();
    register_tags(&tag_table);

    let buffer = gtk4::TextBuffer::builder()
        .tag_table(&tag_table)
        .build();
    buffer.set_text("Привет   \n");

    let diagnostics = validate("Привет   \n");
    apply_to_buffer(&buffer, &diagnostics);

    let warning_tag = tag_table
        .lookup("lint-warning")
        .expect("warning tag is registered");

    // Третий пробел находится на 8-й позиции в символах. Если бы теги
    // ставились по байтовым offset'ам, диапазон оказался бы за концом
    // строки и подчёркивания не было бы.
    let inside = buffer.iter_at_line_offset(0, 8).expect("inside line");
    assert!(inside.has_tag(&warning_tag));

    let before = buffer.iter_at_line_offset(0, 5).expect("inside line");
    assert!(!before.has_tag(&warning_tag));
}


#[gtk4::test]
fn test_document_applies_and_removes_lint_tags() {
    let settings = Rc::new(RefCell::new(AppSettings::default()));
    let doc = Document::new(settings, None, false).expect("create document");

    // restore_draft имитирует восстановление черновика и сразу запускает
    // проверку, не дожидаясь дебаунса.
    doc.restore_draft("Text   \n", 0, 0);
    assert_eq!(doc.diagnostics.borrow().len(), 1);

    let tag = doc
        .tag_table
        .lookup("lint-warning")
        .expect("warning tag is registered");
    let inside = doc.buffer.iter_at_line_offset(0, 5).expect("inside line");
    assert!(inside.has_tag(&tag));

    // Выключение настройки должно снять подчёркивания, но не тронуть текст.
    doc.settings.borrow_mut().markdown_lint = false;
    doc.apply_markdown_lint_setting();

    assert!(doc.diagnostics.borrow().is_empty());
    assert!(!inside.has_tag(&tag));
    let text = doc
        .buffer
        .text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false);
    assert_eq!(text.as_str(), "Text   \n");
}

#[test]
fn test_diagnostic_lookup_finds_rule_at_position() {
    let diagnostics = validate("Text   \n");

    let found = diagnostic_at(&diagnostics, 0, 5).expect("position is inside MD009");
    assert_eq!(found.rule, Rule::Md009);

    // Позиция до хвостовых пробелов не должна попадать в диагностику.
    assert!(diagnostic_at(&diagnostics, 0, 1).is_none());
    // Другая строка — тоже промах.
    assert!(diagnostic_at(&diagnostics, 1, 5).is_none());
}

#[gtk4::test]
fn test_file_monitor_hash_is_tracked_after_save_and_reload() {
    // Регрессия: после сохранения и перезагрузки хеш содержимого должен
    // оставаться актуальным. Иначе GFileMonitor принимает собственную
    // запись за внешнее изменение и показывает лишние тосты.
    let dir = std::env::temp_dir().join(format!(
        "aster-monitor-hash-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("document.md");
    std::fs::write(&path, "old\n").expect("write initial file");

    let settings = Rc::new(RefCell::new(AppSettings::default()));
    let doc = Document::new(settings, Some(path.clone()), false).expect("create document");
    assert_eq!(
        *doc.last_external_file_hash.borrow(),
        Some(hash_text("old\n"))
    );

    std::fs::write(&path, "new\n").expect("modify file on disk");
    doc.reload_from_disk().expect("reload file");
    assert_eq!(
        *doc.last_external_file_hash.borrow(),
        Some(hash_text("new\n")),
        "reload must remember the content it has just read"
    );

    doc.buffer.set_text("saved\n");
    doc.save_to(&path).expect("save document");
    assert_eq!(
        *doc.last_external_file_hash.borrow(),
        Some(hash_text("saved\n")),
        "save must remember our own write"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_lint_md026_trailing_punctuation_in_heading() {
    let diagnostics = validate("## Heading;\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule, Rule::Md026);
    assert_eq!(diagnostics[0].line, 0);
    assert_eq!(diagnostics[0].start_col, 10);
    assert_eq!(diagnostics[0].end_col, 11);

    // Точный пример из документации проекта: заголовок с точкой с запятой.
    let example = validate("## 1. Списки задач (GFM Task Lists);\n");
    assert_eq!(example.len(), 1);
    assert_eq!(example[0].rule, Rule::Md026);

    // Вопросительный знак разрешён по умолчанию (заголовки FAQ).
    assert!(validate("## FAQ?\n").is_empty());

    // Полноширинная пунктуация тоже ловится.
    assert_eq!(validate("## Заголовок；\n")[0].rule, Rule::Md026);

    // Точка с запятой в HTML-сущности — не пунктуация заголовка.
    assert!(validate("## Copyright &copy;\n").is_empty());

    // Закрывающие решётки не мешают найти знак препинания.
    let with_closing_hashes = validate("## Heading; ##\n");
    assert_eq!(with_closing_hashes[0].rule, Rule::Md026);
    assert_eq!(with_closing_hashes[0].start_col, 10);

    // Если после сущности есть настоящая пунктуация, правило срабатывает.
    assert_eq!(validate("## A &copy; B;\n")[0].rule, Rule::Md026);

    // Внутри блока кода заголовок не проверяется.
    assert!(validate("```md\n## Heading;\n```\n").is_empty());
}

#[test]
fn test_lint_heading_formatting_rules() {
    // MD018 — нет пробела после решётки.
    let diagnostics = validate("#Heading\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule, Rule::Md018);

    // MD019 — несколько пробелов после решётки.
    let diagnostics = validate("#  Heading\n");
    assert!(diagnostics.iter().any(|d| d.rule == Rule::Md019));
    assert!(!diagnostics.iter().any(|d| d.rule == Rule::Md018));

    // MD020 — нет пробела перед закрывающими решётками.
    let diagnostics = validate("# Heading#\n");
    assert!(diagnostics.iter().any(|d| d.rule == Rule::Md020));

    // MD021 — несколько пробелов внутри закрытых решёток.
    let diagnostics = validate("#  Heading  #\n");
    assert!(diagnostics.iter().any(|d| d.rule == Rule::Md021));

    // Корректный закрытый заголовок не должен давать MD020/MD021.
    let diagnostics = validate("# Heading #\n");
    assert!(!diagnostics
        .iter()
        .any(|d| matches!(d.rule, Rule::Md020 | Rule::Md021)));

    // MD023 — заголовок с отступом.
    let diagnostics = validate("  # Heading\n");
    assert!(diagnostics.iter().any(|d| d.rule == Rule::Md023));

    // MD024 — повторяющийся текст заголовка.
    let diagnostics = validate("# Same\n\n## Same\n");
    let duplicates: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.rule == Rule::Md024)
        .collect();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0].line, 2);

    // MD024 не срабатывает на разные заголовки.
    assert!(!validate("# One\n\n## Two\n")
        .iter()
        .any(|d| d.rule == Rule::Md024));

    // Внутри блока кода заголовки не проверяются.
    assert!(!validate("```md\n#Heading\n```\n")
        .iter()
        .any(|d| d.rule == Rule::Md018));
}

#[test]
fn test_lint_list_rules() {
    // MD007 — вложенный список с отступом 3 вместо 2.
    let diagnostics = validate("- item\n   - nested\n");
    assert!(diagnostics.iter().any(|d| d.rule == Rule::Md007));
    assert!(!diagnostics.iter().any(|d| d.rule == Rule::Md030));

    // Корректный отступ 2 пробела.
    assert!(!validate("- item\n  - nested\n")
        .iter()
        .any(|d| d.rule == Rule::Md007));

    // Вложенный список под упорядоченным родителем MD007 не проверяет.
    assert!(!validate("1. item\n   - nested\n")
        .iter()
        .any(|d| d.rule == Rule::Md007));

    // MD029 — разрыв в нумерации.
    let diagnostics = validate("1. one\n3. three\n");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.rule == Rule::Md029)
            .count(),
        1
    );

    // Последовательная нумерация и стиль «все единицы» корректны.
    assert!(!validate("1. one\n1. two\n")
        .iter()
        .any(|d| d.rule == Rule::Md029));
    assert!(!validate("1. one\n2. two\n")
        .iter()
        .any(|d| d.rule == Rule::Md029));
    assert!(!validate("0. zero\n1. one\n2. two\n")
        .iter()
        .any(|d| d.rule == Rule::Md029));

    // MD030 — лишние пробелы после маркера.
    assert!(validate("-  item\n")
        .iter()
        .any(|d| d.rule == Rule::Md030));
    assert!(validate("1.  item\n")
        .iter()
        .any(|d| d.rule == Rule::Md030));
    assert!(!validate("- item\n")
        .iter()
        .any(|d| d.rule == Rule::Md030));
}

#[test]
fn test_lint_link_and_image_rules() {
    // MD042 — пустая ссылка.
    assert!(validate("[empty]()\n")
        .iter()
        .any(|d| d.rule == Rule::Md042));
    assert!(validate("[empty](#)\n")
        .iter()
        .any(|d| d.rule == Rule::Md042));

    // Обычные ссылки и непустые якоря корректны.
    assert!(!validate("[ok](https://example.com)\n")
        .iter()
        .any(|d| d.rule == Rule::Md042));
    assert!(!validate("[ok](#fragment)\n")
        .iter()
        .any(|d| d.rule == Rule::Md042));

    // MD042 через пустое reference-определение.
    assert!(validate("[empty][ref]\n\n[ref]: #\n")
        .iter()
        .any(|d| d.rule == Rule::Md042));

    // MD045 — изображение без alt-текста.
    assert!(validate("![](image.png)\n")
        .iter()
        .any(|d| d.rule == Rule::Md045));
    assert!(validate("![ ](image.png)\n")
        .iter()
        .any(|d| d.rule == Rule::Md045));
    assert!(!validate("![alt](image.png)\n")
        .iter()
        .any(|d| d.rule == Rule::Md045));
    assert!(!validate("![**bold**](image.png)\n")
        .iter()
        .any(|d| d.rule == Rule::Md045));

    // Внутри блока кода ссылки и изображения не проверяются.
    assert!(!validate("```md\n![](image.png)\n[empty]()\n```\n")
        .iter()
        .any(|d| matches!(d.rule, Rule::Md042 | Rule::Md045)));
}

#[test]
fn test_lint_table_rules() {
    // MD055 — строка выбивается из стиля pipe-символов.
    let diagnostics = validate("| A | B |\n| --- | --- |\nA | B\n");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.rule == Rule::Md055)
            .count(),
        1
    );

    // MD056 — неверное число колонок в двух строках.
    let diagnostics = validate("| A | B |\n| --- | --- |\n| 1 |\n| 2 | 3 | 4 |\n");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.rule == Rule::Md056)
            .count(),
        2
    );

    // MD060 — смешаны плотный заголовок с компактными строками.
    assert!(validate("|A|B|\n| --- | --- |\n| 1 | 2 |\n")
        .iter()
        .any(|d| d.rule == Rule::Md060));

    // Каждый из поддерживаемых стилей по отдельности корректен.
    assert!(!validate("|A|B|\n|---|---|\n|1|2|\n")
        .iter()
        .any(|d| d.rule == Rule::Md060));
    assert!(!validate("| A | B |\n| --- | --- |\n| 1 | 2 |\n")
        .iter()
        .any(|d| d.rule == Rule::Md060));
    assert!(!validate(
        "| Character | Meaning |\n| --------- | ------- |\n| Y         | Yes     |\n"
    )
    .iter()
    .any(|d| d.rule == Rule::Md060));

    // Текст сразу после таблицы считается её частью.
    let diagnostics = validate("| A | B |\n| --- | --- |\n| 1 | 2 |\nText\n");
    assert!(diagnostics.iter().any(|d| d.rule == Rule::Md055));
    assert!(diagnostics.iter().any(|d| d.rule == Rule::Md056));

    // Внутри блока кода таблицы не проверяются.
    assert!(!validate("```md\n|A|B|\n| --- | --- |\n```\n")
        .iter()
        .any(|d| matches!(d.rule, Rule::Md055 | Rule::Md056 | Rule::Md060)));
}

#[test]
fn test_lint_github_alerts() {
    for kind in ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION", "note"] {
        let text = format!("> [!{kind}]\n> Text\n");
        assert!(
            !validate(&text).iter().any(|d| d.rule == Rule::GhAlert),
            "valid alert {kind} was reported"
        );
    }

    // Неизвестный тип и лишний `!` не станут GitHub Alert.
    let diagnostics = validate("> [!alert!]\n> Text\n");
    let alert = diagnostics
        .iter()
        .find(|d| d.rule == Rule::GhAlert)
        .expect("GHALERT expected");
    assert_eq!(alert.start_col, 2);
    assert_eq!(alert.end_col, 11);

    // Текст после типа на той же строке тоже ломает callout.
    assert!(validate("> [!NOTE] Title\n> Text\n")
        .iter()
        .any(|d| d.rule == Rule::GhAlert));

    // В блоке кода alert-разметка не проверяется.
    assert!(!validate("```md\n> [!alert!]\n```\n")
        .iter()
        .any(|d| d.rule == Rule::GhAlert));

    // Вне цитаты это просто текст.
    assert!(!validate("[!alert!]\n")
        .iter()
        .any(|d| d.rule == Rule::GhAlert));
}

#[gtk4::test]
fn test_modified_flag_clears_after_undo() {
    let settings = Rc::new(RefCell::new(AppSettings::default()));
    let doc = Document::new(settings, None, false).expect("create document");
    assert!(!doc.is_modified.get());

    let mut end = doc.buffer.end_iter();
    doc.buffer.insert(&mut end, "temporary text");
    doc.recompute_modified();
    assert!(doc.is_modified.get(), "typing must mark the document dirty");

    doc.buffer.undo();
    doc.recompute_modified();
    assert!(
        !doc.is_modified.get(),
        "undo back to the saved content must clear the dirty flag"
    );
}

#[gtk4::test]
fn test_modified_flag_is_relative_to_disk_after_external_change() {
    let settings = Rc::new(RefCell::new(AppSettings::default()));
    let doc = Document::new(settings, None, false).expect("create document");

    // Имитируем: на диске уже другая версия, а в буфере — локальная.
    doc.buffer.set_text("local");
    doc.saved_content_hash.set(Some(hash_text("external")));
    doc.recompute_modified();
    assert!(doc.is_modified.get());

    // Буфер совпал с внешней версией — конфликта и правок нет.
    doc.buffer.set_text("external");
    doc.recompute_modified();
    assert!(!doc.is_modified.get());
}

#[gtk4::test]
fn test_format_selection_wraps_and_preserves_inner_selection() {
    for (open, close, expected) in [
        ("**", "**", "**hello**"),
        ("*", "*", "*hello*"),
        ("`", "`", "`hello`"),
    ] {
        let settings = Rc::new(RefCell::new(AppSettings::default()));
        let doc = Document::new(settings, None, false).expect("create document");
        doc.buffer.set_text("hello");

        let start = doc.buffer.iter_at_offset(0);
        let end = doc.buffer.iter_at_offset(5);
        doc.buffer.select_range(&start, &end);

        doc.wrap_selection(open, close);

        let full = doc
            .buffer
            .text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false);
        assert_eq!(full.as_str(), expected);

        let (selected_start, selected_end) =
            doc.buffer.selection_bounds().expect("selection remains");
        let selected = doc.buffer.text(&selected_start, &selected_end, false);
        assert_eq!(selected.as_str(), "hello");
    }
}

#[gtk4::test]
fn test_cycle_selection_case() {
    let settings = Rc::new(RefCell::new(AppSettings::default()));
    let doc = Document::new(settings, None, false).expect("create document");
    doc.buffer.set_text("qWeRtY");

    let start = doc.buffer.iter_at_offset(0);
    let end = doc.buffer.iter_at_offset(6);
    doc.buffer.select_range(&start, &end);

    doc.cycle_selection_case();
    assert_eq!(doc.buffer.text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false).as_str(), "QWERTY");

    doc.cycle_selection_case();
    assert_eq!(doc.buffer.text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false).as_str(), "qwerty");
}

#[gtk4::test]
fn test_format_selection_is_one_undo_step() {
    let settings = Rc::new(RefCell::new(AppSettings::default()));
    let doc = Document::new(settings, None, false).expect("create document");
    doc.buffer.set_text("hello");

    let start = doc.buffer.iter_at_offset(0);
    let end = doc.buffer.iter_at_offset(5);
    doc.buffer.select_range(&start, &end);
    doc.wrap_selection("**", "**");

    doc.buffer.undo();
    let text = doc
        .buffer
        .text(&doc.buffer.start_iter(), &doc.buffer.end_iter(), false);
    assert_eq!(text.as_str(), "hello");
}
