//! 透過 Ghostty 的 AppleScript 介面操作分頁（僅 macOS）。

use std::process::Command;

pub enum Focus {
    Done,
    /// 找不到符合的分頁。
    Missing,
    /// 沒有 osascript 或 Ghostty（例如不是 macOS）。
    Unsupported,
}

/// 標題與名稱的比對方式：Claude Code 的標題是「狀態符號 名稱」，Codex 是「名稱 | 資料夾」。
#[derive(Clone, Copy)]
pub enum Match {
    EndsWith,
    StartsWith,
}

pub struct Tab {
    pub cwd: String,
    pub title: String,
}

/// 找出「工作資料夾相同、且標題符合名稱」的第一個終端機並跳過去；名稱為空時只比對
/// 資料夾。資料夾、名稱與比對方式以參數傳入，不拼進腳本文字。
const FOCUS_SCRIPT: &str = r#"on run argv
  set wantDir to item 1 of argv
  set wantName to item 2 of argv
  set wantMode to item 3 of argv
  tell application "Ghostty"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with term in terminals of t
          set title to name of term
          if (working directory of term) is wantDir and (wantName is "" or (wantMode is "ends" and title ends with wantName) or (wantMode is "starts" and title starts with wantName)) then
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

/// 每個終端機一行：工作資料夾、tab、標題。一次取回所有終端機的屬性，比逐一走訪視窗與
/// 分頁快得多（每次存取都是一次 Apple Event：實測 0.16 秒對 0.57 秒）。
const LIST_SCRIPT: &str = r#"tell application "Ghostty"
  set dirs to working directory of terminals
  set titles to name of terminals
  set out to ""
  repeat with i from 1 to count of dirs
    set out to out & (item i of dirs) & tab & (item i of titles) & linefeed
  end repeat
  return out
end tell"#;

/// 目前所有 Ghostty 終端機的工作資料夾與標題；沒有 Ghostty 或不是 macOS 時回傳 None。
pub fn tabs() -> Option<Vec<Tab>> {
    let output = Command::new("osascript")
        .args(["-e", LIST_SCRIPT])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(cwd, title)| Tab {
                cwd: cwd.to_string(),
                title: title.to_string(),
            })
            .collect(),
    )
}

pub fn focus(cwd: &str, name: &str, how: Match) -> Focus {
    let mode = match how {
        Match::EndsWith => "ends",
        Match::StartsWith => "starts",
    };
    let Ok(output) = Command::new("osascript")
        .args(["-e", FOCUS_SCRIPT, cwd, name, mode])
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
