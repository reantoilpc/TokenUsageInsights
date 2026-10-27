//! 透過 Windows Terminal 操作分頁（Windows 上的 WSL）：以 UI Automation 讀分頁標題、
//! 把視窗帶到前景，以 wt.exe 切換分頁與開新分頁。

use super::terminal::{Focus, Open, TitleMatch};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::process::Command;

/// 每支腳本的開頭：把第一個 Windows Terminal 視窗放進 `$window`，沒開就直接結束（沒有輸出）。
/// ponytail: 只看第一個視窗（UI Automation 依疊放順序列出，最上層的在前）。
const FIRST_WINDOW: &str = r#"$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$AE = [System.Windows.Automation.AutomationElement]
$byClass = New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, 'CASCADIA_HOSTING_WINDOW_CLASS')
$window = $AE::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $byClass)
if (-not $window) { exit 0 }
"#;

/// 每個分頁的標題，一行一個、依分頁順序。
const LIST_TABS: &str = r#"$isTab = New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::TabItem)
foreach ($tab in $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $isTab)) { $tab.Current.Name }"#;

/// 把視窗帶到最前面（縮到最小時先還原）。Windows 不讓背景程式搶前景，只會讓工作列閃爍；
/// 先送一個 Alt 鍵事件，讓這個程式算是剛收到輸入，SetForegroundWindow 才會生效。
const BRING_TO_FRONT: &str = r#"Add-Type -Namespace TokenInsights -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
[DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
'@
$hwnd = [IntPtr]$window.Current.NativeWindowHandle
if ([TokenInsights.Win]::IsIconic($hwnd)) { [void][TokenInsights.Win]::ShowWindow($hwnd, 9) }
[TokenInsights.Win]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
[TokenInsights.Win]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
[void][TokenInsights.Win]::SetForegroundWindow($hwnd)"#;

/// 以 Windows PowerShell 對第一個視窗執行腳本，成功時回傳標準輸出；沒有 powershell.exe
/// （不是 WSL）時回傳 None。
fn on_first_window(script: &str) -> Option<Vec<u8>> {
    // -EncodedCommand 要的是 UTF-16LE 的 base64：腳本不必經過 Windows 命令列的引號處理
    let script: Vec<u8> = format!("{FIRST_WINDOW}{script}")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
        .arg(STANDARD.encode(script))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
}

/// 分頁標題；沒有 powershell.exe 時回傳 None，Windows Terminal 沒開時是空的。
fn tab_titles() -> Option<Vec<String>> {
    let output = on_first_window(LIST_TABS)?;
    Some(
        String::from_utf8_lossy(&output)
            .lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect(),
    )
}

/// 執行 wt.exe；沒有 wt.exe 或執行失敗時回傳 false。
fn wt(args: &[&str]) -> bool {
    Command::new("wt.exe")
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// PATH 裡有 wt.exe（WSL 預設把 Windows 的 PATH 接在後面）。
fn has_wt_exe() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("wt.exe").is_file()))
}

/// 切到第一個標題符合名稱的分頁並帶到前景；序號就是它在第一個視窗裡的位置。
pub fn focus(name: Option<&str>, how: TitleMatch) -> Focus {
    let Some(titles) = tab_titles() else {
        return Focus::Unsupported;
    };
    // 沒有工作資料夾可比對：沒有名稱就找不到
    let Some(index) =
        name.and_then(|name| titles.iter().position(|title| how.matches(title, name)))
    else {
        return Focus::Missing;
    };
    if !wt(&["-w", "0", "focus-tab", "-t", &index.to_string()]) {
        return Focus::Unsupported;
    }
    // 分頁已經切過去了；帶不到前景也不算失敗
    on_first_window(BRING_TO_FRONT);
    Focus::Done
}

/// 在第一個視窗開新分頁，以 wsl.exe 切到資料夾後執行 `command`，agent 結束後留下一般的
/// shell。資料夾是 wsl.exe 的參數、不進指令文字；以 || 與 && 串接而不用 ;，因為 wt.exe
/// 會把 ; 當成下一個 wt 指令的開頭。
pub fn open_tab(cwd: &str, command: &str) -> Open {
    if !has_wt_exe() {
        return Open::Unsupported;
    }
    // ; 會讓 wt.exe 把資料夾名稱後半段當成另一個 wt 指令執行；" 與 \ 會破壞命令列的引號
    if cwd.contains([';', '"', '\\']) {
        return Open::UnsafeFolder;
    }
    let shell = format!("{command} || true && exec bash");
    if wt(&[
        "-w", "0", "new-tab", "wsl.exe", "--cd", cwd, "--", "bash", "-lic", &shell,
    ]) {
        Open::Done
    } else {
        Open::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假的 powershell.exe 測不出 UI Automation 腳本本身的問題，所以保留一個要在
    /// Windows Terminal 裡的 WSL 才能跑的唯讀測試：`cargo test -- --ignored real_windows_terminal`。
    #[test]
    #[ignore = "needs WSL inside Windows Terminal"]
    fn real_windows_terminal_lists_its_tab_titles() {
        let titles = tab_titles().expect("powershell.exe should run");
        assert!(
            !titles.is_empty(),
            "running inside Windows Terminal, so it has a tab"
        );
        assert!(titles.iter().all(|title| !title.ends_with('\r')));
    }
}
