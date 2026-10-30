//! agent 程序的作業系統操作：查程序現況（「進行中」與關閉都會用），以及關閉時送訊號、等它結束。
//! 每次送訊號前都重新確認 pid 還是同一個程序（啟動時間不變），避免 pid 被系統重複使用時誤殺。

use std::time::{Duration, Instant};

// 其他平台的 inspect 一律回傳 None，從不建立 Info
#[cfg_attr(not(unix), allow(dead_code))]
pub struct Info {
    pub pid: u32,
    /// 執行檔名稱（argv[0] 的最後一段），用來確認是 claude 或 codex。
    pub program: String,
    /// 啟動時間；不同程序拿到同一個 pid 時會不一樣。
    started: String,
    /// 被 Ctrl+Z 暫停：收不到 SIGTERM，只能強制結束。
    pub stopped: bool,
    /// 已經結束、只等上層程序收屍（zombie）。
    ended: bool,
    /// 所在的終端機，例如 `/dev/ttys003`（Linux 取 stdin，macOS 取控制終端機）。Ghostty 以它
    /// 找到要關的終端機；「進行中」以它判斷暫停的程序所在的分頁是否已有別的 agent 在跑。
    pub tty: Option<String>,
}

pub enum Signal {
    Term,
    Kill,
}

#[cfg(target_os = "linux")]
pub fn inspect(pid: u32) -> Option<Info> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // 第二欄是括號包住的程式名稱，裡面可能有空白與括號，所以從最後一個右括號之後算起
    let fields: Vec<&str> = stat
        .get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .collect();
    let state = fields.first()?;
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv0 = cmdline.split(|byte| *byte == 0).next()?;
    let tty = std::fs::read_link(format!("/proc/{pid}/fd/0"))
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|path| path.starts_with("/dev/pts/") || path.starts_with("/dev/tty"));
    Some(Info {
        pid,
        program: basename(&String::from_utf8_lossy(argv0)),
        started: fields.get(19)?.to_string(), // stat 的第 22 欄
        stopped: *state == "T",
        ended: *state == "Z",
        tty,
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn inspect(pid: u32) -> Option<Info> {
    // 用固定路徑：PATH 裡的 ps 可能是測試放的假指令（找 Codex 程序用的那個）
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "stat=,tty=,lstart=,args=", "-p", &pid.to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let words: Vec<&str> = text.split_whitespace().collect();
    // lstart 固定五個字，例如「Thu Oct 29 14:37:00 2026」
    let (state, tty, started, argv0) = (
        words.first()?,
        words.get(1)?,
        words.get(2..7)?,
        words.get(7)?,
    );
    Some(Info {
        pid,
        program: basename(argv0),
        started: started.join(" "),
        stopped: state.starts_with('T'),
        ended: state.starts_with('Z'),
        tty: tty.starts_with("tty").then(|| format!("/dev/{tty}")),
    })
}

// ponytail: 目前只支援 macOS／Linux；其他平台本來就沒有進行中的 session
#[cfg(not(unix))]
pub fn inspect(_pid: u32) -> Option<Info> {
    None
}

// 其他平台的 inspect 不讀程序資訊
#[cfg_attr(not(unix), allow(dead_code))]
fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// 送訊號；pid 已經換成別的程序或已結束時不送。
pub fn signal(expected: &Info, signal: Signal) {
    if !same_and_running(expected) {
        return;
    }
    #[cfg(unix)]
    {
        let number = match signal {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        if let Ok(pid) = i32::try_from(expected.pid) {
            unsafe { libc::kill(pid, number) };
        }
    }
    #[cfg(not(unix))]
    let _ = signal;
}

fn same_and_running(expected: &Info) -> bool {
    inspect(expected.pid).is_some_and(|now| now.started == expected.started && !now.ended)
}

/// 等這些程序結束，最多等 `timeout`；回傳時間到了還在的。
pub fn wait_until_ended(processes: Vec<Info>, timeout: Duration) -> Vec<Info> {
    let deadline = Instant::now() + timeout;
    let mut remaining = processes;
    loop {
        remaining.retain(same_and_running);
        if remaining.is_empty() || Instant::now() >= deadline {
            return remaining;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
