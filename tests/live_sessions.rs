//! 「進行中」即時 session 頁面的整合測試。
//!
//! 唯一的測試接縫是真正編好的程式的 HTTP API：啟動 binary、指到暫存資料夾，
//! 並把假的 `ps`／`lsof`／`osascript` 放在 PATH 最前面（腳本依劇本回答，並記錄收到的參數）。
//! 輔助程式刻意獨立於 `additional_sources.rs`，避免與上游對該檔的修改衝突。

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 一組放在 PATH 最前面的假指令。每個指令把收到的參數附加到 `<name>.args`，
/// 並輸出 `<name>.out` 的內容（若存在）。
struct FakeCommands {
    dir: PathBuf,
}

impl FakeCommands {
    fn new(root: &Path) -> Self {
        let dir = root.join("fakebin");
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    /// 建立假指令 `name`，執行時輸出 `stdout`。
    fn script(&self, name: &str, stdout: &str) {
        fs::write(self.dir.join(format!("{name}.out")), stdout).unwrap();
        let path = self.dir.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{dir}/{name}.args'\ncat '{dir}/{name}.out' 2>/dev/null\n",
                dir = self.dir.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// 假指令被呼叫時收到的參數，每次呼叫一行。
    fn calls(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.dir.join(format!("{name}.args")))
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn path_env(&self) -> String {
        format!("{}:/usr/bin:/bin", self.dir.display())
    }
}

/// 同一個測試行程內並行啟動多台伺服器；時間戳解析度不足以區分，故另加序號避免暫存資料夾重名。
static SERVER_COUNT: AtomicUsize = AtomicUsize::new(0);

struct LiveServer {
    child: Child,
    root: PathBuf,
    url: String,
    port: u16,
    fake: FakeCommands,
    /// 測試用的「還活著的程序」，結束時一併清掉。
    sleepers: Vec<Child>,
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        for sleeper in &mut self.sleepers {
            let _ = sleeper.kill();
            let _ = sleeper.wait();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl LiveServer {
    /// 啟動伺服器。`host` 為 `None` 時不設定 `HOST`，用來驗證預設綁定位址。
    /// 並行測試可能挑到同一個空閒埠而讓其中一台啟動失敗，所以失敗時換埠重試。
    async fn start(host: Option<&str>) -> Self {
        let mut last_log = String::new();
        for _ in 0..5 {
            match Self::try_start(host).await {
                Ok(server) => return server,
                Err(log) => last_log = log,
            }
        }
        panic!("startup failed: {last_log}");
    }

    async fn try_start(host: Option<&str>) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "insights-live-{}-{}-{}",
            std::process::id(),
            SERVER_COUNT.fetch_add(1, Ordering::SeqCst),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("insights")).unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let fake = FakeCommands::new(&root);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let log = fs::File::create(root.join("server.log")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_token-usage-insights"));
        command
            .env_clear()
            .env("HOME", &home)
            .env("PATH", fake.path_env())
            .env("INSIGHTS_DIR", root.join("insights"))
            .env("PORT", port.to_string())
            .env("CLAUDE_DIR", root.join("claude"))
            .env("CODEX_DIR", root.join("codex"))
            .arg("--no-auto-update")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
        if let Some(host) = host {
            command.env("HOST", host);
        }
        let mut server = Self {
            child: command.spawn().unwrap(),
            root,
            url: format!("http://127.0.0.1:{port}"),
            port,
            fake,
            sleepers: Vec::new(),
        };
        server.wait_until_ready().await?;
        Ok(server)
    }

    async fn wait_until_ready(&mut self) -> Result<(), String> {
        let client = client();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if client
                .get(format!("{}/api/version", self.url))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return Ok(());
            }
            if Instant::now() >= deadline || self.child.try_wait().unwrap().is_some() {
                return Err(fs::read_to_string(self.root.join("server.log")).unwrap_or_default());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// 啟動一個會睡著的程序，回傳它的 pid，代表「還活著的 Claude Code 程序」。
    fn alive_pid(&mut self) -> u32 {
        let child = Command::new("sleep").arg("600").spawn().unwrap();
        let pid = child.id();
        self.sleepers.push(child);
        pid
    }

    /// 回傳一個已經結束的程序的 pid，代表「已經關掉的 Claude Code」。
    fn dead_pid(&self) -> u32 {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    /// 寫入一筆 Claude Code 的 session 登記檔（`<CLAUDE_DIR>/sessions/<pid>.json`）。
    fn claude_session(&self, pid: u32, mut fields: Value) {
        fields["pid"] = json!(pid);
        let dir = self.root.join("claude/sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{pid}.json")), fields.to_string()).unwrap();
    }

    /// 寫入某個 session 的對話紀錄；`project_dir` 是 Claude Code 把工作資料夾路徑編碼後的資料夾名
    /// （非英數字元換成連字號，例如 `/work/erp` 對應 `-work-erp`）。
    fn transcript(&self, project_dir: &str, session_id: &str, lines: &[Value]) {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        self.transcript_raw(project_dir, session_id, &text);
    }

    fn transcript_raw(&self, project_dir: &str, session_id: &str, text: &str) {
        let dir = self.root.join("claude/projects").join(project_dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{session_id}.jsonl")), text).unwrap();
    }

    async fn get_text(&self, path: &str) -> (u16, String) {
        let response = client()
            .get(format!("{}{path}", self.url))
            .send()
            .await
            .unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }

    async fn live(&self) -> Value {
        client()
            .get(format!("{}/api/live", self.url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

#[tokio::test]
async fn live_api_starts_with_two_empty_lists() {
    let server = LiveServer::start(Some("127.0.0.1")).await;

    let live = server.live().await;

    assert_eq!(live["live"], serde_json::json!([]));
    assert_eq!(live["recent"], serde_json::json!([]));
}

#[tokio::test]
async fn live_page_is_served_with_its_empty_state_and_the_dashboard_links_to_it() {
    let server = LiveServer::start(Some("127.0.0.1")).await;

    let (status, page) = server.get_text("/live.html").await;
    assert_eq!(status, 200);
    assert!(page.contains("目前沒有進行中的 session"));

    let (status, home) = server.get_text("/").await;
    assert_eq!(status, 200);
    assert!(home.contains(r#"href="/live.html""#));
}

#[tokio::test]
async fn server_listens_on_loopback_only_unless_host_is_set() {
    let server = LiveServer::start(None).await;

    let log = fs::read_to_string(server.root.join("server.log")).unwrap();

    assert!(
        log.contains(&format!("127.0.0.1:{}", server.port)),
        "預設應只綁定本機，伺服器輸出：{log}"
    );
}

#[tokio::test]
async fn host_env_still_overrides_the_default_bind_address() {
    let server = LiveServer::start(Some("0.0.0.0")).await;

    let log = fs::read_to_string(server.root.join("server.log")).unwrap();

    assert!(log.contains(&format!("0.0.0.0:{}", server.port)), "{log}");
}

#[tokio::test]
async fn fake_commands_answer_from_the_script_and_record_their_arguments() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.fake.script("ps", "  101 ttys001 codex resume\n");

    let output = Command::new(server.fake.dir.join("ps"))
        .args(["-axo", "pid,tty,command"])
        .output()
        .unwrap();

    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "  101 ttys001 codex resume\n"
    );
    assert_eq!(server.fake.calls("ps"), vec!["-axo pid,tty,command"]);
}

fn registered(id: &str, cwd: &str, status: &str, updated_at: u64) -> Value {
    json!({"sessionId":id,"cwd":cwd,"status":status,"updatedAt":updated_at,"kind":"interactive","entrypoint":"cli"})
}

#[tokio::test]
async fn open_claude_sessions_are_listed_with_name_folder_and_status() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let (busy, idle) = (server.alive_pid(), server.alive_pid());
    let mut named = registered("s-busy", "/work/mango-hsu", "busy", 2000);
    named["name"] = json!("芒果許訂購系統");
    server.claude_session(busy, named);
    server.claude_session(idle, registered("s-idle", "/work/erp", "idle", 1000));

    let live = server.live().await;
    let items = live["live"].as_array().unwrap();

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["agent"], "claude");
    assert_eq!(items[0]["id"], "s-busy");
    assert_eq!(items[0]["name"], "芒果許訂購系統");
    assert_eq!(items[0]["folder"], "mango-hsu");
    assert_eq!(items[0]["status"], "working");
    assert_eq!(items[1]["id"], "s-idle");
    assert_eq!(items[1]["name"], Value::Null);
    assert_eq!(items[1]["folder"], "erp");
    assert_eq!(items[1]["status"], "idle");
}

#[tokio::test]
async fn a_registry_entry_whose_process_is_gone_is_not_live() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let alive = server.alive_pid();
    let gone = server.dead_pid();
    server.claude_session(alive, registered("s-alive", "/work/erp", "idle", 1000));
    server.claude_session(gone, registered("s-gone", "/work/old", "busy", 2000));

    let live = server.live().await;

    let ids: Vec<&Value> = live["live"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| &item["id"])
        .collect();
    assert_eq!(ids, vec!["s-alive"]);
}

#[tokio::test]
async fn background_runs_are_not_live_sessions() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let (interactive, headless) = (server.alive_pid(), server.alive_pid());
    server.claude_session(interactive, registered("s-cli", "/work/erp", "idle", 1000));
    let mut background = registered("s-p", "/work/erp", "busy", 3000);
    background["entrypoint"] = json!("sdk-cli");
    background["kind"] = json!("headless");
    server.claude_session(headless, background);

    let live = server.live().await;

    assert_eq!(live["live"].as_array().unwrap().len(), 1);
    assert_eq!(live["live"][0]["id"], "s-cli");
}

#[tokio::test]
async fn secret_key_files_next_to_the_registry_never_leak_into_the_response() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    server.claude_session(pid, registered("s-1", "/work/erp", "idle", 1000));
    fs::write(
        server.root.join(format!("claude/sessions/{pid}.key")),
        r#"{"sessionId":"s-secret","cwd":"/work/secret","status":"busy","pid":1,"secret":"SENTINEL-KEY"}"#,
    )
    .unwrap();

    let (_, body) = server.get_text("/api/live").await;

    assert!(!body.contains("SENTINEL-KEY"));
    assert!(!body.contains("s-secret"));
    assert!(body.contains("s-1"));
}

#[tokio::test]
async fn live_sessions_are_ordered_by_status_then_most_recent_activity() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    for (id, status, updated_at) in [
        ("idle-new", "idle", 9000),
        ("busy-old", "busy", 1000),
        ("idle-old", "idle", 2000),
        ("shell-new", "shell", 5000),
    ] {
        let pid = server.alive_pid();
        server.claude_session(pid, registered(id, "/work/erp", status, updated_at));
    }

    let live = server.live().await;

    let ids: Vec<&Value> = live["live"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| &item["id"])
        .collect();
    assert_eq!(ids, vec!["shell-new", "busy-old", "idle-new", "idle-old"]);
}

fn typed(text: &str, timestamp: &str) -> Value {
    json!({"type":"user","timestamp":timestamp,"message":{"role":"user","content":text}})
}

fn reply(timestamp: &str) -> Value {
    json!({"type":"assistant","timestamp":timestamp,"message":{"role":"assistant","content":[{"type":"text","text":"好的"}]}})
}

/// 啟動一個還活著的 session 並寫入對話紀錄，回傳伺服器。
async fn server_with_transcript(lines: &[Value]) -> LiveServer {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    server.claude_session(pid, registered("s-1", "/work/erp", "idle", 1000));
    server.transcript("-work-erp", "s-1", lines);
    server
}

#[tokio::test]
async fn card_carries_the_last_thing_the_person_typed_and_when() {
    let server = server_with_transcript(&[
        typed("先看一下登入頁", "2026-10-23T07:00:00.000Z"),
        reply("2026-10-23T07:00:05.000Z"),
        typed("幫我修好登入頁", "2026-10-23T08:00:00.000Z"),
        reply("2026-10-23T08:00:09.000Z"),
    ])
    .await;

    let live = server.live().await;

    assert_eq!(live["live"][0]["lastPrompt"], "幫我修好登入頁");
    assert_eq!(live["live"][0]["promptAt"], 1792742400000u64);
}

#[tokio::test]
async fn only_what_the_person_typed_counts_as_an_instruction() {
    let tool_result = json!({"type":"user","timestamp":"2026-10-23T08:01:00.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}});
    let meta = json!({"type":"user","isMeta":true,"timestamp":"2026-10-23T08:02:00.000Z","message":{"role":"user","content":"Caveat: local commands"}});
    let sidechain = json!({"type":"user","isSidechain":true,"timestamp":"2026-10-23T08:03:00.000Z","message":{"role":"user","content":"子 agent 的任務說明"}});
    let server = server_with_transcript(&[
        typed("幫我修好登入頁", "2026-10-23T08:00:00.000Z"),
        tool_result,
        meta,
        sidechain,
        typed(
            "<task-notification>done</task-notification>",
            "2026-10-23T08:04:00.000Z",
        ),
        typed("<bash-stdout>ok</bash-stdout>", "2026-10-23T08:04:30.000Z"),
        typed(
            "<local-command-stdout>x</local-command-stdout>",
            "2026-10-23T08:05:00.000Z",
        ),
    ])
    .await;

    let live = server.live().await;

    assert_eq!(live["live"][0]["lastPrompt"], "幫我修好登入頁");
    assert_eq!(live["live"][0]["promptAt"], 1792742400000u64);
}

#[tokio::test]
async fn shell_commands_slash_commands_and_pasted_images_count_as_typed() {
    let server = server_with_transcript(&[typed(
        "<bash-input>git status</bash-input>",
        "2026-10-23T08:00:00.000Z",
    )])
    .await;
    assert_eq!(server.live().await["live"][0]["lastPrompt"], "! git status");

    let server = server_with_transcript(&[typed(
        "<command-message>board</command-message>\n<command-name>/board</command-name>\n<command-args>now</command-args>",
        "2026-10-23T08:00:00.000Z",
    )])
    .await;
    assert_eq!(server.live().await["live"][0]["lastPrompt"], "/board now");

    let with_image = json!({"type":"user","timestamp":"2026-10-23T08:00:00.000Z","message":{"role":"user","content":[{"type":"image"},{"type":"text","text":"看這張圖"}]}});
    let server = server_with_transcript(&[with_image]).await;
    assert_eq!(server.live().await["live"][0]["lastPrompt"], "看這張圖");
}

#[tokio::test]
async fn a_session_without_a_transcript_is_still_listed_without_an_instruction() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    server.claude_session(pid, registered("s-new", "/work/erp", "idle", 1000));

    let live = server.live().await;

    assert_eq!(live["live"][0]["id"], "s-new");
    assert_eq!(live["live"][0]["lastPrompt"], Value::Null);
    assert_eq!(live["live"][0]["promptAt"], Value::Null);
}

/// 超過尾端讀取範圍的長對話：把一行很長的助理回覆重複寫進去。
fn filler(timestamp: &str, count: usize) -> String {
    let line = format!("{}\n", reply(timestamp)).replace("好的", &"長".repeat(1000));
    line.repeat(count)
}

#[tokio::test]
async fn a_cut_first_line_in_the_tail_window_does_not_break_parsing() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    server.claude_session(pid, registered("s-1", "/work/erp", "idle", 1000));
    // 約 500 KB 的填充，讓尾端 400 KB 的起點落在某一行的中間
    let text = format!(
        "{}{}\n",
        filler("2026-10-23T07:00:00.000Z", 170),
        typed("尾端裡的指令", "2026-10-23T08:00:00.000Z")
    );
    server.transcript_raw("-work-erp", "s-1", &text);

    let live = server.live().await;

    assert_eq!(live["live"][0]["lastPrompt"], "尾端裡的指令");
    assert_eq!(live["live"][0]["promptAt"], 1792742400000u64);
}

#[tokio::test]
async fn an_instruction_older_than_the_tail_window_is_found_by_scanning_the_whole_file() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    server.claude_session(pid, registered("s-1", "/work/erp", "idle", 1000));
    // 超過 4 MiB，而且最後一個指令在檔案最前面
    let text = format!(
        "{}\n{}",
        typed("很久以前下的指令", "2026-10-23T07:00:00.000Z"),
        filler("2026-10-23T08:00:00.000Z", 5000)
    );
    assert!(text.len() > 4 * 1024 * 1024);
    server.transcript_raw("-work-erp", "s-1", &text);

    let live = server.live().await;

    assert_eq!(live["live"][0]["lastPrompt"], "很久以前下的指令");
    assert_eq!(live["live"][0]["promptAt"], 1792738800000u64);
}

fn todo_write(todos: &[(&str, &str, &str)]) -> Value {
    let todos: Vec<Value> = todos
        .iter()
        .map(|(content, active, status)| json!({"content":content,"activeForm":active,"status":status}))
        .collect();
    json!({"type":"assistant","timestamp":"2026-10-23T08:00:10.000Z","message":{"role":"assistant","content":[
        {"type":"text","text":"更新待辦"},
        {"type":"tool_use","id":"t-todo","name":"TodoWrite","input":{"todos":todos}}
    ]}})
}

#[tokio::test]
async fn card_shows_how_many_todos_are_done_and_which_one_is_running() {
    let server = server_with_transcript(&[
        typed("做登入功能", "2026-10-23T08:00:00.000Z"),
        todo_write(&[
            ("寫測試", "寫測試中", "pending"),
            ("實作", "實作中", "in_progress"),
            ("設計", "設計中", "completed"),
        ]),
    ])
    .await;

    let live = server.live().await;

    assert_eq!(
        live["live"][0]["progress"],
        json!({"done":1,"total":3,"current":"實作中"})
    );
}

#[tokio::test]
async fn the_latest_todo_update_wins_and_a_finished_list_has_no_running_item() {
    let server = server_with_transcript(&[
        typed("做登入功能", "2026-10-23T08:00:00.000Z"),
        todo_write(&[
            ("設計", "設計中", "in_progress"),
            ("實作", "實作中", "pending"),
        ]),
        todo_write(&[
            ("設計", "設計中", "completed"),
            ("實作", "實作中", "completed"),
        ]),
    ])
    .await;

    let live = server.live().await;

    assert_eq!(
        live["live"][0]["progress"],
        json!({"done":2,"total":2,"current":null})
    );
}

#[tokio::test]
async fn a_session_without_todos_has_no_progress() {
    let server = server_with_transcript(&[typed("隨便問問", "2026-10-23T08:00:00.000Z")]).await;
    assert_eq!(server.live().await["live"][0]["progress"], Value::Null);

    let server = server_with_transcript(&[
        typed("做登入功能", "2026-10-23T08:00:00.000Z"),
        todo_write(&[("設計", "設計中", "in_progress")]),
        todo_write(&[]),
    ])
    .await;
    assert_eq!(server.live().await["live"][0]["progress"], Value::Null);
}
