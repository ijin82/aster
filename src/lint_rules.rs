//! Тексты встроенной справки по правилам проверки стиля Markdown.
//!
//! Короткие сообщения для подсказок живут в `i18n.rs`: они показываются
//! постоянно. Здесь находятся длинные пояснения и примеры, которые нужны
//! только в окне справки. Такое разделение не раздувает i18n и позволяет
//! держать примеры рядом с описанием правила.
//!
//! Тексты написаны самостоятельно; ссылки ведут на официальную
//! документацию markdownlint, где можно прочитать детали и историю правила.

use crate::document::{Rule, Severity};
use crate::i18n::Language;

/// Пояснение и примеры для одного правила.
pub(crate) struct RuleHelp {
    /// Зачем нужно правило и что оно предотвращает.
    pub why: &'static str,
    /// Пример, который нарушает правило.
    pub bad: &'static str,
    /// Исправленный пример.
    pub good: &'static str,
}

/// Возвращает справку по правилу на выбранном языке.
pub(crate) fn rule_help(rule: Rule, lang: Language) -> RuleHelp {
    match lang {
        Language::Russian => russian_help(rule),
        Language::English => english_help(rule),
    }
}

/// Ссылка на официальную страницу правила.
pub(crate) fn docs_url(rule: Rule) -> &'static str {
    match rule {
        Rule::Md009 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md009.md",
        Rule::Md010 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md010.md",
        Rule::Md012 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md012.md",
        Rule::Md022 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md022.md",
        Rule::Md025 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md025.md",
        Rule::Md031 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md031.md",
        Rule::Md032 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md032.md",
        Rule::Md040 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md040.md",
        Rule::Md047 => "https://github.com/DavidAnson/markdownlint/blob/main/doc/md047.md",
    }
}

/// Подпись важности для строки каталога и страницы правила.
pub(crate) fn severity_label(severity: Severity, lang: Language) -> &'static str {
    match (severity, lang) {
        (Severity::Warning, Language::Russian) => "Предупреждение",
        (Severity::Warning, Language::English) => "Warning",
        (Severity::Error, Language::Russian) => "Ошибка",
        (Severity::Error, Language::English) => "Error",
    }
}

/// Проверяет, подходит ли правило под поисковый запрос.
///
/// Ищем по номеру, короткому сообщению, объяснению и примерам. Запрос
/// разбивается на слова: все слова должны встретиться хотя бы где-нибудь
/// в описании. Это позволяет писать «заголовок пустые» и находить MD022.
pub(crate) fn matches_query(rule: Rule, lang: Language, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }

    let help = rule_help(rule, lang);
    let haystack = format!(
        "{} {} {} {} {}",
        rule.id(),
        lang.lint_message(rule.id()),
        help.why,
        help.bad,
        help.good
    )
    .to_lowercase();

    query.split_whitespace().all(|word| haystack.contains(word))
}

fn russian_help(rule: Rule) -> RuleHelp {
    match rule {
        Rule::Md009 => RuleHelp {
            why: "Невидимые пробелы в конце строки копятся при редактировании, создают шум в diff и могут неожиданно стать жёстким переносом. Ровно два пробела в конце непустой строки — исключение: в Markdown это способ сделать перенос без нового абзаца. В примерах ниже символ · обозначает пробел.",
            bad: "Строка с лишними пробелами···\nСледующая строка",
            good: "Строка с переносом··\nСледующая строка",
        },
        Rule::Md010 => RuleHelp {
            why: "Ширина табуляции зависит от настроек редактора и просмотрщика, поэтому выравнивание «плывёт». В обычном тексте надёжнее пробелы; внутри блоков кода табуляция допустима и правилом не проверяется. В примерах символ ⇥ обозначает табуляцию.",
            bad: "Колонка⇥значение",
            good: "Колонка····значение",
        },
        Rule::Md012 => RuleHelp {
            why: "Markdown не требует больше одной пустой строки между блоками. Лишние пустые строки раздувают diff и затрудняют навигацию по файлу, ничего не добавляя к разметке.",
            bad: "Первый абзац\n\n\nВторой абзац",
            good: "Первый абзац\n\nВторой абзац",
        },
        Rule::Md022 => RuleHelp {
            why: "Пустая строка до и после заголовка отделяет его от соседних абзацев и списков. Без неё заголовок визуально слипается с текстом, а структура документа становится неочевидной для читателя и инструментов.",
            bad: "Текст перед заголовком\n## Заголовок\nТекст сразу после",
            good: "Текст перед заголовком\n\n## Заголовок\n\nТекст после",
        },
        Rule::Md025 => RuleHelp {
            why: "H1 — это название документа. Несколько H1 ломают иерархию для оглавления, навигации и вспомогательных технологий. Для разделов используйте H2 и ниже, оставляя один H1 на весь документ.",
            bad: "# Первый заголовок\n\n# Второй заголовок",
            good: "# Название документа\n\n## Раздел",
        },
        Rule::Md031 => RuleHelp {
            why: "Пустые строки отделяют блок кода от соседнего текста и списков. Без них fence может «прилипнуть» к абзацу и отрендериться не так, как ожидает автор.",
            bad: "Текст\n```rust\nfn main() {}\n```",
            good: "Текст\n\n```rust\nfn main() {}\n```",
        },
        Rule::Md032 => RuleHelp {
            why: "Пустая строка до и после списка отделяет его от обычных абзацев. Иначе список может визуально слиться с текстом или изменить структуру документа при рендеринге.",
            bad: "Текст\n- первый пункт\n- второй пункт\nТекст после",
            good: "Текст\n\n- первый пункт\n- второй пункт\n\nТекст после",
        },
        Rule::Md040 => RuleHelp {
            why: "Язык после открывающих ``` включает подсветку синтаксиса и подсказывает читателю, что за код перед ним. Без него блок выглядит как обычный текст, а предпросмотр Aster не сможет его подсветить.",
            bad: "```\nfn main() {}\n```",
            good: "```rust\nfn main() {}\n```",
        },
        Rule::Md047 => RuleHelp {
            why: "Один перевод строки в конце — соглашение POSIX и большинства инструментов. Он делает diff аккуратнее и избавляет от предупреждения Git «No newline at end of file». Лишние пустые строки в конце файла тоже не нужны. Символ ⏎ в примере обозначает перевод строки.",
            bad: "Последняя строка",
            good: "Последняя строка⏎",
        },
    }
}

fn english_help(rule: Rule) -> RuleHelp {
    match rule {
        Rule::Md009 => RuleHelp {
            why: "Invisible trailing spaces accumulate during editing, add noise to diffs, and can unexpectedly turn into a hard line break. Exactly two trailing spaces on a non-empty line are the exception: in Markdown that is how you add a break without starting a new paragraph. In the examples below, · stands for a space.",
            bad: "Line with trailing spaces···\nNext line",
            good: "Line with a hard break··\nNext line",
        },
        Rule::Md010 => RuleHelp {
            why: "Tab width depends on the editor and viewer settings, so alignment drifts. Spaces are more predictable in prose; inside code blocks tabs are acceptable and are not checked by this rule. In the examples, ⇥ stands for a tab.",
            bad: "Column⇥value",
            good: "Column····value",
        },
        Rule::Md012 => RuleHelp {
            why: "Markdown does not need more than one blank line between blocks. Extra blank lines bloat diffs and make the file harder to navigate without adding anything to the markup.",
            bad: "First paragraph\n\n\nSecond paragraph",
            good: "First paragraph\n\nSecond paragraph",
        },
        Rule::Md022 => RuleHelp {
            why: "A blank line before and after a heading separates it from the surrounding paragraphs and lists. Without it the heading visually sticks to the text, and the document structure becomes unclear to both readers and tools.",
            bad: "Text before the heading\n## Heading\nText right after",
            good: "Text before the heading\n\n## Heading\n\nText after",
        },
        Rule::Md025 => RuleHelp {
            why: "H1 is the title of the document. Multiple H1 headings break the outline used by tables of contents, navigation, and assistive technologies. Use H2 and lower for sections, and keep a single H1 for the whole document.",
            bad: "# First heading\n\n# Second heading",
            good: "# Document title\n\n## Section",
        },
        Rule::Md031 => RuleHelp {
            why: "Blank lines separate a code block from the surrounding text and lists. Without them a fence can stick to a paragraph and render differently than the author expects.",
            bad: "Text\n```rust\nfn main() {}\n```",
            good: "Text\n\n```rust\nfn main() {}\n```",
        },
        Rule::Md032 => RuleHelp {
            why: "A blank line before and after a list separates it from ordinary paragraphs. Otherwise the list can visually merge with the text or change the document structure when rendered.",
            bad: "Text\n- first item\n- second item\nText after",
            good: "Text\n\n- first item\n- second item\n\nText after",
        },
        Rule::Md040 => RuleHelp {
            why: "The language after the opening ``` enables syntax highlighting and tells the reader what kind of code this is. Without it the block looks like plain text, and Aster's preview cannot highlight it.",
            bad: "```\nfn main() {}\n```",
            good: "```rust\nfn main() {}\n```",
        },
        Rule::Md047 => RuleHelp {
            why: "A single final newline is a POSIX and tooling convention. It keeps diffs tidy and avoids Git's \"No newline at end of file\" warning. Extra blank lines at the end of the file are equally unnecessary. The ⏎ symbol in the example marks the newline.",
            bad: "Last line",
            good: "Last line⏎",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_rules_have_help_in_both_languages() {
        for rule in Rule::ALL {
            for lang in [Language::Russian, Language::English] {
                let help = rule_help(rule, lang);
                assert!(!help.why.is_empty(), "{} has no explanation", rule.id());
                assert!(!help.bad.is_empty(), "{} has no bad example", rule.id());
                assert!(!help.good.is_empty(), "{} has no good example", rule.id());

                let url = docs_url(rule);
                assert!(
                    url.starts_with("https://github.com/DavidAnson/markdownlint/"),
                    "{} has an unexpected documentation URL: {url}",
                    rule.id()
                );
            }
        }
    }

    #[test]
    fn test_search_matches_rule_number_and_translation() {
        assert!(matches_query(Rule::Md022, Language::Russian, "MD022"));
        assert!(matches_query(
            Rule::Md022,
            Language::Russian,
            "заголовок пустая"
        ));
        assert!(matches_query(Rule::Md010, Language::English, "tab"));
        assert!(!matches_query(Rule::Md040, Language::English, "список"));
    }
}
