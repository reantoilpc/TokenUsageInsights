//! 對話紀錄（jsonl，Claude Code 與 Codex 共用）的讀取：只讀尾端；尾端找不到時沿用上次
//! 讀到的，再不行才回頭掃整份檔案。結果以「檔案大小＋修改時間」為鍵快取，檔案沒變就不重讀。

use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

/// 對話紀錄可能超過數 MB，平常只讀最後這麼多位元組。
const TAIL_BYTES: u64 = 400_000;
/// 這些工具會停下來等人回答（選擇題、計畫核准）；一般的權限確認框紀錄裡看不出來。
const ASKING_TOOLS: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];
/// 這幾種「看起來像標籤」的使用者訊息，其實是人自己打的 shell 指令或斜線指令。
const TYPED_TAGS: [&str; 3] = ["<bash-input", "<command-name", "<command-message"];

#[derive(Clone, Default)]
pub struct Prompt {
    pub text: String,
    /// 該則訊息的時間（epoch 毫秒）。
    pub at: Option<u64>,
}

/// 待辦清單的進度：完成幾項／共幾項，以及目前進行中那一項的文字。
#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub current: Option<String>,
}

#[derive(Clone, Default)]
pub struct Transcript {
    pub last_prompt: Option<Prompt>,
    pub progress: Option<Progress>,
    /// 有選擇題或計畫正在等人回答。
    pub asking: bool,
    /// 最後一個回合是否還在進行；紀錄裡看不出來時為 None（只有 Codex 用得到）。
    pub running: Option<bool>,
}

/// 逐行掃描一份紀錄：Claude Code 與 Codex 的格式不同，各自提供。
pub type Scan = fn(&mut dyn Iterator<Item = String>) -> Transcript;

type Cache = Mutex<HashMap<PathBuf, (u64, SystemTime, Transcript)>>;
static CACHE: OnceLock<Cache> = OnceLock::new();

pub fn read(path: &Path, scan: Scan) -> Transcript {
    let Ok(meta) = std::fs::metadata(path) else {
        return Transcript::default();
    };
    let stamp = (
        meta.len(),
        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    );
    let cache = CACHE.get_or_init(Default::default);
    let previous = match cache.lock().unwrap().get(path) {
        Some((size, mtime, parsed)) if (*size, *mtime) == stamp => return parsed.clone(),
        Some((_, _, parsed)) => Some(parsed.clone()),
        None => None,
    };
    let parsed = parse(path, stamp.0, scan, previous.unwrap_or_default());
    cache
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), (stamp.0, stamp.1, parsed.clone()));
    parsed
}

/// 檔案最後約 400 KB 的文字；起點可能落在某一行中間，那一行不是完整的 JSON。
pub(super) fn read_tail(file: &mut File, len: u64) -> Option<String> {
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    file.by_ref()
        .take(TAIL_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse(path: &Path, len: u64, scan: Scan, previous: Transcript) -> Transcript {
    let Ok(mut file) = File::open(path) else {
        return Transcript::default();
    };
    let Some(text) = read_tail(&mut file, len) else {
        return Transcript::default();
    };
    let start = len.saturating_sub(TAIL_BYTES);
    let mut found = scan(&mut text.lines().map(str::to_string));
    // 尾端沒有的資訊：紀錄只會往後加，先沿用上次讀到的
    found.last_prompt = found.last_prompt.or(previous.last_prompt);
    found.running = found.running.or(previous.running);
    let missing =
        found.last_prompt.is_none() || (found.running.is_none() && previous.running.is_none());
    if start > 0 && missing && file.seek(SeekFrom::Start(0)).is_ok() {
        // 還是找不到（第一次讀一份很長的紀錄）：回頭掃整份檔案，只取指令與回合狀態。
        // ponytail: 待辦進度只看尾端；每次檔案變動都掃整檔太貴，更早的清單也已過時
        let mut lines = BufReader::new(file)
            .split(b'\n')
            .map_while(Result::ok)
            .map(|line| String::from_utf8_lossy(&line).into_owned());
        let full = scan(&mut lines);
        found.last_prompt = found.last_prompt.or(full.last_prompt);
        found.running = found.running.or(full.running);
    }
    found.progress = found.progress.filter(|progress| progress.total > 0);
    found
}

/// Claude Code 的對話紀錄。
pub fn claude_scan(lines: &mut dyn Iterator<Item = String>) -> Transcript {
    let mut found = Transcript::default();
    let mut unanswered: HashSet<String> = HashSet::new();
    for line in lines {
        let line = line.as_str();
        let maybe_prompt = line.contains("\"user\""); // 工具回傳也是 user 訊息
        let maybe_todos = line.contains("TodoWrite");
        let maybe_ask = ASKING_TOOLS.iter().any(|tool| line.contains(tool));
        if !maybe_prompt && !maybe_todos && !maybe_ask {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue; // 被截斷或壞掉的行
        };
        if let Some(text) = typed_text(&entry) {
            found.last_prompt = Some(Prompt {
                text,
                at: timestamp_ms(&entry),
            });
        }
        if let Some(progress) = todo_progress(&entry) {
            found.progress = Some(progress);
        }
        track_questions(&entry, &mut unanswered);
    }
    found.asking = !unanswered.is_empty();
    found
}

/// 記下還沒收到結果的選擇題／計畫核准呼叫；收到對應的 tool_result 就移除。
fn track_questions(entry: &Value, unanswered: &mut HashSet<String>) {
    let Some(parts) = entry["message"]["content"].as_array() else {
        return;
    };
    for part in parts {
        match part["type"].as_str() {
            Some("tool_use") if ASKING_TOOLS.iter().any(|tool| part["name"] == *tool) => {
                unanswered.extend(part["id"].as_str().map(str::to_string));
            }
            Some("tool_result") => {
                if let Some(id) = part["tool_use_id"].as_str() {
                    unanswered.remove(id);
                }
            }
            _ => {}
        }
    }
}

/// 一次 TodoWrite 呼叫就是一份完整的清單；最後一次呼叫代表目前的進度。
fn todo_progress(entry: &Value) -> Option<Progress> {
    let call = entry["message"]["content"]
        .as_array()?
        .iter()
        .rev()
        .find(|part| part["type"] == "tool_use" && part["name"] == "TodoWrite")?;
    let todos = call["input"]["todos"].as_array()?;
    let current = todos
        .iter()
        .find(|todo| todo["status"] == "in_progress")
        .and_then(|todo| {
            todo["activeForm"]
                .as_str()
                .filter(|text| !text.is_empty())
                .or_else(|| todo["content"].as_str())
        })
        .map(str::to_string);
    Some(Progress {
        done: todos
            .iter()
            .filter(|todo| todo["status"] == "completed")
            .count(),
        total: todos.len(),
        current,
    })
}

/// 只有「人自己打的字」才算指令：排除工具回傳、meta 訊息、子 agent、背景任務通知與指令輸出。
fn typed_text(entry: &Value) -> Option<String> {
    if entry["type"] != "user" || entry["isMeta"] == true || entry["isSidechain"] == true {
        return None;
    }
    let text = match &entry["message"]["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => {
            if parts.iter().any(|part| part["type"] == "tool_result") {
                return None;
            }
            let texts: Vec<&str> = parts
                .iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect();
            texts.join("\n")
        }
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with('<') && !TYPED_TAGS.iter().any(|tag| text.starts_with(tag)) {
        return None;
    }
    Some(display_text(text))
}

/// 把人打的 shell 指令與斜線指令還原成原本打的樣子。
fn display_text(text: &str) -> String {
    if let Some(command) = tag_content(text, "bash-input") {
        return format!("! {}", command.trim());
    }
    if let Some(name) = tag_content(text, "command-name") {
        let args = tag_content(text, "command-args").unwrap_or("").trim();
        return format!("{} {args}", name.trim()).trim().to_string();
    }
    text.to_string()
}

fn tag_content<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let rest = &text[text.find(&format!("<{tag}>"))? + tag.len() + 2..];
    Some(&rest[..rest.find(&format!("</{tag}>"))?])
}

pub(super) fn timestamp_ms(entry: &Value) -> Option<u64> {
    let parsed = chrono::DateTime::parse_from_rfc3339(entry["timestamp"].as_str()?).ok()?;
    u64::try_from(parsed.timestamp_millis()).ok()
}
