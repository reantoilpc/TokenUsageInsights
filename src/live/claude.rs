//! Claude Code 的 session：還開著的從它自己維護的登記檔讀，已結束的從對話紀錄讀。

use super::{
    folder_of,
    transcript::{self, Prompt},
    EndedSession, LiveSession, RECENT_LIMIT,
};
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

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

pub fn sessions(claude_dir: &Path) -> Vec<LiveSession> {
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
        .filter_map(|registered| into_live_session(registered, claude_dir))
        .collect()
}

fn into_live_session(registered: Registered, claude_dir: &Path) -> Option<LiveSession> {
    let transcript = transcript_path(claude_dir, &registered.cwd, &registered.session_id)
        .map(|path| transcript::read(&path, &transcript::CLAUDE))
        .unwrap_or_default();
    if transcript.background {
        return None; // 登記檔沒標出來，但對話紀錄說它是背景執行
    }
    let (last_prompt, prompt_at) = Prompt::split(transcript.last_prompt);
    Some(LiveSession {
        folder: folder_of(&registered.cwd),
        cwd: registered.cwd,
        pids: vec![registered.pid],
        agent: "claude",
        id: registered.session_id,
        // 分頁標題顯示的名稱：自己取的名字優先，沒取名就是 Claude Code 自動產生的標題；
        // 登記檔的 name 在沒取名時是「資料夾-代號」，對不上分頁標題，只當最後的備案
        name: transcript
            .title
            .or(transcript.ai_title)
            .or(registered.name.filter(|name| !name.is_empty())),
        // ponytail: 登記檔只有 busy／idle／shell，shell 視為工作中；其餘一律當閒置
        status: match registered.status.as_deref() {
            _ if transcript.asking => "waiting", // 正在等我回答，優先於登記檔的狀態
            Some("busy" | "shell") => "working",
            _ => "idle",
        },
        updated_at: registered.updated_at.unwrap_or(0),
        last_prompt,
        prompt_at,
        // recap 是我離開時寫的；之後我又下了指令，它描述的就是舊狀態，不顯示
        recap: transcript
            .recap
            .filter(|recap| match (recap.at, prompt_at) {
                (Some(recap_at), Some(prompt_at)) => recap_at > prompt_at,
                (Some(_), None) => true,
                (None, _) => false,
            })
            .map(|recap| recap.text),
        progress: transcript.progress,
        context: transcript.context,
    })
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

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // 與 updater 內私有的 is_process_alive 相同；複製而不改動上游檔案。
    // 登記檔的內容不可信：0（整個行程群組）和超過 i32::MAX 的值（轉成帶號整數會變成負數，
    // 例如 -1 代表所有行程）都不是單一行程，kill(pid, 0) 對它們會回報「活著」。
    let Some(pid) = i32::try_from(pid).ok().filter(|pid| *pid > 0) else {
        return false;
    };
    let res = unsafe { libc::kill(pid, 0) };
    res == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

// ponytail: 目前只支援 macOS／Linux；其他平台一律視為沒有進行中的 session
#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    false
}

/// 專案資料夾下的 `<session id>.jsonl`；子 agent 的紀錄放在 `<session id>/subagents/`，不在這一層。
pub fn recent(claude_dir: &Path, open: &HashSet<String>, since_ms: u64) -> Vec<EndedSession> {
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
            let transcript = transcript::read(&path, &transcript::CLAUDE);
            if transcript.background {
                return None;
            }
            let cwd = transcript.cwd?;
            let (last_prompt, prompt_at) = Prompt::split(transcript.last_prompt);
            Some(EndedSession {
                folder: folder_of(&cwd),
                cwd,
                agent: "claude",
                id,
                // 與進行中相同：自己取的名字優先，沒取名就用 Claude Code 自動產生的標題
                name: transcript.title.or(transcript.ai_title),
                last_prompt,
                prompt_at,
                updated_at,
            })
        })
        .take(RECENT_LIMIT)
        .collect()
}
