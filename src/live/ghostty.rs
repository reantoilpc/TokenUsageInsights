//! 透過 Ghostty 的 AppleScript 介面操作分頁（僅 macOS）。

use std::process::Command;

pub enum Focus {
    Done,
    /// 找不到符合的分頁。
    Missing,
    /// 沒有 osascript 或 Ghostty（例如不是 macOS）。
    Unsupported,
}

/// 找出「工作資料夾相同、且標題以 session 名稱結尾」的第一個終端機並跳過去；
/// 名稱為空時只比對資料夾。資料夾與名稱以參數傳入，不拼進腳本文字。
const FOCUS_SCRIPT: &str = r#"on run argv
  set wantDir to item 1 of argv
  set wantName to item 2 of argv
  tell application "Ghostty"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with term in terminals of t
          if (working directory of term) is wantDir and (wantName is "" or (name of term) ends with wantName) then
            focus term
            activate
            return "ok"
          end if
        end repeat
      end repeat
    end repeat
  end tell
  return "missing"
end run"#;

pub fn focus(cwd: &str, name: &str) -> Focus {
    let Ok(output) = Command::new("osascript")
        .args(["-e", FOCUS_SCRIPT, cwd, name])
        .output()
    else {
        return Focus::Unsupported;
    };
    if !output.status.success() {
        return Focus::Unsupported;
    }
    if String::from_utf8_lossy(&output.stdout).trim() == "ok" {
        Focus::Done
    } else {
        Focus::Missing
    }
}
