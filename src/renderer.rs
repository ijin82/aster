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

    // Настраиваем цвета и переменные GitHub в зависимости от выбранной темы
    let theme_mode = if is_dark { "dark" } else { "light" };

    // Цвета для фона, текста, границ, таблиц и блоков кода
    // Используем мягкие, не режущие глаз тона оригинального GitHub:
    // В темной теме фон страницы #0d1117, а блок кода мягкий темно-серый #161b22 с рамкой #30363d.
    // В светлой теме фон страницы #ffffff, а блок кода мягкий светло-серый #f6f8fa с рамкой #d0d7de.
    let (bg_default, bg_muted, fg_default, border_default, code_bg, code_inline_bg) = if is_dark {
        (
            "#0d1117",              // Основной фон страницы
            "#151b23",              // Фон шапки таблиц
            "#e6edf3",              // Основной цвет текста
            "#30363d",              // Границы
            "#161b22",              // Мягкий фон многострочного блока кода <pre>
            "rgba(110,118,129,0.4)", // Фон инлайн-кода `code`
        )
    } else {
        (
            "#ffffff",              // Основной фон страницы
            "#f6f8fa",              // Фон шапки таблиц
            "#1f2328",              // Основной цвет текста
            "#d0d7de",              // Границы
            "#f6f8fa",              // Мягкий фон многострочного блока кода <pre>
            "rgba(175,184,193,0.2)", // Фон инлайн-кода `code`
        )
    };

    // Формируем готовый HTML5 документ со сбалансированной палитрой
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
            background-color: {bg_default} !important;
            color: {fg_default} !important;
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

        .markdown-body {{
            background-color: transparent !important;
            color: {fg_default} !important;
            font-size: {font_size_px}px;
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

        /* Чередование строк таблицы (зебра) */
        .markdown-body table tr:nth-child(2n) {{
            background-color: {bg_muted} !important;
        }}
    </style>
</head>
<body class="markdown-body" data-theme="{theme_mode}">
{html_output}
</body>
</html>"#
    )
}
