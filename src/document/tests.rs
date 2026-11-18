//! Тесты документа, поиска, редактора и политики ссылок.

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
        let _gutter = setup_line_numbers_gutter(&text_view, &settings);

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
