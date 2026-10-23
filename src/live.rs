//! 「進行中」即時 session API：回傳目前還開著的 session 與最近結束的 session。
//! 即時狀態不進資料庫，每次請求即時讀取。

mod codex;
mod ghostty;
mod transcript;

use axum::{
    http::{
        header::{HOST, ORIGIN},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSession {
    /// 工作資料夾的完整路徑，只在伺服器內部用來跳轉與接續，不回給前端。
    #[serde(skip)]
    cwd: String,
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

/// 最近結束的 session（已經沒有程序，資料取自對話紀錄）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Recent {
    /// 工作資料夾的完整路徑，只在伺服器內部用來接續，不回給前端。
    #[serde(skip)]
    cwd: String,
    agent: &'static str,
    id: String,
    name: Option<String>,
    folder: String,
    last_prompt: Option<String>,
    prompt_at: Option<u64>,
    /// 最後活動的時間（epoch 毫秒），也就是結束的時間。
    updated_at: u64,
}

const RECENT_WINDOW_MS: u64 = 24 * 3600 * 1000;
const RECENT_LIMIT: usize = 10;

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
    let (live, recent) = tokio::task::spawn_blocking(|| {
        let live = live_sessions();
        let recent = recent_sessions(&live);
        (live, recent)
    })
    .await
    .unwrap_or_default();
    Json(json!({ "live": live, "recent": recent }))
}

/// Claude Code 與 Codex 的 session 合在一起排序。
fn live_sessions() -> Vec<LiveSession> {
    let mut sessions = claude_sessions(&crate::db::get_claude_dir());
    sessions.extend(codex::sessions(&crate::db::get_codex_dir()));
    sessions.sort_by(|a, b| {
        status_rank(a.status)
            .cmp(&status_rank(b.status))
            .then(b.updated_at.cmp(&a.updated_at))
    });
    sessions
}

fn claude_sessions(claude_dir: &Path) -> Vec<LiveSession> {
    let Ok(entries) = std::fs::read_dir(claude_dir.join("sessions")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        // 只讀 .json：同資料夾內的 .key 是機密，絕不開啟
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<Registered>(&text).ok())
        .filter(Registered::is_open)
        .map(|registered| into_live_session(registered, claude_dir))
        .collect()
}

fn into_live_session(registered: Registered, claude_dir: &Path) -> LiveSession {
    let folder = folder_of(&registered.cwd);
    let transcript = transcript_path(claude_dir, &registered.cwd, &registered.session_id)
        .map(|path| transcript::read(&path, transcript::claude_scan))
        .unwrap_or_default();
    let prompt = transcript.last_prompt;
    LiveSession {
        cwd: registered.cwd,
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

fn folder_of(cwd: &str) -> String {
    Path::new(cwd).file_name().map_or_else(
        || cwd.to_string(),
        |name| name.to_string_lossy().into_owned(),
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

/// 前端只回報要操作哪個 session；資料夾與名稱一律由伺服器自己查，不信任請求端。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRef {
    agent: String,
    id: String,
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

/// 只接受看板自己頁面發出的請求：Origin 必須是本機位址，且與請求的 Host 一致。
/// （只比對 Origin 與 Host 相等擋不住 DNS rebinding，所以 Host 也必須是本機。）
fn from_dashboard(headers: &HeaderMap) -> bool {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let (Some(origin), Some(host)) = (header(ORIGIN), header(HOST)) else {
        return false;
    };
    let hostname = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().map(|ip| format!("[{ip}]")),
        None => host.split(':').next().map(str::to_string),
    };
    origin == format!("http://{host}")
        && matches!(
            hostname.as_deref(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        )
}

fn find_session(request: &SessionRef) -> Option<LiveSession> {
    live_sessions()
        .into_iter()
        .find(|session| session.agent == request.agent && session.id == request.id)
}

pub async fn post_focus(headers: HeaderMap, Json(request): Json<SessionRef>) -> Response {
    if !from_dashboard(&headers) {
        return failure(StatusCode::FORBIDDEN, "請求來源不符");
    }
    let outcome = tokio::task::spawn_blocking(move || {
        let session = find_session(&request)?;
        let how = if session.agent == "codex" {
            ghostty::Match::StartsWith
        } else {
            ghostty::Match::EndsWith
        };
        Some(ghostty::focus(
            &session.cwd,
            session.name.as_deref().unwrap_or(""),
            how,
        ))
    })
    .await
    .ok()
    .flatten();
    match outcome {
        None => failure(StatusCode::NOT_FOUND, "找不到這個 session"),
        Some(ghostty::Focus::Done) => Json(json!({ "focused": true })).into_response(),
        Some(ghostty::Focus::Missing) => {
            failure(StatusCode::NOT_FOUND, "找不到這個 session 的分頁")
        }
        Some(ghostty::Focus::Unsupported) => failure(
            StatusCode::NOT_IMPLEMENTED,
            "無法操作 Ghostty（僅支援 macOS 上的 Ghostty）",
        ),
    }
}

/// 24 小時內結束、目前沒開著的 session，Claude Code 與 Codex 合在一起，最近的在前，最多 10 筆。
fn recent_sessions(live: &[LiveSession]) -> Vec<Recent> {
    let open: HashSet<String> = live.iter().map(|session| session.id.clone()).collect();
    // 重開同一段對話會換 session id，舊的紀錄還在：標題與資料夾都相同就當作同一段，不重複列出
    let continued: HashSet<(&str, &str)> = live
        .iter()
        .filter_map(|session| Some((session.cwd.as_str(), session.name.as_deref()?)))
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let since = now.saturating_sub(RECENT_WINDOW_MS);
    let mut recent = claude_recent(&crate::db::get_claude_dir(), &open, since);
    recent.retain(|item| {
        !item
            .name
            .as_deref()
            .is_some_and(|name| continued.contains(&(item.cwd.as_str(), name)))
    });
    recent.extend(codex::recent(&crate::db::get_codex_dir(), &open, since));
    recent.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    recent.truncate(RECENT_LIMIT);
    recent
}

/// 專案資料夾下的 `<session id>.jsonl`；子 agent 的紀錄放在 `<session id>/subagents/`，不在這一層。
fn claude_recent(claude_dir: &Path, open: &HashSet<String>, since_ms: u64) -> Vec<Recent> {
    let Ok(projects) = std::fs::read_dir(claude_dir.join("projects")) else {
        return Vec::new();
    };
    let mut files: Vec<(u64, PathBuf)> = projects
        .flatten()
        .filter_map(|project| std::fs::read_dir(project.path()).ok())
        .flat_map(|files| files.flatten())
        .filter(|file| file.path().extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|file| {
            let modified = file.metadata().ok()?.modified().ok()?;
            let ms = modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
            Some((ms, file.path()))
        })
        .filter(|(ms, path)| {
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("");
            *ms >= since_ms && !open.contains(id)
        })
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files
        .into_iter()
        .filter_map(|(updated_at, path)| {
            let id = path.file_stem()?.to_str()?.to_string();
            let transcript = transcript::read(&path, transcript::claude_scan);
            if transcript.background {
                return None;
            }
            let cwd = transcript.cwd?;
            let prompt = transcript.last_prompt;
            Some(Recent {
                folder: folder_of(&cwd),
                cwd,
                agent: "claude",
                id,
                name: transcript.title,
                prompt_at: prompt.as_ref().and_then(|prompt| prompt.at),
                last_prompt: prompt.map(|prompt| prompt.text),
                updated_at,
            })
        })
        .take(RECENT_LIMIT)
        .collect()
}
