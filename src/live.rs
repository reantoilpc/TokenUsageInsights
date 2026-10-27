//! 「進行中」即時 session API：回傳目前還開著的 session 與最近結束的 session。
//! 即時狀態不進資料庫，每次請求即時讀取。

mod claude;
mod codex;
mod ghostty;
mod terminal;
mod transcript;
mod windows_terminal;

use axum::{
    http::{
        header::{HOST, ORIGIN},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashSet,
    path::Path,
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
struct EndedSession {
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

impl EndedSession {
    /// 接續用的固定指令；id 已經過 UUID 格式檢查，而且是清單上已知的 session。
    fn resume_command(&self) -> String {
        match self.agent {
            "codex" => format!("codex resume {}", self.id),
            _ => format!("claude --resume {}", self.id),
        }
    }
}

const RECENT_WINDOW_MS: u64 = 24 * 3600 * 1000;
const RECENT_LIMIT: usize = 10;

pub async fn get_live(headers: HeaderMap) -> Response {
    // 內容含我打過的指令與資料夾：不接受別的網域（DNS rebinding）把請求導到本機來讀
    if !loopback_host(&headers) {
        return failure(StatusCode::FORBIDDEN, "只接受以本機位址連線");
    }
    let (live, recent) = tokio::task::spawn_blocking(|| {
        let live = live_sessions();
        let recent = recent_sessions(&live);
        (live, recent)
    })
    .await
    .unwrap_or_default();
    Json(json!({ "live": live, "recent": recent })).into_response()
}

/// Claude Code 與 Codex 的 session 合在一起排序。
fn live_sessions() -> Vec<LiveSession> {
    let mut sessions = claude::sessions(&crate::db::get_claude_dir());
    sessions.extend(codex::sessions(&crate::db::get_codex_dir()));
    sessions.sort_by(|a, b| {
        status_rank(a.status)
            .cmp(&status_rank(b.status))
            .then(b.updated_at.cmp(&a.updated_at))
    });
    sessions
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

const NO_TERMINAL: &str = "無法操作終端機（支援 macOS 的 Ghostty 與 Windows 的 Windows Terminal）";

/// Host 必須是本機位址。只比對 Origin 與 Host 相等擋不住 DNS rebinding（攻擊者的網域
/// 指到 127.0.0.1 時兩者會一致），所以 Host 本身也必須是本機。
fn loopback_host(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let hostname = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().map(|ip| format!("[{ip}]")),
        None => host.split(':').next().map(str::to_string),
    };
    matches!(
        hostname.as_deref(),
        Some("localhost" | "127.0.0.1" | "[::1]")
    )
}

/// 只接受看板自己頁面發出的請求：Origin 必須是本機位址，且與請求的 Host 一致。
fn from_dashboard(headers: &HeaderMap) -> bool {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    let host = headers.get(HOST).and_then(|value| value.to_str().ok());
    matches!((origin, host), (Some(origin), Some(host)) if origin == format!("http://{host}"))
        && loopback_host(headers)
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
        find_session(&request).map(|session| terminal::focus(&session))
    })
    .await
    .ok()
    .flatten();
    match outcome {
        None => failure(StatusCode::NOT_FOUND, "找不到這個 session"),
        Some(terminal::Focus::Done) => Json(json!({ "focused": true })).into_response(),
        Some(terminal::Focus::Missing) => {
            failure(StatusCode::NOT_FOUND, "找不到這個 session 的分頁")
        }
        Some(terminal::Focus::Unsupported) => failure(StatusCode::NOT_IMPLEMENTED, NO_TERMINAL),
    }
}

/// 24 小時內結束、目前沒開著的 session，Claude Code 與 Codex 合在一起，最近的在前，最多 10 筆。
fn recent_sessions(live: &[LiveSession]) -> Vec<EndedSession> {
    let open: HashSet<String> = live.iter().map(|session| session.id.clone()).collect();
    // 重開同一段對話會換 session id，舊的紀錄還在：標題與資料夾都相同就當作同一段，不重複列出
    let open_titles: HashSet<(&str, &str)> = live
        .iter()
        .filter_map(|session| Some((session.cwd.as_str(), session.name.as_deref()?)))
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let since = now.saturating_sub(RECENT_WINDOW_MS);
    let mut recent = claude::recent(&crate::db::get_claude_dir(), &open, since);
    recent.retain(|item| {
        !item
            .name
            .as_deref()
            .is_some_and(|name| open_titles.contains(&(item.cwd.as_str(), name)))
    });
    recent.extend(codex::recent(&crate::db::get_codex_dir(), &open, since));
    recent.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    recent.truncate(RECENT_LIMIT);
    recent
}

/// 為已結束的 session 開新分頁接續。只能執行固定的 `claude --resume <id>` 或
/// `codex resume <id>`；id 必須是看板「最近結束」清單上的 session，資料夾由伺服器自己查。
pub async fn post_resume(headers: HeaderMap, Json(request): Json<SessionRef>) -> Response {
    if !from_dashboard(&headers) {
        return failure(StatusCode::FORBIDDEN, "請求來源不符");
    }
    if !codex::is_uuid(&request.id) {
        return failure(StatusCode::BAD_REQUEST, "session id 格式不正確");
    }
    let outcome = tokio::task::spawn_blocking(move || {
        let live = live_sessions();
        if live.iter().any(|session| session.id == request.id) {
            return Err((StatusCode::CONFLICT, "這個 session 還開著，請直接跳過去"));
        }
        let ended = recent_sessions(&live)
            .into_iter()
            .find(|item| item.agent == request.agent && item.id == request.id)
            .ok_or((StatusCode::NOT_FOUND, "找不到這個已結束的 session"))?;
        Ok(terminal::open_tab(&ended.cwd, &ended.resume_command()))
    })
    .await;
    match outcome {
        Ok(Ok(terminal::Open::Done)) => Json(json!({ "resumed": true })).into_response(),
        Ok(Ok(terminal::Open::Unsupported)) => failure(StatusCode::NOT_IMPLEMENTED, NO_TERMINAL),
        Ok(Ok(terminal::Open::UnsafeFolder)) => failure(
            StatusCode::NOT_IMPLEMENTED,
            "資料夾名稱含有 ; \" 或 \\，無法安全地在 Windows Terminal 開啟",
        ),
        Ok(Err((status, message))) => failure(status, message),
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "內部錯誤"),
    }
}
