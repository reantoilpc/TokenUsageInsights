//! 從卡片關閉 session，效果等同在 CLI 打 /exit 再關掉分頁：先關分頁讓 agent 自己收尾，
//! 沒結束才補送 SIGTERM；被 Ctrl+Z 暫停的舊程序收不到訊號，直接強制結束。

use super::{
    process::{self, Info, Signal},
    terminal, LiveSession,
};
use std::time::Duration;

/// 關分頁或送 SIGTERM 之後各等多久。實測 Claude Code 約 2.5 秒結束。
const WAIT: Duration = Duration::from_secs(5);

pub struct Closed {
    /// 程序都結束了；false 表示送了 SIGTERM 仍在執行（不會再強制結束）。
    pub ended: bool,
    pub tab_closed: bool,
}

pub enum Refusal {
    Missing,
    /// 工作中被關會中斷正在跑的回合。
    Working,
    /// 同一個資料夾有好幾個沒帶 id 的 Codex，分不出是哪個程序。
    CannotTell,
}

/// `copies`：同一個 session 的每一筆（每個程序一筆），最近更新的在前。
pub fn close(copies: &[LiveSession]) -> Result<Closed, Refusal> {
    let target = copies.first().ok_or(Refusal::Missing)?;
    if target.status == "working" {
        return Err(Refusal::Working);
    }
    if target.pids.is_empty() {
        return Err(Refusal::CannotTell);
    }
    // 只動確實是這個 agent 的程序：登記檔裡的 pid 可能已經被系統給了別的程式
    let (stopped, running): (Vec<Info>, Vec<Info>) = copies
        .iter()
        .flat_map(|copy| &copy.pids)
        .filter_map(|pid| process::inspect(*pid))
        .filter(|info| info.program == target.agent)
        .partition(|info| info.stopped);
    if stopped.is_empty() && running.is_empty() {
        return Err(Refusal::Missing);
    }

    // 只剩暫停中的程序時，分頁一樣要關
    let tab_closed = running
        .first()
        .or(stopped.first())
        .is_some_and(|info| terminal::close(target, info.tty.as_deref()));
    let mut remaining = if tab_closed {
        process::wait_until_ended(running, WAIT)
    } else {
        running
    };
    if !remaining.is_empty() {
        for info in &remaining {
            process::signal(info, Signal::Term);
        }
        remaining = process::wait_until_ended(remaining, WAIT);
    }
    for info in &stopped {
        process::signal(info, Signal::Kill);
    }
    let unkilled = process::wait_until_ended(stopped, WAIT);
    Ok(Closed {
        ended: remaining.is_empty() && unkilled.is_empty(),
        tab_closed,
    })
}
