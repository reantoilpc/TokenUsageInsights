//! Claude Code 對話紀錄（jsonl）的讀取：只讀尾端；尾端找不到時回頭掃整份檔案。
//! 結果以「檔案大小＋修改時間」為鍵快取，檔案沒變就不重讀。

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
/// 這幾種「看起來像標籤」的使用者訊息，其實是人自己打的 shell 指令或斜線指令。
/// 這些工具會停下來等人回答（選擇題、計畫核准）；一般的權限確認框紀錄裡看不出來。
const ASKING_TOOLS: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];
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
}

type Cache = Mutex<HashMap<PathBuf, (u64, SystemTime, Transcript)>>;
static CACHE: OnceLock<Cache> = OnceLock::new();

pub fn read(path: &Path) -> Transcript {
    let Ok(meta) = std::fs::metadata(path) else {
        return Transcript::default();
    };
    let stamp = (
        meta.len(),
        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    );
    let cache = CACHE.get_or_init(Default::default);
    if let Some((size, mtime, parsed)) = cache.lock().unwrap().get(path) {
        if (*size, *mtime) == stamp {
            return parsed.clone();
        }
    }
    let parsed = parse(path, stamp.0);
    cache
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), (stamp.0, stamp.1, parsed.clone()));
    parsed
}

fn parse(path: &Path, len: u64) -> Transcript {
    let Ok(mut file) = File::open(path) else {
        return Transcript::default();
    };
    let start = len.saturating_sub(TAIL_BYTES);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err()
        || file
            .by_ref()
            .take(TAIL_BYTES)
            .read_to_end(&mut bytes)
            .is_err()
    {
        return Transcript::default();
    }
    // 尾端的起點可能落在某一行中間；那一行不是完整的 JSON，解析失敗時會被略過
    let text = String::from_utf8_lossy(&bytes);
    let mut found = scan(text.lines());
    if found.last_prompt.is_none() && start > 0 && file.seek(SeekFrom::Start(0)).is_ok() {
        // 最後一個指令早在尾端範圍之前：回頭掃整份檔案找指令。
        // ponytail: 待辦進度只看尾端；每次檔案變動都掃整檔太貴，更早的清單也已過時
        let lines = BufReader::new(file)
            .split(b'\n')
            .map_while(Result::ok)
            .map(|line| String::from_utf8_lossy(&line).into_owned());
        found.last_prompt = scan(lines).last_prompt;
    }
    found.progress = found.progress.filter(|progress| progress.total > 0);
    found
}

fn scan<S: AsRef<str>>(lines: impl Iterator<Item = S>) -> Transcript {
    let mut found = Transcript::default();
    let mut unanswered: HashSet<String> = HashSet::new();
    for line in lines {
        let line = line.as_ref();
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

fn timestamp_ms(entry: &Value) -> Option<u64> {
    let parsed = chrono::DateTime::parse_from_rfc3339(entry["timestamp"].as_str()?).ok()?;
    u64::try_from(parsed.timestamp_millis()).ok()
}
