use pulldown_cmark::{html, Options, Parser};

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

    let parser = Parser::new_ext(markdown_input, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);

    let palette = ThemePalette::for_theme(theme, system_is_dark);
    let theme_mode = if palette.is_dark { "dark" } else { "light" };

    // Вычисляем ширину центрального блока: 100% - 2 * padding_percent
    let safe_padding = padding_percent.min(45);
    let body_width_percent = 100 - (safe_padding * 2);

    let base_tag = match base_url {
        Some(url) => format!(r#"<base href="{url}">"#),
        None => String::new(),
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="{theme_mode}">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
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

        body {{
            box-sizing: border-box;
            width: {body_width_percent}%;
            max-width: {body_width_percent}%;
            margin: 0 auto;
            padding: 35px {safe_padding}%;
            font-size: {font_size_px}px;
        }}

        @media (max-width: 767px) {{
            body {{
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
    }

    #[test]
    fn test_render_table_and_base_url() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let html = render_markdown_to_html(md, "dark", true, 16, 5, Some("file:///tmp/"));
        assert!(html.contains("<table>"));
        assert!(html.contains(r#"<base href="file:///tmp/">"#));
    }
}
