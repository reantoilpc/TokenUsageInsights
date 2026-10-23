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
}

pub struct Tab {
    pub cwd: String,
    pub title: String,
}

/// 找出「工作資料夾相同、且標題符合名稱」的第一個終端機並跳過去；名稱為空時只比對
/// 資料夾。資料夾、名稱與比對方式以參數傳入，不拼進腳本文字。Codex 的標題是
/// 「名稱 | 資料夾」，名稱後面要接分隔符號，「fix」才不會對到「fix login」。
/// Ghostty 沒開時直接回報找不到：對沒開的程式下 `tell` 會把它啟動起來。
const FOCUS_SCRIPT: &str = r#"on run argv
  if application "Ghostty" is not running then return "missing"
  set wantDir to item 1 of argv
  set wantName to item 2 of argv
  set wantMode to item 3 of argv
  tell application "Ghostty"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with term in terminals of t
          set title to name of term
          if (working directory of term) is wantDir and (wantName is "" or (wantMode is "ends" and title ends with wantName) or (wantMode is "starts" and title starts with (wantName & " | "))) then
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

/// 每個終端機一行：工作資料夾、定位字元、標題。定位字元要在 `tell` 區塊外先定義：
/// 在 Ghostty 的 `tell` 裡，`tab` 指的是它的 tab 類別，會被轉成「tab」這個字。
/// 一次取回所有終端機的屬性，比逐一走訪視窗與
/// 分頁快得多（每次存取都是一次 Apple Event：實測 0.16 秒對 0.57 秒）。
const LIST_SCRIPT: &str = r#"if application "Ghostty" is not running then return ""
set separator to ASCII character 9
tell application "Ghostty"
  set dirs to working directory of terminals
  set titles to name of terminals
  set out to ""
  repeat with i from 1 to count of dirs
    set out to out & (item i of dirs) & separator & (item i of titles) & linefeed
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

pub fn focus(cwd: &str, name: &str, how: TitleMatch) -> Focus {
    let mode = match how {
        TitleMatch::EndsWith => "ends",
        TitleMatch::StartsWith => "starts",
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

/// 在最前面的視窗開一個新分頁（沒有視窗就開新視窗）：資料夾以初始工作資料夾設定，
/// 指令以初始輸入打進 shell，所以結束後分頁仍是一般的 shell。兩者都以參數傳入。
const NEW_TAB_SCRIPT: &str = r#"on run argv
  set wantDir to item 1 of argv
  set wantInput to item 2 of argv
  tell application "Ghostty"
    set cfg to new surface configuration
    set initial working directory of cfg to wantDir
    set initial input of cfg to wantInput
    if (count of windows) is 0 then
      new window with configuration cfg
    else
      new tab in front window with configuration cfg
    end if
    activate
  end tell
  return "ok"
end run"#;

/// 開新分頁並輸入 `input`；沒有 Ghostty 或不是 macOS 時回傳 false。
pub fn open_tab(cwd: &str, input: &str) -> bool {
    Command::new("osascript")
        .args(["-e", NEW_TAB_SCRIPT, cwd, input])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假的 osascript 測不出 AppleScript 本身的問題（例如在 `tell` 區塊裡 `tab` 會被當成
    /// Ghostty 的 tab 類別而不是定位字元），所以保留一個要真的 Ghostty 才能跑的測試：
    /// `cargo test -- --ignored real_ghostty`。
    #[test]
    #[ignore = "needs Ghostty running on macOS"]
    fn real_ghostty_listing_comes_back_with_real_separators() {
        let tabs = tabs().expect("osascript should run");
        assert!(!tabs.is_empty(), "Ghostty is running, so it has terminals");
        assert!(tabs.iter().all(|tab| tab.cwd.starts_with('/')));
    }
}
