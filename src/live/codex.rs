//! 終端機裡的 Codex CLI session。Codex 沒有 session 登記資料夾，所以從系統裡正在
//! 執行的 `codex` 互動程序反推，再對應到 Codex 本機狀態資料庫裡的對話。

use super::{
    ghostty,
    transcript::{self, timestamp_ms, Progress, Prompt, Transcript},
    LiveSession,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
};

/// 這些子命令不是互動介面（常駐程式、非互動執行、登入等）。
const NON_INTERACTIVE: [&str; 7] = [
    "app-server",
    "exec-server",
    "exec",
    "mcp",
    "mcp-server",
    "login",
    "logout",
];

const TURN_EVENTS: [&str; 3] = ["task_started", "task_complete", "turn_aborted"];

struct Process {
    cwd: String,
    /// `codex resume <id>` 帶的對話 id。
    thread_id: Option<String>,
}

struct Thread {
    id: String,
    rollout_path: PathBuf,
    cwd: String,
    name: Option<String>,
    updated_at: u64,
}

pub fn sessions(codex_dir: &Path) -> Vec<LiveSession> {
    let processes = processes();
    if processes.is_empty() {
        return Vec::new();
    }
    let threads = threads(codex_dir); // 最近更新的在前
    let mut open: Vec<&Thread> = Vec::new();

    // `codex resume <id>` 已經指明是哪個對話
    for id in processes
        .iter()
        .filter_map(|process| process.thread_id.as_ref())
    {
        if let Some(thread) = threads.iter().find(|thread| &thread.id == id) {
            if !open.iter().any(|known| known.id == thread.id) {
                open.push(thread);
            }
        }
    }

    // 沒有 id 的程序：只知道資料夾，要從該資料夾的對話裡挑
    let mut by_folder: Vec<(&str, usize)> = Vec::new();
    for process in processes
        .iter()
        .filter(|process| process.thread_id.is_none())
    {
        match by_folder.iter_mut().find(|(cwd, _)| *cwd == process.cwd) {
            Some((_, count)) => *count += 1,
            None => by_folder.push((&process.cwd, 1)),
        }
    }
    let mut tabs = None; // 要用到才向 Ghostty 問，而且一次請求最多問一次
    for (cwd, count) in by_folder {
        let candidates: Vec<&Thread> = threads
            .iter()
            .filter(|thread| thread.cwd == cwd && !open.iter().any(|known| known.id == thread.id))
            .collect();
        let chosen = if candidates.len() > 1 {
            // 同資料夾有好幾個對話：看哪些有對應的分頁（標題是「名稱 | 資料夾」）
            let tabs = tabs.get_or_insert_with(ghostty::tabs);
            let titled: Vec<&Thread> = candidates
                .iter()
                .copied()
                .filter(|thread| {
                    thread.name.as_deref().is_some_and(|name| {
                        tabs.iter()
                            .flatten()
                            .any(|tab| tab.cwd == cwd && tab.title.starts_with(name))
                    })
                })
                .collect();
            // ponytail: 讀不到分頁標題時，只顯示最近更新的那一個
            if titled.is_empty() {
                candidates[..1].to_vec()
            } else {
                titled
            }
        } else {
            candidates
        };
        open.extend(chosen.into_iter().take(count));
    }
    open.into_iter().map(into_live_session).collect()
}

fn into_live_session(thread: &Thread) -> LiveSession {
    let transcript = transcript::read(&thread.rollout_path, scan);
    let prompt = transcript.last_prompt;
    LiveSession {
        cwd: thread.cwd.clone(),
        agent: "codex",
        id: thread.id.clone(),
        name: thread.name.clone(),
        folder: super::folder_of(&thread.cwd),
        status: match (transcript.running == Some(true), transcript.asking) {
            (true, _) => "working", // 問了問題之後 Codex 還會繼續做事，這時仍是工作中
            (false, true) => "waiting",
            (false, false) => "idle",
        },
        updated_at: thread.updated_at,
        prompt_at: prompt.as_ref().and_then(|prompt| prompt.at),
        last_prompt: prompt.map(|prompt| prompt.text),
        progress: transcript.progress,
    }
}

/// Codex 的對話紀錄（rollout）。
fn scan(lines: &mut dyn Iterator<Item = String>) -> Transcript {
    let mut found = Transcript::default();
    for line in lines {
        let maybe_turn = TURN_EVENTS.iter().any(|event| line.contains(event));
        let maybe_prompt = ["UserMessage", "user_message", "\"user\""]
            .iter()
            .any(|mark| line.contains(mark));
        let maybe_call = ["update_plan", "request_user_input_async"]
            .iter()
            .any(|name| line.contains(name));
        if !maybe_turn && !maybe_prompt && !maybe_call {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue; // 被截斷或壞掉的行
        };
        match (entry["type"].as_str(), entry["payload"]["type"].as_str()) {
            (Some("event_msg"), Some("task_started")) => found.running = Some(true),
            (Some("event_msg"), Some("task_complete" | "turn_aborted")) => {
                found.running = Some(false);
            }
            (Some("response_item"), Some("function_call")) => {
                match entry["payload"]["name"].as_str() {
                    Some("update_plan") => found.progress = plan_progress(&entry["payload"]),
                    // 問了問題：直到我下一則訊息之前都算還沒回答
                    Some("request_user_input_async") => found.asking = true,
                    _ => {}
                }
            }
            _ => {
                if let Some(text) = typed_text(&entry) {
                    found.last_prompt = Some(Prompt {
                        text,
                        at: timestamp_ms(&entry),
                    });
                    found.asking = false;
                }
            }
        }
    }
    found
}

/// 計畫工具的參數是一段 JSON 字串：`{"plan":[{"step":"…","status":"pending|in_progress|completed"}]}`。
fn plan_progress(call: &Value) -> Option<Progress> {
    let arguments: Value = serde_json::from_str(call["arguments"].as_str()?).ok()?;
    let steps = arguments["plan"].as_array()?;
    Some(Progress {
        done: steps
            .iter()
            .filter(|step| step["status"] == "completed")
            .count(),
        total: steps.len(),
        current: steps
            .iter()
            .find(|step| step["status"] == "in_progress")
            .and_then(|step| step["step"].as_str())
            .map(str::to_string),
    })
}

/// 人打的字。目前版本記在 item_completed 的 UserMessage，較舊版本記在 user_message
/// 事件；response_item 裡的 user 訊息也有，但其中混有 Codex 自動注入的內容
/// （AGENTS.md 指示、環境資訊），那些不算。
fn typed_text(entry: &Value) -> Option<String> {
    let payload = &entry["payload"];
    let text = match (entry["type"].as_str(), payload["type"].as_str()) {
        (Some("event_msg"), Some("user_message")) => payload["message"].as_str()?.to_string(),
        (Some("event_msg"), Some("item_completed")) if payload["item"]["type"] == "UserMessage" => {
            join_text(&payload["item"]["content"])
        }
        (Some("response_item"), Some("message")) if payload["role"] == "user" => {
            let text = join_text(&payload["content"]);
            if text.starts_with('<') || text.starts_with("# AGENTS.md") {
                return None;
            }
            text
        }
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn join_text(parts: &Value) -> String {
    let texts: Vec<&str> = parts
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect();
    texts.join("\n")
}

/// 有終端機、而且是 Codex 互動介面的程序。
fn processes() -> Vec<Process> {
    let Ok(output) = Command::new("ps")
        .args(["-axo", "pid=,tty=,command="])
        .output()
    else {
        return Vec::new();
    };
    let listed: Vec<(u32, Option<String>)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(interactive_codex)
        .collect();
    let cwds = cwds(&listed.iter().map(|(pid, _)| *pid).collect::<Vec<_>>());
    listed
        .into_iter()
        .filter_map(|(pid, thread_id)| {
            Some(Process {
                cwd: cwds.get(&pid)?.clone(),
                thread_id,
            })
        })
        .collect()
}

/// 解析 `ps` 的一行；若是終端機裡的 Codex 互動程序，回傳 pid 與 `resume` 帶的對話 id。
fn interactive_codex(line: &str) -> Option<(u32, Option<String>)> {
    let mut words = line.split_whitespace();
    let pid = words.next()?.parse().ok()?;
    if words.next()?.starts_with('?') {
        return None; // 沒有終端機：常駐程式、VS Code 擴充套件、桌面版
    }
    let program = words.next()?;
    if program.rsplit('/').next() != Some("codex") {
        return None;
    }
    let args: Vec<&str> = words.collect();
    if args
        .first()
        .is_some_and(|arg| NON_INTERACTIVE.contains(arg))
    {
        return None;
    }
    let resumed = args
        .iter()
        .position(|arg| *arg == "resume")
        .and_then(|at| args.get(at + 1))
        .filter(|id| is_uuid(id))
        .map(|id| id.to_string());
    Some((pid, resumed))
}

pub(super) fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
        && text.matches('-').count() == 4
}

/// 各程序目前的工作資料夾。
fn cwds(pids: &[u32]) -> HashMap<u32, String> {
    let mut cwds = HashMap::new();
    if pids.is_empty() {
        return cwds;
    }
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let Ok(output) = Command::new("lsof")
        .args(["-a", "-d", "cwd", "-Fpn", "-p", &list])
        .output()
    else {
        return cwds;
    };
    let mut current = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(pid) = line.strip_prefix('p') {
            current = pid.parse::<u32>().ok();
        } else if let (Some(path), Some(pid)) = (line.strip_prefix('n'), current) {
            cwds.insert(pid, path.to_string());
        }
    }
    cwds
}

/// Codex 的對話清單：只收使用者自己的對話，不收子 agent，也不收已封存的。
fn threads(codex_dir: &Path) -> Vec<Thread> {
    let Some(conn) = state_db(codex_dir).and_then(|path| open_read_only(&path)) else {
        return Vec::new();
    };
    let Ok(mut statement) = conn.prepare(
        "SELECT id, rollout_path, cwd, name, updated_at_ms FROM threads \
         WHERE archived = 0 AND (thread_source IS NULL OR thread_source = 'user') \
         ORDER BY updated_at_ms DESC",
    ) else {
        return Vec::new();
    };
    let rows = statement.query_map([], |row| {
        Ok(Thread {
            id: row.get(0)?,
            rollout_path: PathBuf::from(row.get::<_, Option<String>>(1)?.unwrap_or_default()),
            cwd: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            name: row
                .get::<_, Option<String>>(3)?
                .filter(|name| !name.is_empty()),
            updated_at: row.get::<_, Option<u64>>(4)?.unwrap_or(0),
        })
    });
    rows.map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// 版本號最大的 `state_<n>.sqlite`。
fn state_db(codex_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(codex_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u32>()
                .ok()?;
            Some((version, entry.path()))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

/// Codex 的資料庫是 WAL 模式：一般唯讀開啟才看得到最新的資料，immutable 會漏掉還沒
/// 合併進主檔的變更。
fn open_read_only(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .or_else(|_| {
            // ponytail: 一般唯讀開不了時退而用 immutable，可能看到略舊的資料
            let escaped = path
                .to_string_lossy()
                .replace('%', "%25")
                .replace('?', "%3F")
                .replace('#', "%23")
                .replace(' ', "%20");
            Connection::open_with_flags(
                format!("file:{escaped}?immutable=1"),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )
        })
        .ok()
}

/// 24 小時內結束、來自終端機版（codex-tui）的對話。來源欄位不可靠（終端機接續 VS Code
/// 建立的對話時仍是 vscode），所以用 originator 判斷；舊版資料庫沒有這個欄位時就不列。
pub fn recent(codex_dir: &Path, open: &HashSet<String>, since_ms: u64) -> Vec<super::Recent> {
    let Some(conn) = state_db(codex_dir).and_then(|path| open_read_only(&path)) else {
        return Vec::new();
    };
    let Ok(mut statement) = conn.prepare(
        "SELECT id, rollout_path, cwd, name, updated_at_ms FROM threads \
         WHERE archived = 0 AND (thread_source IS NULL OR thread_source = 'user') \
         AND originator = 'codex-tui' AND updated_at_ms >= ?1 ORDER BY updated_at_ms DESC",
    ) else {
        return Vec::new();
    };
    let rows = statement.query_map([since_ms], |row| {
        Ok(Thread {
            id: row.get(0)?,
            rollout_path: PathBuf::from(row.get::<_, Option<String>>(1)?.unwrap_or_default()),
            cwd: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            name: row
                .get::<_, Option<String>>(3)?
                .filter(|name| !name.is_empty()),
            updated_at: row.get::<_, Option<u64>>(4)?.unwrap_or(0),
        })
    });
    rows.map(|rows| rows.flatten().collect::<Vec<Thread>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|thread| !open.contains(&thread.id))
        .map(|thread| {
            let prompt = transcript::read(&thread.rollout_path, scan).last_prompt;
            super::Recent {
                folder: super::folder_of(&thread.cwd),
                cwd: thread.cwd,
                agent: "codex",
                id: thread.id,
                name: thread.name,
                prompt_at: prompt.as_ref().and_then(|prompt| prompt.at),
                last_prompt: prompt.map(|prompt| prompt.text),
                updated_at: thread.updated_at,
            }
        })
        .collect()
}
