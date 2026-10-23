//! 「進行中」即時 session API：回傳目前還開著的 session 與最近結束的 session。
//! 即時狀態不進資料庫，每次請求即時讀取。

mod transcript;

use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSession {
    agent: &'static str,
    id: String,
    name: Option<String>,
    folder: String,
    status: &'static str,
    updated_at: u64,
    /// 我最後打的一個指令與它的時間（epoch 毫秒）。
    last_prompt: Option<String>,
    prompt_at: Option<u64>,
    /// 待辦清單進度；沒有清單時為 null。
    progress: Option<transcript::Progress>,
}

/// Claude Code 自己維護的 session 登記檔（`<CLAUDE_DIR>/sessions/<pid>.json`）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registered {
    pid: u32,
    session_id: String,
    cwd: String,
    name: Option<String>,
    status: Option<String>,
    updated_at: Option<u64>,
    kind: Option<String>,
    entrypoint: Option<String>,
}

impl Registered {
    /// 互動式、還在執行的 session；`claude -p` 這類背景執行不算。
    fn is_open(&self) -> bool {
        self.kind
            .as_deref()
            .is_none_or(|kind| kind == "interactive")
            && self.entrypoint.as_deref() != Some("sdk-cli")
            && pid_alive(self.pid)
    }
}

pub async fn get_live() -> Json<Value> {
    let live = tokio::task::spawn_blocking(|| claude_sessions(&crate::db::get_claude_dir()))
        .await
        .unwrap_or_default();
    Json(json!({ "live": live, "recent": [] }))
}

fn claude_sessions(claude_dir: &Path) -> Vec<LiveSession> {
    let Ok(entries) = std::fs::read_dir(claude_dir.join("sessions")) else {
        return Vec::new();
    };
    let mut sessions: Vec<LiveSession> = entries
        .flatten()
        .map(|entry| entry.path())
        // 只讀 .json：同資料夾內的 .key 是機密，絕不開啟
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<Registered>(&text).ok())
        .filter(Registered::is_open)
        .map(|registered| into_live_session(registered, claude_dir))
        .collect();
    sessions.sort_by(|a, b| {
        status_rank(a.status)
            .cmp(&status_rank(b.status))
            .then(b.updated_at.cmp(&a.updated_at))
    });
    sessions
}

fn into_live_session(registered: Registered, claude_dir: &Path) -> LiveSession {
    let folder = Path::new(&registered.cwd).file_name().map_or_else(
        || registered.cwd.clone(),
        |name| name.to_string_lossy().into_owned(),
    );
    let transcript = transcript_path(claude_dir, &registered.cwd, &registered.session_id)
        .map(|path| transcript::read(&path))
        .unwrap_or_default();
    let prompt = transcript.last_prompt;
    LiveSession {
        agent: "claude",
        id: registered.session_id,
        name: registered.name.filter(|name| !name.is_empty()),
        folder,
        // ponytail: 登記檔只有 busy／idle／shell，shell 視為工作中；其餘一律當閒置
        status: match registered.status.as_deref() {
            _ if transcript.asking => "waiting", // 正在等我回答，優先於登記檔的狀態
            Some("busy" | "shell") => "working",
            _ => "idle",
        },
        updated_at: registered.updated_at.unwrap_or(0),
        prompt_at: prompt.as_ref().and_then(|prompt| prompt.at),
        last_prompt: prompt.map(|prompt| prompt.text),
        progress: transcript.progress,
    }
}

/// 對話紀錄：`projects/<工作資料夾路徑，非英數字元換成連字號>/<session id>.jsonl`。
fn transcript_path(claude_dir: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id == ".." {
        return None; // 登記檔的內容不可信，不讓它指到別的路徑
    }
    let project: String = cwd
        .chars()
        .flat_map(|c| {
            let dashes = if c.is_ascii_alphanumeric() {
                0
            } else {
                c.len_utf16()
            };
            // 與 Claude Code 一致：以 UTF-16 單位計，補增字元會變成兩個連字號
            std::iter::repeat_n('-', dashes).chain(c.is_ascii_alphanumeric().then_some(c))
        })
        .collect();
    Some(
        claude_dir
            .join("projects")
            .join(project)
            .join(format!("{session_id}.jsonl")),
    )
}

/// 排序用：等你回覆 > 工作中 > 閒置。
fn status_rank(status: &str) -> u8 {
    match status {
        "waiting" => 0,
        "working" => 1,
        _ => 2,
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // 與 updater 內私有的 is_process_alive 相同；複製而不改動上游檔案
    let res = unsafe { libc::kill(pid as libc::pid_t, 0) };
    res == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

// ponytail: 目前只支援 macOS／Linux；其他平台一律視為沒有進行中的 session
#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    false
}
