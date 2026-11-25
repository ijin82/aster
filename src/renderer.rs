use pulldown_cmark::{html, BlockQuoteKind, Event, Options, Parser, Tag, TagEnd};

/// Встроенная таблица стилей GitHub Markdown CSS.
/// Включается непосредственно в бинарный файл во время компиляции через `include_str!`,
/// благодаря чему приложению не требуются внешние runtime-файлы стилей.
const GITHUB_CSS: &str = include_str!("../assets/github-markdown.css");

/// Описание параметров цветовой схемы для предпросмотра
struct ThemePalette {
    is_dark: bool,
    bg_default: &'static str,
    bg_muted: &'static str,
    fg_default: &'static str,
    border_default: &'static str,
    code_bg: &'static str,
    code_inline_bg: &'static str,
    accent_color: &'static str,
}

impl ThemePalette {
    fn for_theme(theme: &str, system_is_dark: bool) -> Self {
        match theme {
            "dracula" => Self {
                is_dark: true,
                bg_default: "#282a36",
                bg_muted: "#21222c",
                fg_default: "#f8f8f2",
                border_default: "#6272a4",
                code_bg: "#21222c",
                code_inline_bg: "rgba(98, 114, 164, 0.4)",
                accent_color: "#bd93f9",
            },
            "kimbie_dark" => Self {
                is_dark: true,
                bg_default: "#221a0f",
                bg_muted: "#2a2013",
                fg_default: "#d3af86",
                border_default: "#55432d",
                code_bg: "#2a2013",
                code_inline_bg: "rgba(138, 91, 51, 0.35)",
                accent_color: "#f79a32",
            },
            "solarized_dark" => Self {
                is_dark: true,
                bg_default: "#002b36",
                bg_muted: "#073642",
                fg_default: "#839496",
                border_default: "#586e75",
                code_bg: "#073642",
                code_inline_bg: "rgba(88, 110, 117, 0.35)",
                accent_color: "#268bd2",
            },
            "solarized_light" => Self {
                is_dark: false,
                bg_default: "#fdf6e3",
                bg_muted: "#eee8d5",
                fg_default: "#657b83",
                border_default: "#93a1a1",
                code_bg: "#eee8d5",
                code_inline_bg: "rgba(147, 161, 161, 0.25)",
                accent_color: "#268bd2",
            },
            "light" => Self {
                is_dark: false,
                bg_default: "#ffffff",
                bg_muted: "#f6f8fa",
                fg_default: "#1f2328",
                border_default: "#d0d7de",
                code_bg: "#f6f8fa",
                code_inline_bg: "rgba(175, 184, 193, 0.2)",
                accent_color: "#0969da",
            },
            "dark" => Self {
                is_dark: true,
                bg_default: "#0d1117",
                bg_muted: "#151b23",
                fg_default: "#e6edf3",
                border_default: "#30363d",
                code_bg: "#161b22",
                code_inline_bg: "rgba(110, 118, 129, 0.4)",
                accent_color: "#4493f8",
            },
            _ => {
                // "system": выбираем GitHub Dark или GitHub Light
                if system_is_dark {
                    Self::for_theme("dark", true)
                } else {
                    Self::for_theme("light", false)
                }
            }
        }
    }
}

/// HTML-иконки GitHub для alert-блоков.
///
/// Используются оригинальные Octicons: они наследуют цвет заголовка
/// через `currentColor`, поэтому одинаково работают во всех темах.
const ALERT_ICON_NOTE: &str = r#"<svg class="octicon octicon-info mr-2" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M0 8a8 8 0 1 1 16 0A8 8 0 0 1 0 8Zm8-6.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13ZM6.5 7.75A.75.75 0 0 1 7.25 7h1a.75.75 0 0 1 .75.75v2.75h.25a.75.75 0 0 1 0 1.5h-2a.75.75 0 0 1 0-1.5h.25v-2h-.25a.75.75 0 0 1-.75-.75ZM8 6a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z"/></svg>"#;
const ALERT_ICON_TIP: &str = r#"<svg class="octicon octicon-light-bulb mr-2" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M8 1.5c-2.363 0-4 1.69-4 3.75 0 .984.424 1.625.984 2.304l.214.253c.223.264.47.556.673.848.284.411.537.896.621 1.49a.75.75 0 0 1-1.484.211c-.04-.282-.163-.547-.37-.847a8.456 8.456 0 0 0-.542-.68c-.084-.1-.173-.205-.268-.32C3.201 7.75 2.5 6.766 2.5 5.25 2.5 2.31 4.863 0 8 0s5.5 2.31 5.5 5.25c0 1.516-.701 2.5-1.328 3.259-.095.115-.184.22-.268.319-.207.245-.383.453-.541.681-.208.3-.33.565-.37.847a.751.751 0 0 1-1.485-.212c.084-.593.337-1.078.621-1.489.203-.292.45-.584.673-.848.075-.088.147-.173.213-.253.561-.679.985-1.32.985-2.304 0-2.06-1.637-3.75-4-3.75ZM5.75 12h4.5a.75.75 0 0 1 0 1.5h-4.5a.75.75 0 0 1 0-1.5ZM6 15.25a.75.75 0 0 1 .75-.75h2.5a.75.75 0 0 1 0 1.5h-2.5a.75.75 0 0 1-.75-.75Z"/></svg>"#;
const ALERT_ICON_IMPORTANT: &str = r#"<svg class="octicon octicon-report mr-2" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M0 1.75C0 .784.784 0 1.75 0h12.5C15.216 0 16 .784 16 1.75v9.5A1.75 1.75 0 0 1 14.25 13H8.06l-2.573 2.573A1.458 1.458 0 0 1 3 14.543V13H1.75A1.75 1.75 0 0 1 0 11.25Zm1.75-.25a.25.25 0 0 0-.25.25v9.5c0 .138.112.25.25.25h2a.75.75 0 0 1 .75.75v2.19l2.72-2.72a.749.749 0 0 1 .53-.22h6.5a.25.25 0 0 0 .25-.25v-9.5a.25.25 0 0 0-.25-.25Zm7 2.25v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 9a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z"/></svg>"#;
const ALERT_ICON_WARNING: &str = r#"<svg class="octicon octicon-alert mr-2" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M6.457 1.047c.659-1.234 2.427-1.234 3.086 0l6.082 11.378A1.75 1.75 0 0 1 14.082 15H1.918a1.75 1.75 0 0 1-1.543-2.575Zm1.763.707a.25.25 0 0 0-.44 0L1.698 13.132a.25.25 0 0 0 .22.368h12.164a.25.25 0 0 0 .22-.368Zm.53 3.996v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 11a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z"/></svg>"#;
const ALERT_ICON_CAUTION: &str = r#"<svg class="octicon octicon-stop mr-2" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="M4.47.22A.749.749 0 0 1 5 0h6c.199 0 .389.079.53.22l4.25 4.25c.141.14.22.331.22.53v6a.749.749 0 0 1-.22.53l-4.25 4.25A.749.749 0 0 1 11 16H5a.749.749 0 0 1-.53-.22L.22 11.53A.749.749 0 0 1 0 11V5c0-.199.079-.389.22-.53Zm.84 1.28L1.5 5.31v5.38l3.81 3.81h5.38l3.81-3.81V5.31L10.69 1.5ZM8 4a.75.75 0 0 1 .75.75v3.5a.75.75 0 0 1-1.5 0v-3.5A.75.75 0 0 1 8 4Zm0 8a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z"/></svg>"#;

/// Преобразует блок-цитаты GitHub-алертов в HTML-структуру GitHub.
///
/// `pulldown-cmark` с `ENABLE_GFM` уже распознаёт `[!NOTE]` и отдаёт
/// `BlockQuoteKind`, но его HTML writer выводит только `<blockquote>`.
/// Здесь добавляются обёртка `.markdown-alert`, заголовок с иконкой и
/// закрывающий `</div>`, чтобы заработал `github-markdown-css`.
fn transform_alerts<'a>(parser: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut events = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::BlockQuote(Some(kind))) => {
                let (class, title, icon) = match kind {
                    BlockQuoteKind::Note => ("note", "Note", ALERT_ICON_NOTE),
                    BlockQuoteKind::Tip => ("tip", "Tip", ALERT_ICON_TIP),
                    BlockQuoteKind::Important => ("important", "Important", ALERT_ICON_IMPORTANT),
                    BlockQuoteKind::Warning => ("warning", "Warning", ALERT_ICON_WARNING),
                    BlockQuoteKind::Caution => ("caution", "Caution", ALERT_ICON_CAUTION),
                };

                events.push(Event::Html(
                    format!(r#"<div class="markdown-alert markdown-alert-{class}" dir="auto">"#)
                        .into(),
                ));
                events.push(Event::Html(
                    format!(r#"<p class="markdown-alert-title" dir="auto">{icon}{title}</p>"#)
                        .into(),
                ));
            }
            Event::End(TagEnd::BlockQuote(Some(_))) => {
                events.push(Event::Html("</div>".into()));
            }
            other => events.push(other),
        }
    }

    events
}

/// Преобразует сырой текст в формате Markdown в полноценный HTML-документ
/// с выбранной цветовой палитрой и настраиваемыми отступами.
///
/// # Аргументы
/// * `markdown_input` — исходный текст на языке разметки Markdown.
/// * `theme` — название темы ("system", "light", "dark", "dracula", "kimbie_dark", "solarized_dark", "solarized_light").
/// * `system_is_dark` — флаг темной темы системы (для режима "system").
/// * `font_size_px` — размер базового шрифта в пикселях.
/// * `padding_percent` — процент боковых полей (по умолчанию 5%, ширина контента = 100 - 2 * padding).
/// * `base_url` — опциональный базовый URI для разрешения локальных картинок.
pub fn render_markdown_to_html(
    markdown_input: &str,
    theme: &str,
    system_is_dark: bool,
    font_size_px: u32,
    padding_percent: u32,
    base_url: Option<&str>,
) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    // Включает GitHub-алерты: `> [!NOTE]` и четыре других типа.
    options.insert(Options::ENABLE_GFM);

    let parser = Parser::new_ext(markdown_input, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, transform_alerts(parser).into_iter());

    let palette = ThemePalette::for_theme(theme, system_is_dark);
    let theme_mode = if palette.is_dark { "dark" } else { "light" };

    // Вычисляем ширину центрального блока: 100% - 2 * padding_percent
    let safe_padding = padding_percent.min(45);
    let body_width_percent = 100 - (safe_padding * 2);

    let base_tag = match base_url {
        Some(url) => {
            let escaped = url
                .replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            format!(r#"<base href="{escaped}">"#)
        }
        None => String::new(),
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="{theme_mode}">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <!-- Предпросмотр не должен исполнять скрипты из Markdown и не должен
         иметь доступ к сети. Собственный скрипт Aster внедряется как
         WebKit user script и CSP не блокируется. -->
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; base-uri file: http: https:; img-src file: data: https: http:; style-src 'unsafe-inline'; font-src data:; script-src 'none'; connect-src 'none'; object-src 'none'; frame-src 'none'; media-src 'none'; form-action 'none';">
    {base_tag}
    <style>
        {GITHUB_CSS}

        html, body {{
            margin: 0;
            padding: 0;
            background-color: {bg_default} !important;
            color: {fg_default} !important;
            color-scheme: {theme_mode};
        }}

        /* body.markdown-body, а не просто body: GitHub CSS задает
           .markdown-body с margin: 0 и более высокой специфичностью.
           Из-за этого margin:auto не применялся, и справа оставалась
           лишняя пустая полоса. */
        body.markdown-body {{
            box-sizing: border-box;
            width: {body_width_percent}%;
            max-width: {body_width_percent}%;
            margin: 0 auto;
            padding: 35px {safe_padding}%;
            font-size: {font_size_px}px;
        }}

        @media (max-width: 767px) {{
            body.markdown-body {{
                width: 96%;
                max-width: 96%;
                padding: 15px 2%;
            }}
        }}

        .markdown-body {{
            background-color: transparent !important;
            color: {fg_default} !important;
            font-size: {font_size_px}px;
        }}

        .markdown-body a {{
            color: {accent_color} !important;
        }}

        /* --- Блоки кода (fenced code blocks <pre><code>) --- */
        .markdown-body pre {{
            background-color: {code_bg} !important;
            color: {fg_default} !important;
            border: 1px solid {border_default} !important;
            border-radius: 6px !important;
            padding: 16px !important;
            overflow: auto !important;
            line-height: 1.45 !important;
            font-size: 88% !important;
        }}

        .markdown-body pre code {{
            background-color: transparent !important;
            color: {fg_default} !important;
            padding: 0 !important;
            margin: 0 !important;
            border: none !important;
            font-size: 100% !important;
        }}

        /* --- Встроенный инлайн-код (`foo`) --- */
        .markdown-body :not(pre) > code {{
            background-color: {code_inline_bg} !important;
            color: {fg_default} !important;
            padding: 0.2em 0.4em !important;
            margin: 0 !important;
            font-size: 85% !important;
            border-radius: 6px !important;
        }}

        /* --- Таблицы (GFM Tables) --- */
        .markdown-body table {{
            border-collapse: collapse !important;
            width: max-content;
            max-width: 100%;
            margin-top: 1em;
            margin-bottom: 1.5em;
            color: {fg_default} !important;
        }}

        .markdown-body table th {{
            background-color: {bg_muted} !important;
            color: {fg_default} !important;
            font-weight: 600;
            padding: 8px 14px;
            border: 1px solid {border_default} !important;
        }}

        .markdown-body table td {{
            color: {fg_default} !important;
            padding: 8px 14px;
            border: 1px solid {border_default} !important;
        }}

        .markdown-body table tr {{
            background-color: {bg_default} !important;
            border-top: 1px solid {border_default} !important;
        }}

        .markdown-body table tr:nth-child(2n) {{
            background-color: {bg_muted} !important;
        }}

        /* Разделители и цитаты */
        .markdown-body hr {{
            background-color: {border_default} !important;
            height: 2px !important;
        }}

        .markdown-body blockquote {{
            color: {fg_default} !important;
            border-left: 0.25em solid {border_default} !important;
            opacity: 0.85;
        }}

        /* --- Подсветка поиска в предпросмотре --- */
        mark.aster-search-match {{
            background-color: #f5a97f !important;
            color: #181926 !important;
            border-radius: 2px;
            padding: 0;
        }}

        mark.aster-search-current {{
            background-color: #f97316 !important;
            color: #fff7ed !important;
            border-radius: 2px;
            padding: 0;
            font-weight: 700;
        }}
    </style>
</head>
<body class="markdown-body" data-theme="{theme_mode}">
{html_output}
</body>
</html>"#,
        bg_default = palette.bg_default,
        fg_default = palette.fg_default,
        bg_muted = palette.bg_muted,
        border_default = palette.border_default,
        code_bg = palette.code_bg,
        code_inline_bg = palette.code_inline_bg,
        accent_color = palette.accent_color,
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_markdown_html() {
        let md = "# Title\n\nParagraph 1\n\n- Item 1\n- Item 2\n\n```rust\nfn main() {}\n```\n";
        let html = render_markdown_to_html(md, "light", false, 16, 5, None);
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<p>Paragraph 1</p>"));
        assert!(html.contains("<li>Item 1</li>"));
        assert!(html.contains("<pre><code class=\"language-rust\">"));
        assert!(html.contains("class=\"markdown-body\""));
        assert!(!html.contains("source-line-anchor"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("script-src 'none'"));
        assert!(html.contains("body.markdown-body"));
        assert!(html.contains("mark.aster-search-match"));
        assert!(html.contains("mark.aster-search-current"));
    }

    #[test]
    fn test_render_table_and_base_url() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let html = render_markdown_to_html(md, "dark", true, 16, 5, Some("file:///tmp/"));
        assert!(html.contains("<table>"));
        assert!(html.contains(r#"<base href="file:///tmp/">"#));
    }

    #[test]
    fn test_render_github_alerts() {
        for (kind, class, title) in [
            ("NOTE", "note", "Note"),
            ("TIP", "tip", "Tip"),
            ("IMPORTANT", "important", "Important"),
            ("WARNING", "warning", "Warning"),
            ("CAUTION", "caution", "Caution"),
        ] {
            let md = format!("> [!{kind}]\n> Useful information.\n");
            let html = render_markdown_to_html(&md, "light", false, 16, 5, None);

            assert!(
                html.contains(&format!(
                    r#"<div class="markdown-alert markdown-alert-{class}" dir="auto">"#
                )),
                "missing wrapper for {kind}: {html}"
            );
            assert!(html.contains(r#"<p class="markdown-alert-title" dir="auto">"#));
            assert!(html.contains(&format!(">{title}</p>")));
            assert!(html.contains("<p>Useful information.</p>"));
            assert!(!html.contains("<blockquote>"));
        }
    }

    #[test]
    fn test_render_unknown_alert_stays_blockquote() {
        let md = "> [!alert!]\n> Not a GitHub alert.\n";
        let html = render_markdown_to_html(md, "light", false, 16, 5, None);

        assert!(html.contains("<blockquote>"));
        assert!(html.contains("[!alert!]"));
        assert!(!html.contains(r#"<div class="markdown-alert"#));
    }
}
