//! session 所在的終端機分頁：先試 macOS 的 Ghostty，Ghostty 不支援時改試 Windows 的
//! Windows Terminal（WSL）。

use super::{ghostty, windows_terminal, LiveSession};

pub enum Focus {
    Done,
    /// 找不到符合的分頁。
    Missing,
    /// 沒有可用的終端機。
    Unsupported,
}

pub enum Open {
    Done,
    /// 沒有可用的終端機。
    Unsupported,
    /// 資料夾名稱含有 Windows Terminal 無法安全處理的字元。
    UnsafeFolder,
}

/// 標題與名稱的比對方式：Claude Code 的標題是「狀態符號 名稱」，Codex 是「名稱 | 資料夾」。
#[derive(Clone, Copy)]
pub enum TitleMatch {
    EndsWith,
    StartsWith,
}

impl TitleMatch {
    pub fn of(agent: &str) -> Self {
        if agent == "codex" {
            Self::StartsWith
        } else {
            Self::EndsWith
        }
    }

    /// 分頁標題是否屬於這個名稱，給不在 AppleScript 裡比對的終端機用。這些終端機沒有資料夾
    /// 可比對，所以比 Ghostty 的 FOCUS_SCRIPT 嚴格：Claude 的標題在狀態符號之後必須剛好是名稱。
    pub fn matches(self, title: &str, name: &str) -> bool {
        match self {
            Self::EndsWith => title
                .split_once(' ')
                .is_some_and(|(_, after_symbol)| after_symbol == name),
            Self::StartsWith => title.starts_with(&format!("{name} | ")),
        }
    }
}

pub fn focus(session: &LiveSession) -> Focus {
    let name = session.name.as_deref();
    let how = TitleMatch::of(session.agent);
    // Ghostty 的腳本以空字串表示沒有名稱，只比對資料夾
    match ghostty::focus(&session.cwd, name.unwrap_or(""), how) {
        Focus::Unsupported => windows_terminal::focus(name, how),
        outcome => outcome,
    }
}

/// 關掉 session 所在的分頁，回傳有沒有關到。agent 會收到掛斷訊號（SIGHUP），像 /exit 一樣收尾。
/// `tty` 是 agent 程序的控制終端機，只有 Ghostty 用得上。
pub fn close(session: &LiveSession, tty: Option<&str>) -> bool {
    let name = session.name.as_deref();
    let how = TitleMatch::of(session.agent);
    ghostty::close(tty, &session.cwd, name.unwrap_or(""), how)
        .unwrap_or_else(|| windows_terminal::close(name, how))
}

/// 開新分頁、切到資料夾並執行 `command`；agent 結束後留下一般的 shell。
pub fn open_tab(cwd: &str, command: &str) -> Open {
    if ghostty::open_tab(cwd, &format!("{command}\n")) {
        Open::Done
    } else {
        windows_terminal::open_tab(cwd, command)
    }
}
