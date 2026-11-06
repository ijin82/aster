use pulldown_cmark::{html, Options, Parser};

/// Встроенная таблица стилей GitHub Markdown CSS.
/// Включается непосредственно в бинарный файл во время компиляции через `include_str!`,
/// благодаря чему приложению не требуются внешние runtime-файлы стилей.
const GITHUB_CSS: &str = include_str!("../assets/github-markdown.css");

/// Преобразует сырой текст в формате Markdown в полноценный HTML-документ,
/// стилизованный точь-в-точь как страницы на GitHub.
///
/// # Аргументы
/// * `markdown_input` — исходный текст на языке разметки Markdown.
/// * `is_dark` — флаг темной темы (true для темной темы GitHub, false для светлой).
/// * `font_size_px` — размер базового шрифта в пикселях.
pub fn render_markdown_to_html(markdown_input: &str, is_dark: bool, font_size_px: u32) -> String {
    // Настраиваем парсер Markdown, включая расширения стандарта GitHub Flavored Markdown (GFM):
    let mut options = Options::empty();
    // 1. Таблицы: | col1 | col2 |
    options.insert(Options::ENABLE_TABLES);
    // 2. Сноски: [^1]
    options.insert(Options::ENABLE_FOOTNOTES);
    // 3. Зачеркнутый текст: ~~текст~~
    options.insert(Options::ENABLE_STRIKETHROUGH);
    // 4. Списки задач: - [ ] не сделано, - [x] сделано
    options.insert(Options::ENABLE_TASKLISTS);
    // 5. Идентификаторы и атрибуты заголовков: # Заголовок {#id}
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    // Создаем поток токенов Markdown
    let parser = Parser::new_ext(markdown_input, options);

    // Конвертируем токены в разметку фрагмента HTML
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);

    // В github-markdown-css темы активируются через атрибут [data-theme="dark"] или [data-theme="light"]
    let theme_mode = if is_dark { "dark" } else { "light" };
    let bg_color = if is_dark { "#0d1117" } else { "#ffffff" };
    let text_color = if is_dark { "#e6edf3" } else { "#1f2328" };

    // Формируем готовый HTML5 документ с CSS-переменными темы GitHub
    format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="{theme_mode}">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
        {GITHUB_CSS}

        html, body {{
            margin: 0;
            padding: 0;
            background-color: {bg_color} !important;
            color: {text_color} !important;
            color-scheme: {theme_mode};
        }}

        body {{
            box-sizing: border-box;
            min-width: 200px;
            max-width: 980px;
            margin: 0 auto;
            padding: 35px 45px;
            font-size: {font_size_px}px;
        }}

        @media (max-width: 767px) {{
            body {{
                padding: 15px;
            }}
        }}

        /* Применяем размер шрифта и обеспечиваем контрастный цвет текста */
        .markdown-body {{
            background-color: transparent !important;
            color: {text_color} !important;
            font-size: {font_size_px}px;
        }}
    </style>
</head>
<body class="markdown-body" data-theme="{theme_mode}">
{html_output}
</body>
</html>"#
    )
}
