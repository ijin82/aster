//! Восстановление несохранённых безымянных вкладок после сбоя.
//!
//! Черновики хранятся отдельно от `settings.json`, чтобы большой текст не
//! раздувал настройки и не переписывался на каждое изменение. Файл
//! `~/.local/share/aster/recovery.json` удаляется при штатном закрытии
//! приложения; если приложение упало, он остаётся и читается при следующем
//! запуске.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Один несохранённый черновик.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryDraft {
    /// Полное содержимое буфера.
    pub content: String,
    /// Строка курсора на момент сохранения черновика.
    #[serde(default)]
    pub cursor_line: usize,
    /// Колонка курсора на момент сохранения черновика.
    #[serde(default)]
    pub cursor_column: usize,
}

/// Путь к файлу аварийного восстановления.
fn recovery_path() -> Option<PathBuf> {
    dirs::data_dir().map(|mut path| {
        path.push("aster");
        path.push("recovery.json");
        path
    })
}

/// Загружает черновики, оставшиеся после прошлого сбоя.
///
/// Повреждённый файл не должен мешать запуску приложения: в этом случае
/// возвращается пустой список.
pub fn load() -> Vec<RecoveryDraft> {
    let Some(path) = recovery_path() else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<RecoveryDraft>>(&content).unwrap_or_default()
}

/// Сохраняет черновики атомарно.
///
/// Пустой список удаляет файл: значит, несохранённых безымянных вкладок нет.
pub fn save(drafts: &[RecoveryDraft]) {
    let Some(path) = recovery_path() else {
        return;
    };

    if drafts.is_empty() {
        let _ = fs::remove_file(path);
        return;
    }

    let Some(parent) = path.parent() else {
        return;
    };
    let _ = fs::create_dir_all(parent);

    let Ok(json) = serde_json::to_string_pretty(drafts) else {
        return;
    };

    let temp_path = path.with_extension("json.tmp");
    if fs::write(&temp_path, json).is_err() {
        let _ = fs::remove_file(temp_path);
        return;
    }
    if fs::rename(&temp_path, &path).is_err() {
        let _ = fs::remove_file(temp_path);
    }
}

/// Удаляет файл восстановления после штатного завершения приложения.
pub fn clear() {
    if let Some(path) = recovery_path() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_draft_roundtrip() {
        let drafts = vec![RecoveryDraft {
            content: "черновик\nвторая строка".to_string(),
            cursor_line: 1,
            cursor_column: 4,
        }];

        let json = serde_json::to_string(&drafts).expect("serialize drafts");
        let restored: Vec<RecoveryDraft> = serde_json::from_str(&json).expect("deserialize drafts");

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].content, drafts[0].content);
        assert_eq!(restored[0].cursor_line, 1);
        assert_eq!(restored[0].cursor_column, 4);
    }
}
