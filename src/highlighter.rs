//! Модуль легковесной подсветки синтаксиса Markdown для `GtkTextBuffer`.
//!
//! Использует текстовые теги `GtkTextTag`, не требуя внешних тяжелых библиотек.

use gtk4::prelude::*;

/// Цвета подсветки синтаксиса Markdown в зависимости от темы
pub struct HighlightColors {
    pub header: &'static str,
    pub bold: &'static str,
    pub italic: &'static str,
    pub code: &'static str,
    pub link: &'static str,
    pub quote: &'static str,
    pub list: &'static str,
}

impl HighlightColors {
    pub fn for_theme(theme: &str, is_dark: bool) -> Self {
        match theme {
            "dracula" => Self {
                header: "#bd93f9",
                bold: "#ffb86c",
                italic: "#ff79c6",
                code: "#50fa7b",
                link: "#8be9fd",
                quote: "#6272a4",
                list: "#f1fa8c",
            },
            "kimbie_dark" => Self {
                header: "#f79a32",
                bold: "#dc3958",
                italic: "#98676a",
                code: "#889b4a",
                link: "#7e602c",
                quote: "#a57a4c",
                list: "#f06431",
            },
            "solarized_dark" => Self {
                header: "#268bd2",
                bold: "#cb4b16",
                italic: "#6c71c4",
                code: "#859900",
                link: "#2aa198",
                quote: "#586e75",
                list: "#b58900",
            },
            "solarized_light" => Self {
                header: "#268bd2",
                bold: "#cb4b16",
                italic: "#6c71c4",
                code: "#859900",
                link: "#2aa198",
                quote: "#93a1a1",
                list: "#b58900",
            },
            _ => {
                if is_dark {
                    Self {
                        header: "#79c0ff",
                        bold: "#ffa657",
                        italic: "#d2a8ff",
                        code: "#7ee787",
                        link: "#58a6ff",
                        quote: "#8b949e",
                        list: "#d29922",
                    }
                } else {
                    Self {
                        header: "#0969da",
                        bold: "#bc4c00",
                        italic: "#8250df",
                        code: "#1a7f37",
                        link: "#0550ae",
                        quote: "#656d76",
                        list: "#953800",
                    }
                }
            }
        }
    }
}

/// Инициализирует и регистрирует теги подсветки синтаксиса Markdown в таблице тегов буфера.
pub fn register_highlight_tags(tag_table: &gtk4::TextTagTable, theme: &str, is_dark: bool) {
    let colors = HighlightColors::for_theme(theme, is_dark);

    let tags = [
        ("md-header", colors.header, 700, false),
        ("md-bold", colors.bold, 700, false),
        ("md-italic", colors.italic, 400, true),
        ("md-code", colors.code, 400, false),
        ("md-link", colors.link, 400, false),
        ("md-quote", colors.quote, 400, true),
        ("md-list", colors.list, 600, false),
    ];

    for (name, fg, weight, italic) in tags {
        let tag = gtk4::TextTag::builder()
            .name(name)
            .foreground(fg)
            .weight(weight)
            .style(if italic { gtk4::pango::Style::Italic } else { gtk4::pango::Style::Normal })
            .build();
        tag_table.add(&tag);
    }
}

/// Обновляет цвета зарегистрированных тегов подсветки при смене темы.
pub fn update_highlight_colors(tag_table: &gtk4::TextTagTable, theme: &str, is_dark: bool) {
    let colors = HighlightColors::for_theme(theme, is_dark);

    let tag_colors = [
        ("md-header", colors.header),
        ("md-bold", colors.bold),
        ("md-italic", colors.italic),
        ("md-code", colors.code),
        ("md-link", colors.link),
        ("md-quote", colors.quote),
        ("md-list", colors.list),
    ];

    for (name, color) in tag_colors {
        if let Some(tag) = tag_table.lookup(name) {
            tag.set_foreground(Some(color));
        }
    }
}

/// Выполняет быстрый проход по строкам буфера и расставляет теги подсветки Markdown.
pub fn highlight_markdown_buffer(buffer: &gtk4::TextBuffer) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();

    let tag_names = [
        "md-header",
        "md-bold",
        "md-italic",
        "md-code",
        "md-link",
        "md-quote",
        "md-list",
    ];

    for name in tag_names {
        buffer.remove_tag_by_name(name, &start, &end);
    }

    let line_count = buffer.line_count();
    let mut in_code_block = false;

    for i in 0..line_count {
        let line_start = buffer.iter_at_line(i).unwrap_or_else(|| buffer.end_iter());
        let mut line_end = line_start;
        line_end.forward_to_line_end();

        let line_text = buffer.text(&line_start, &line_end, false);
        let trimmed = line_text.trim_start();

        // Проверяем многострочные блоки кода ```
        if trimmed.starts_with("```") {
            buffer.apply_tag_by_name("md-code", &line_start, &line_end);
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            buffer.apply_tag_by_name("md-code", &line_start, &line_end);
            continue;
        }

        // 1. Заголовки: # Заголовок
        if trimmed.starts_with('#') {
            let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
            if hash_count <= 6 && trimmed.chars().nth(hash_count) == Some(' ') {
                buffer.apply_tag_by_name("md-header", &line_start, &line_end);
                continue;
            }
        }

        // 2. Цитаты: > Цитата
        if trimmed.starts_with('>') {
            buffer.apply_tag_by_name("md-quote", &line_start, &line_end);
            continue;
        }

        // 3. Списки: - , * , + , 1.
        if trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
            || (trimmed.len() > 2 && trimmed.chars().next().map_or(false, |c| c.is_ascii_digit()) && trimmed.contains(". "))
        {
            let mut prefix_end = line_start;
            let offset = line_text.len() - trimmed.len() + 2;
            prefix_end.forward_chars(offset as i32);
            buffer.apply_tag_by_name("md-list", &line_start, &prefix_end);
        }

        // 4. Поиск инлайн-элементов: `код`, **жирный**, [текст](ссылка)
        highlight_inline(buffer, i, &line_text);
    }
}

/// Выделяет инлайн-элементы Markdown в пределах одной строки
fn highlight_inline(buffer: &gtk4::TextBuffer, line_idx: i32, text: &str) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut idx = 0;

    while idx < len {
        // Инлайн-код `...`
        if bytes[idx] == b'`' {
            if let Some(close) = text[idx + 1..].find('`') {
                let start_char = text[..idx].chars().count();
                let match_len = text[idx..idx + 1 + close + 1].chars().count();

                let mut s_iter = buffer.iter_at_line(line_idx).unwrap_or_else(|| buffer.end_iter());
                s_iter.forward_chars(start_char as i32);
                let mut e_iter = s_iter;
                e_iter.forward_chars(match_len as i32);

                buffer.apply_tag_by_name("md-code", &s_iter, &e_iter);
                idx += 1 + close + 1;
                continue;
            }
        }

        // Жирный текст **...**
        if idx + 1 < len && bytes[idx] == b'*' && bytes[idx + 1] == b'*' {
            if let Some(close) = text[idx + 2..].find("**") {
                let start_char = text[..idx].chars().count();
                let match_len = text[idx..idx + 2 + close + 2].chars().count();

                let mut s_iter = buffer.iter_at_line(line_idx).unwrap_or_else(|| buffer.end_iter());
                s_iter.forward_chars(start_char as i32);
                let mut e_iter = s_iter;
                e_iter.forward_chars(match_len as i32);

                buffer.apply_tag_by_name("md-bold", &s_iter, &e_iter);
                idx += 2 + close + 2;
                continue;
            }
        }

        // Ссылки [текст](ссылка)
        if bytes[idx] == b'[' {
            if let Some(bracket_close) = text[idx + 1..].find(']') {
                let after_bracket = idx + 1 + bracket_close + 1;
                if after_bracket < len && bytes[after_bracket] == b'(' {
                    if let Some(paren_close) = text[after_bracket + 1..].find(')') {
                        let total_end = after_bracket + 1 + paren_close + 1;
                        let start_char = text[..idx].chars().count();
                        let match_len = text[idx..total_end].chars().count();

                        let mut s_iter = buffer.iter_at_line(line_idx).unwrap_or_else(|| buffer.end_iter());
                        s_iter.forward_chars(start_char as i32);
                        let mut e_iter = s_iter;
                        e_iter.forward_chars(match_len as i32);

                        buffer.apply_tag_by_name("md-link", &s_iter, &e_iter);
                        idx = total_end;
                        continue;
                    }
                }
            }
        }

        idx += 1;
    }
}
