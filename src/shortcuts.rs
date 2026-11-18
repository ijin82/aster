//! Универсальное сопоставление аппаратных клавиш с латинскими хоткеями.
//!
//! GTK-акселераторы привязаны к keyval текущей раскладки: в русской
//! раскладке `Ctrl+S` приходит как `Ctrl+ы`, и акселератор не срабатывает.
//! Здесь мы по hardware keycode находим латинскую клавишу через keymap GDK,
//! а если в keymap нет латинской группы — используем статическую таблицу
//! для Wayland (evdev) и X11.

use gtk4::gdk;
use gtk4::gdk::prelude::*;

/// Возвращает латинский символ физической клавиши.
///
/// Для Tab возвращается `'\t'`, для запятой — `','`. Функция не зависит
/// от активной раскладки, насколько это позволяет текущий keymap.
pub fn latin_char_for_keycode(keycode: u32) -> Option<char> {
    if let Some(display) = gdk::Display::default() {
        if let Some(entries) = display.map_keycode(keycode) {
            for (_, key) in entries {
                if key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab {
                    return Some('\t');
                }
                if let Some(ch) = key.to_unicode() {
                    if ch.is_ascii_alphabetic() {
                        return Some(ch.to_ascii_lowercase());
                    }
                    if ch == ',' {
                        return Some(',');
                    }
                }
            }
        }

        // GDK может использовать X11-коды (evdev + 8) или чистые evdev-коды.
        // Определяем соглашение по Tab: у X11 это keycode 23, у evdev — 15.
        let uses_x11_keycodes = display
            .map_keycode(23)
            .map(|entries| {
                entries
                    .iter()
                    .any(|(_, key)| *key == gdk::Key::Tab || *key == gdk::Key::ISO_Left_Tab)
            })
            .unwrap_or(true);

        if uses_x11_keycodes {
            return static_char_x11(keycode);
        }
        return static_char_evdev(keycode);
    }

    None
}

/// Статическая карта evdev keycode → латинская клавиша (Wayland).
fn static_char_evdev(keycode: u32) -> Option<char> {
    Some(match keycode {
        16 => 'q',
        17 => 'w',
        18 => 'e',
        19 => 'r',
        20 => 't',
        21 => 'y',
        22 => 'u',
        23 => 'i',
        24 => 'o',
        25 => 'p',
        30 => 'a',
        31 => 's',
        32 => 'd',
        33 => 'f',
        34 => 'g',
        35 => 'h',
        36 => 'j',
        37 => 'k',
        38 => 'l',
        44 => 'z',
        45 => 'x',
        46 => 'c',
        47 => 'v',
        48 => 'b',
        49 => 'n',
        50 => 'm',
        51 => ',',
        15 => '\t',
        _ => return None,
    })
}

/// Статическая карта X11 keycode → латинская клавиша.
fn static_char_x11(keycode: u32) -> Option<char> {
    Some(match keycode {
        24 => 'q',
        25 => 'w',
        26 => 'e',
        27 => 'r',
        28 => 't',
        29 => 'y',
        30 => 'u',
        31 => 'i',
        32 => 'o',
        33 => 'p',
        38 => 'a',
        39 => 's',
        40 => 'd',
        41 => 'f',
        42 => 'g',
        43 => 'h',
        44 => 'j',
        45 => 'k',
        46 => 'l',
        52 => 'z',
        53 => 'x',
        54 => 'c',
        55 => 'v',
        56 => 'b',
        57 => 'n',
        58 => 'm',
        59 => ',',
        23 => '\t',
        _ => return None,
    })
}

/// Действие `win.*` для комбинации Ctrl+клавиша.
///
/// `shift = true` соответствует Ctrl+Shift+клавиша.
pub fn ctrl_shortcut_action(key: char, shift: bool) -> Option<&'static str> {
    if shift {
        match key {
            's' => Some("win.save_as"),
            't' => Some("win.reopen_tab"),
            'h' => Some("win.toggle_meditation"),
            'l' => Some("win.toggle_line_numbers"),
            'p' => Some("win.toggle_whitespace"),
            'w' => Some("win.toggle_soft_wrap"),
            '\t' => Some("win.previous_tab"),
            _ => None,
        }
    } else {
        match key {
            's' => Some("win.save"),
            'o' => Some("win.open"),
            't' => Some("win.new_tab"),
            'w' => Some("win.close_tab"),
            'e' => Some("win.toggle_preview"),
            'f' => Some("win.find"),
            'r' => Some("win.find_replace"),
            ',' => Some("win.preferences"),
            'q' => Some("win.close"),
            '\t' => Some("win.next_tab"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_keycode_tables() {
        assert_eq!(static_char_evdev(32), Some('d'));
        assert_eq!(static_char_evdev(31), Some('s'));
        assert_eq!(static_char_evdev(59), None);
        assert_eq!(static_char_x11(40), Some('d'));
        assert_eq!(static_char_x11(39), Some('s'));
        assert_eq!(static_char_x11(32), Some('o'));
    }

    #[gtk4::test]
    fn test_keycode_translation_on_current_display() {
        let Some(display) = gdk::Display::default() else {
            return;
        };

        let has_latin = |keycode: u32, expected: char| {
            display
                .map_keycode(keycode)
                .map(|entries| {
                    entries
                        .iter()
                        .any(|(_, key)| key.to_unicode() == Some(expected))
                })
                .unwrap_or(false)
        };

        let d_keycode = if has_latin(40, 'd') { 40 } else { 32 };
        let comma_keycode = if has_latin(59, ',') { 59 } else { 51 };

        assert_eq!(latin_char_for_keycode(d_keycode), Some('d'));
        assert_eq!(latin_char_for_keycode(comma_keycode), Some(','));
    }

    #[test]
    fn test_ctrl_shortcut_table() {
        assert_eq!(ctrl_shortcut_action('s', false), Some("win.save"));
        assert_eq!(ctrl_shortcut_action('s', true), Some("win.save_as"));
        assert_eq!(ctrl_shortcut_action('r', false), Some("win.find_replace"));
        assert_eq!(
            ctrl_shortcut_action('p', true),
            Some("win.toggle_whitespace")
        );
        assert_eq!(
            ctrl_shortcut_action('l', true),
            Some("win.toggle_line_numbers")
        );
        assert_eq!(
            ctrl_shortcut_action('w', true),
            Some("win.toggle_soft_wrap")
        );
        assert_eq!(ctrl_shortcut_action(',', false), Some("win.preferences"));
        assert_eq!(ctrl_shortcut_action('\t', false), Some("win.next_tab"));
        assert_eq!(
            ctrl_shortcut_action('\t', true),
            Some("win.previous_tab")
        );
        assert_eq!(ctrl_shortcut_action('x', false), None);
    }
}
