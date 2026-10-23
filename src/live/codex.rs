//! 終端機裡的 Codex CLI session。Codex 沒有 session 登記資料夾，所以從系統裡正在
//! 執行的 `codex` 互動程序反推，再對應到 Codex 本機狀態資料庫裡的對話。

use super::{transcript, LiveSession};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
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
    let mut seen = HashSet::new();
    let mut sessions = Vec::new();
    for process in &processes {
        let thread = match &process.thread_id {
            Some(id) => threads.iter().find(|thread| &thread.id == id),
            // 沒有 id：取同資料夾最近更新的對話
            None => threads.iter().find(|thread| thread.cwd == process.cwd),
        };
        if let Some(thread) = thread.filter(|thread| seen.insert(thread.id.clone())) {
            sessions.push(into_live_session(thread));
        }
    }
    sessions
}

fn into_live_session(thread: &Thread) -> LiveSession {
    LiveSession {
        cwd: thread.cwd.clone(),
        agent: "codex",
        id: thread.id.clone(),
        name: thread.name.clone(),
        folder: super::folder_of(&thread.cwd),
        status: if turn_running(&thread.rollout_path) {
            "working"
        } else {
            "idle"
        },
        updated_at: thread.updated_at,
        last_prompt: None,
        prompt_at: None,
        progress: None,
    }
}

/// 紀錄尾端最後一個回合事件是「開始」就是工作中；「完成」或「中止」就是閒置。
fn turn_running(rollout: &Path) -> bool {
    let Ok(mut file) = File::open(rollout) else {
        return false;
    };
    let Some(len) = file.metadata().ok().map(|meta| meta.len()) else {
        return false;
    };
    let Some(tail) = transcript::read_tail(&mut file, len) else {
        return false;
    };
    tail.lines()
        .filter_map(|line| {
            ["task_started", "task_complete", "turn_aborted"]
                .into_iter()
                .find(|event| line.contains(&format!("\"{event}\"")))
        })
        .next_back()
        == Some("task_started")
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

fn is_uuid(text: &str) -> bool {
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
