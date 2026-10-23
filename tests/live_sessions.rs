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

/// 一組放在 PATH 最前面的假指令。每個指令把收到的參數附加到 `<name>.args`
/// （一次呼叫一行，參數之間以 `\x1f` 分隔），並輸出 `<name>.out` 的內容（若存在）。
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
        self.write_script(name, stdout, 0);
    }

    /// 建立執行就失敗（結束碼 1）的假指令，模擬沒有 Ghostty 或不是 macOS。
    fn failing(&self, name: &str) {
        self.write_script(name, "", 1);
    }

    fn write_script(&self, name: &str, stdout: &str, exit_code: i32) {
        fs::write(self.dir.join(format!("{name}.out")), stdout).unwrap();
        let path = self.dir.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\n{{ for a in \"$@\"; do printf '%s\\037' \"$a\" | tr '\\n' ' '; done; echo; }} >> '{dir}/{name}.args'\ncat '{dir}/{name}.out' 2>/dev/null\nexit {exit_code}\n",
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

    /// 假指令被呼叫時收到的參數：每次呼叫一組。
    fn calls(&self, name: &str) -> Vec<Vec<String>> {
        fs::read_to_string(self.dir.join(format!("{name}.args")))
            .map(|text| {
                text.lines()
                    .map(|line| {
                        // 每個參數後面都接一個分隔字元，最後多出來的空字串要去掉；空參數則保留
                        let mut args: Vec<String> =
                            line.split('\x1f').map(str::to_string).collect();
                        args.pop();
                        args
                    })
                    .collect()
            })
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
        // 預設沒有任何終端機程序，測試才不會讀到這台電腦真正在跑的 Codex
        fake.script("ps", "");
        fake.script("lsof", "");

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

    /// 在假的 Codex 狀態資料庫加入一個對話，並寫入它的紀錄檔。
    /// `turn`：`running` ＝最後一個回合還沒結束；`done` ＝已完成；`aborted` ＝被中止。
    fn codex_thread(
        &self,
        id: &str,
        cwd: &str,
        name: Option<&str>,
        updated_at_ms: i64,
        thread_source: &str,
        turn: &str,
    ) {
        let dir = self.root.join("codex");
        fs::create_dir_all(dir.join("sessions")).unwrap();
        let rollout = dir.join(format!("sessions/rollout-{id}.jsonl"));
        let event = |kind: &str| json!({"timestamp":"2026-10-23T08:00:00.000Z","type":"event_msg","payload":{"type":kind,"turn_id":"t1"}});
        let events = match turn {
            "running" => vec![event("task_started")],
            "done" => vec![event("task_started"), event("task_complete")],
            "aborted" => vec![event("task_started"), event("turn_aborted")],
            _ => vec![],
        };
        let text: String = events.iter().map(|line| format!("{line}\n")).collect();
        fs::write(&rollout, text).unwrap();
        let conn = rusqlite::Connection::open(dir.join("state_5.sqlite")).unwrap();
        conn.execute(
            "create table if not exists threads (id text primary key, rollout_path text, cwd text, name text, title text, updated_at_ms integer, thread_source text, archived integer default 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into threads (id, rollout_path, cwd, name, title, updated_at_ms, thread_source) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, rollout.to_str().unwrap(), cwd, name, "第一則訊息", updated_at_ms, thread_source],
        )
        .unwrap();
    }

    /// 用指定的內容取代某個 Codex 對話的紀錄檔。
    fn codex_rollout(&self, id: &str, lines: &[Value]) {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(
            self.root.join(format!("codex/sessions/rollout-{id}.jsonl")),
            text,
        )
        .unwrap();
    }

    /// 讓假的 `ps` 與 `lsof` 回報這些終端機裡的程序：(pid, tty, 指令列, 工作資料夾)。
    fn processes(&self, rows: &[(u32, &str, &str, &str)]) {
        let ps: String = rows
            .iter()
            .map(|(pid, tty, command, _)| format!("{pid} {tty} {command}\n"))
            .collect();
        let lsof: String = rows
            .iter()
            .map(|(pid, _, _, cwd)| format!("p{pid}\nn{cwd}\n"))
            .collect();
        self.fake.script("ps", &ps);
        self.fake.script("lsof", &lsof);
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

    /// 以完全自訂的標頭送出 POST；回傳狀態碼與（若是 JSON 的）回應內容。
    async fn post_raw(&self, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
        let mut request = client()
            .post(format!("{}{path}", self.url))
            .body(body.to_string());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// 像看板自己的頁面那樣送出 JSON POST：來源是伺服器自己的位址。
    async fn post_json(&self, path: &str, body: Value) -> (u16, Value) {
        self.post_raw(
            path,
            &[("Origin", &self.url), ("Content-Type", "application/json")],
            &body.to_string(),
        )
        .await
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
    assert_eq!(
        server.fake.calls("ps"),
        vec![vec!["-axo".to_string(), "pid,tty,command".to_string()]]
    );
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

fn tool_call(name: &str, id: &str) -> Value {
    json!({"type":"assistant","timestamp":"2026-10-23T08:00:20.000Z","message":{"role":"assistant","content":[
        {"type":"tool_use","id":id,"name":name,"input":{}}
    ]}})
}

fn tool_answer(id: &str) -> Value {
    json!({"type":"user","timestamp":"2026-10-23T08:00:40.000Z","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":id,"content":"ok"}
    ]}})
}

#[tokio::test]
async fn a_question_waiting_for_the_person_marks_the_card_as_waiting_and_puts_it_first() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let (asking, busy) = (server.alive_pid(), server.alive_pid());
    server.claude_session(asking, registered("s-ask", "/work/erp", "idle", 1000));
    server.transcript(
        "-work-erp",
        "s-ask",
        &[
            typed("做登入功能", "2026-10-23T08:00:00.000Z"),
            tool_call("AskUserQuestion", "q1"),
        ],
    );
    server.claude_session(busy, registered("s-busy", "/work/mango", "busy", 9000));

    let live = server.live().await;

    assert_eq!(live["live"][0]["id"], "s-ask");
    assert_eq!(live["live"][0]["status"], "waiting");
    assert_eq!(live["live"][1]["id"], "s-busy");
    assert_eq!(live["live"][1]["status"], "working");
}

#[tokio::test]
async fn a_plan_waiting_for_approval_is_also_waiting() {
    let server = server_with_transcript(&[
        typed("規劃一下", "2026-10-23T08:00:00.000Z"),
        tool_call("ExitPlanMode", "p1"),
    ])
    .await;

    assert_eq!(server.live().await["live"][0]["status"], "waiting");
}

#[tokio::test]
async fn once_the_question_is_answered_the_card_returns_to_its_registered_status() {
    let server = server_with_transcript(&[
        typed("做登入功能", "2026-10-23T08:00:00.000Z"),
        tool_call("AskUserQuestion", "q1"),
        tool_answer("q1"),
    ])
    .await;

    assert_eq!(server.live().await["live"][0]["status"], "idle");
}

#[tokio::test]
async fn a_slow_ordinary_tool_call_is_not_guessed_to_be_waiting() {
    let server = server_with_transcript(&[
        typed("跑測試", "2026-10-23T08:00:00.000Z"),
        tool_call("Bash", "b1"),
    ])
    .await;

    assert_eq!(server.live().await["live"][0]["status"], "idle");
}

const FOCUS: &str = "/api/live/focus";

/// 一個還活著、名稱為「芒果許」、在 /work/mango-hsu 的 Claude Code session（id 為 s-1）。
async fn server_with_named_session() -> LiveServer {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    let mut session = registered("s-1", "/work/mango-hsu", "idle", 1000);
    session["name"] = json!("芒果許");
    server.claude_session(pid, session);
    server.fake.script("osascript", "ok\n");
    server
}

#[tokio::test]
async fn clicking_a_card_asks_ghostty_for_the_tab_in_that_folder_titled_with_the_session_name() {
    let server = server_with_named_session().await;

    let (status, body) = server
        .post_json(FOCUS, json!({"agent":"claude","id":"s-1"}))
        .await;

    assert_eq!(status, 200);
    assert_eq!(body["focused"], true);
    let calls = server.fake.calls("osascript");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][0], "-e");
    assert!(calls[0][1].contains("focus"));
    assert_eq!(calls[0][2..], ["/work/mango-hsu", "芒果許", "ends"]);
}

#[tokio::test]
async fn a_session_without_a_name_is_matched_by_folder_only() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    server.claude_session(pid, registered("s-1", "/work/erp", "idle", 1000));
    server.fake.script("osascript", "ok\n");

    let (status, _) = server
        .post_json(FOCUS, json!({"agent":"claude","id":"s-1"}))
        .await;

    assert_eq!(status, 200);
    let calls = server.fake.calls("osascript");
    assert_eq!(calls[0][2..], ["/work/erp", "", "ends"]);
}

#[tokio::test]
async fn when_no_tab_matches_the_page_is_told_so_and_nothing_new_is_opened() {
    let server = server_with_named_session().await;
    server.fake.script("osascript", "missing\n");

    let (status, body) = server
        .post_json(FOCUS, json!({"agent":"claude","id":"s-1"}))
        .await;

    assert_eq!(status, 404);
    assert_eq!(body["message"], "找不到這個 session 的分頁");
    assert_eq!(server.fake.calls("osascript").len(), 1);
}

#[tokio::test]
async fn without_ghostty_the_answer_is_a_clear_unsupported_message() {
    let server = server_with_named_session().await;
    server.fake.failing("osascript");

    let (status, body) = server
        .post_json(FOCUS, json!({"agent":"claude","id":"s-1"}))
        .await;

    assert_eq!(status, 501);
    assert!(body["message"].as_str().unwrap().contains("Ghostty"));
}

#[tokio::test]
async fn focus_requests_that_do_not_come_from_the_dashboard_itself_are_refused() {
    let server = server_with_named_session().await;
    let body = json!({"agent":"claude","id":"s-1"}).to_string();
    let json_type = ("Content-Type", "application/json");
    let port = server.port;
    let rebinding_origin = format!("http://evil.example:{port}");
    let other_port_origin = format!("http://127.0.0.1:{}", port.wrapping_add(1));
    let own_origin = server.url.clone();
    let rebinding_host = format!("evil.example:{port}");

    let attempts: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("沒有 Origin", vec![json_type]),
        (
            "別的網站",
            vec![json_type, ("Origin", "https://evil.example")],
        ),
        (
            "本機但埠不同",
            vec![json_type, ("Origin", &other_port_origin)],
        ),
        (
            "DNS rebinding：Host 與 Origin 一致但不是本機",
            vec![
                json_type,
                ("Origin", &rebinding_origin),
                ("Host", &rebinding_host),
            ],
        ),
        (
            "內容類型不是 JSON",
            vec![("Origin", &own_origin), ("Content-Type", "text/plain")],
        ),
    ];

    for (label, headers) in attempts {
        let (status, _) = server.post_raw(FOCUS, &headers, &body).await;
        assert!(
            (400..500).contains(&status),
            "{label} 應被拒絕，卻回傳 {status}"
        );
    }
    assert!(server.fake.calls("osascript").is_empty());
}

#[tokio::test]
async fn focus_only_accepts_an_agent_and_a_known_session_id() {
    let server = server_with_named_session().await;

    for body in [
        json!({"agent":"claude","id":"does-not-exist"}),
        json!({"agent":"vim","id":"s-1"}),
        // 請求端不能自己指定資料夾、標題或指令
        json!({"agent":"claude","id":"s-1","cwd":"/etc"}),
        json!({"agent":"claude","id":"s-1","command":"rm -rf /"}),
    ] {
        let (status, _) = server.post_json(FOCUS, body.clone()).await;
        assert!(
            (400..500).contains(&status),
            "{body} 應被拒絕，卻回傳 {status}"
        );
    }
    assert!(server.fake.calls("osascript").is_empty());
}

const T1: &str = "01a12339-3ac8-7211-9bea-d289dde24cd3";
const T2: &str = "01a12f46-3b09-7c03-860f-137f0a2de48f";

#[tokio::test]
async fn terminal_codex_sessions_show_name_folder_and_status() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(
        T1,
        "/work/mango-hsu",
        Some("比較模式"),
        2000,
        "user",
        "running",
    );
    server.codex_thread(T2, "/work/erp", None, 1000, "user", "done");
    server.processes(&[
        (101, "ttys003", "codex", "/work/mango-hsu"),
        (102, "ttys004", &format!("codex resume {T2}"), "/work/erp"),
    ]);

    let live = server.live().await;
    let items = live["live"].as_array().unwrap();

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["agent"], "codex");
    assert_eq!(items[0]["id"], T1);
    assert_eq!(items[0]["name"], "比較模式");
    assert_eq!(items[0]["folder"], "mango-hsu");
    assert_eq!(items[0]["status"], "working");
    assert_eq!(items[1]["id"], T2);
    assert_eq!(items[1]["name"], Value::Null);
    assert_eq!(items[1]["folder"], "erp");
    assert_eq!(items[1]["status"], "idle");
}

#[tokio::test]
async fn an_aborted_turn_is_idle_not_working() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", None, 1000, "user", "aborted");
    server.processes(&[(101, "ttys003", "codex", "/work/erp")]);

    assert_eq!(server.live().await["live"][0]["status"], "idle");
}

#[tokio::test]
async fn codex_processes_that_are_not_terminal_sessions_are_ignored() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", None, 1000, "user", "done");
    server.processes(&[
        (101, "ttys003", "codex", "/work/erp"),
        // 常駐程式、VS Code 擴充套件、桌面版：沒有終端機
        (
            102,
            "??",
            "/Users/me/.codex/packages/bin/codex app-server --listen unix:// --managed-daemon",
            "/work/erp",
        ),
        (
            103,
            "??",
            "/Users/me/.vscode/extensions/openai.chatgpt/bin/codex -c x=1 app-server",
            "/work/erp",
        ),
        // 有終端機但不是互動介面
        (
            104,
            "ttys005",
            "codex app-server --listen stdio",
            "/work/erp",
        ),
        (105, "ttys006", "codex exec fix-the-bug", "/work/erp"),
        (106, "ttys007", "vim codex-notes.md", "/work/erp"),
    ]);

    let live = server.live().await;

    assert_eq!(live["live"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn without_an_id_the_latest_real_conversation_in_the_folder_is_used() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", Some("舊的"), 1000, "user", "done");
    server.codex_thread(T2, "/work/erp", Some("新的"), 2000, "user", "done");
    server.codex_thread(
        "01a1-sub",
        "/work/erp",
        Some("子 agent"),
        9000,
        "subagent",
        "running",
    );
    server.codex_thread(
        "01a1-other",
        "/work/other",
        Some("別的資料夾"),
        9500,
        "user",
        "done",
    );
    server.processes(&[(101, "ttys003", "codex", "/work/erp")]);

    let live = server.live().await;

    assert_eq!(live["live"].as_array().unwrap().len(), 1);
    assert_eq!(live["live"][0]["name"], "新的");
}

#[tokio::test]
async fn an_id_that_matches_no_conversation_shows_no_card() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", None, 1000, "user", "done");
    server.processes(&[(101, "ttys003", &format!("codex resume {T2}"), "/work/erp")]);

    assert_eq!(server.live().await["live"], json!([]));
}

#[tokio::test]
async fn two_processes_in_one_folder_without_ids_show_a_single_card_for_now() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", Some("唯一一張"), 1000, "user", "done");
    server.processes(&[
        (101, "ttys003", "codex", "/work/erp"),
        (102, "ttys004", "codex", "/work/erp"),
    ]);

    assert_eq!(server.live().await["live"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn codex_and_claude_cards_are_ordered_together() {
    let mut server = LiveServer::start(Some("127.0.0.1")).await;
    let pid = server.alive_pid();
    server.claude_session(pid, registered("s-idle", "/work/claude", "idle", 5000));
    server.codex_thread(
        T1,
        "/work/codex",
        Some("工作中的 Codex"),
        1000,
        "user",
        "running",
    );
    server.processes(&[(101, "ttys003", "codex", "/work/codex")]);

    let live = server.live().await;

    assert_eq!(live["live"][0]["agent"], "codex");
    assert_eq!(live["live"][0]["status"], "working");
    assert_eq!(live["live"][1]["agent"], "claude");
    assert_eq!(live["live"][1]["status"], "idle");
}

fn codex_event(timestamp: &str, payload: Value) -> Value {
    json!({"timestamp":timestamp,"type":"event_msg","payload":payload})
}

/// 目前版本的 Codex：使用者訊息以 item_completed 的 UserMessage 記錄。
fn codex_user_item(timestamp: &str, text: &str) -> Value {
    codex_event(
        timestamp,
        json!({"type":"item_completed","item":{"type":"UserMessage","id":"u1","content":[{"type":"text","text":text}]}}),
    )
}

/// 較舊版本的 Codex：event_msg 的 user_message。
fn codex_user_event(timestamp: &str, text: &str) -> Value {
    codex_event(timestamp, json!({"type":"user_message","message":text}))
}

/// response_item 形式的使用者訊息（含系統自動注入的內容也是這種形式）。
fn codex_user_response(timestamp: &str, parts: &[&str]) -> Value {
    let content: Vec<Value> = parts
        .iter()
        .map(|text| json!({"type":"input_text","text":text}))
        .collect();
    json!({"timestamp":timestamp,"type":"response_item","payload":{"type":"message","role":"user","content":content}})
}

async fn server_with_codex_rollout(lines: &[Value]) -> LiveServer {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", None, 1000, "user", "done");
    server.codex_rollout(T1, lines);
    server.processes(&[(101, "ttys003", "codex", "/work/erp")]);
    server
}

#[tokio::test]
async fn codex_card_shows_the_last_instruction_and_when_for_every_message_format() {
    for message in [
        codex_user_item("2026-10-23T08:00:00.000Z", "幫我修好登入頁"),
        codex_user_event("2026-10-23T08:00:00.000Z", "幫我修好登入頁"),
        codex_user_response("2026-10-23T08:00:00.000Z", &["幫我修好登入頁"]),
    ] {
        let server = server_with_codex_rollout(&[
            codex_user_item("2026-10-23T07:00:00.000Z", "更早的指令"),
            message,
        ])
        .await;

        let live = server.live().await;

        assert_eq!(live["live"][0]["lastPrompt"], "幫我修好登入頁");
        assert_eq!(live["live"][0]["promptAt"], 1792742400000u64);
    }
}

#[tokio::test]
async fn text_injected_by_codex_is_not_mistaken_for_an_instruction() {
    let server = server_with_codex_rollout(&[
        codex_user_item("2026-10-23T08:00:00.000Z", "幫我修好登入頁"),
        codex_user_response(
            "2026-10-23T08:05:00.000Z",
            &[
                "# AGENTS.md instructions for /work/erp\n\n<INSTRUCTIONS>...</INSTRUCTIONS>",
                "<environment_context>\n  <cwd>/work/erp</cwd>\n</environment_context>",
            ],
        ),
    ])
    .await;

    let live = server.live().await;

    assert_eq!(live["live"][0]["lastPrompt"], "幫我修好登入頁");
    assert_eq!(live["live"][0]["promptAt"], 1792742400000u64);
}

#[tokio::test]
async fn a_codex_instruction_older_than_the_tail_window_is_still_found() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", None, 1000, "user", "done");
    server.processes(&[(101, "ttys003", "codex", "/work/erp")]);
    let filler = codex_event(
        "2026-10-23T08:00:00.000Z",
        json!({"type":"agent_message","message":"長".repeat(1000)}),
    );
    let text = format!(
        "{}\n{}",
        codex_user_item("2026-10-23T07:00:00.000Z", "很久以前下的指令"),
        format!("{filler}\n").repeat(400)
    );
    assert!(text.len() > 400_000);
    fs::write(
        server
            .root
            .join(format!("codex/sessions/rollout-{T1}.jsonl")),
        text,
    )
    .unwrap();

    let live = server.live().await;

    assert_eq!(live["live"][0]["lastPrompt"], "很久以前下的指令");
    assert_eq!(live["live"][0]["promptAt"], 1792738800000u64);
}

#[tokio::test]
async fn a_long_turn_whose_start_is_older_than_the_tail_window_is_still_working() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", None, 1000, "user", "done");
    server.processes(&[(101, "ttys003", "codex", "/work/erp")]);
    let output = codex_event(
        "2026-10-23T08:00:01.000Z",
        json!({"type":"agent_message","message":"長".repeat(1000)}),
    );
    // 回合開始之後有 400 KB 以上的輸出，尾端範圍裡已經看不到任何回合事件
    let text = format!(
        "{}\n{}{}\n",
        codex_event(
            "2026-10-23T08:00:00.000Z",
            json!({"type":"task_started","turn_id":"t1"})
        ),
        format!("{output}\n").repeat(400),
        // 最後的指令就在尾端範圍裡，只有回合狀態需要往前找
        codex_user_item("2026-10-23T08:10:00.000Z", "補一個指令")
    );
    assert!(text.len() > 400_000);
    fs::write(
        server
            .root
            .join(format!("codex/sessions/rollout-{T1}.jsonl")),
        text,
    )
    .unwrap();

    assert_eq!(server.live().await["live"][0]["status"], "working");
}

fn codex_call(timestamp: &str, name: &str, arguments: Value) -> Value {
    json!({"timestamp":timestamp,"type":"response_item","payload":{"type":"function_call","name":name,"arguments":arguments.to_string(),"call_id":"c1"}})
}

fn codex_plan(steps: &[(&str, &str)]) -> Value {
    let plan: Vec<Value> = steps
        .iter()
        .map(|(step, status)| json!({"step":step,"status":status}))
        .collect();
    codex_call(
        "2026-10-23T08:00:10.000Z",
        "update_plan",
        json!({"plan":plan}),
    )
}

fn codex_question() -> Value {
    codex_call(
        "2026-10-23T08:01:00.000Z",
        "request_user_input_async",
        json!({"questions":[{"title":"要用哪種格式？","options":["A","B"]}]}),
    )
}

fn codex_turn(timestamp: &str, kind: &str) -> Value {
    codex_event(timestamp, json!({"type":kind,"turn_id":"t1"}))
}

#[tokio::test]
async fn codex_plan_steps_become_a_progress_bar() {
    let server = server_with_codex_rollout(&[
        codex_user_item("2026-10-23T08:00:00.000Z", "做登入功能"),
        codex_plan(&[
            ("設計", "pending"),
            ("實作", "in_progress"),
            ("驗證", "pending"),
        ]),
        codex_plan(&[
            ("設計", "completed"),
            ("實作", "in_progress"),
            ("驗證", "pending"),
        ]),
    ])
    .await;

    let live = server.live().await;

    assert_eq!(
        live["live"][0]["progress"],
        json!({"done":1,"total":3,"current":"實作"})
    );
}

#[tokio::test]
async fn a_codex_session_without_a_plan_has_no_progress() {
    let server =
        server_with_codex_rollout(&[codex_user_item("2026-10-23T08:00:00.000Z", "隨便問問")]).await;
    assert_eq!(server.live().await["live"][0]["progress"], Value::Null);
}

#[tokio::test]
async fn a_question_codex_asked_and_left_unanswered_makes_the_card_waiting() {
    let server = server_with_codex_rollout(&[
        codex_user_item("2026-10-23T08:00:00.000Z", "做登入功能"),
        codex_turn("2026-10-23T08:00:01.000Z", "task_started"),
        codex_question(),
        codex_turn("2026-10-23T08:01:05.000Z", "task_complete"),
    ])
    .await;

    assert_eq!(server.live().await["live"][0]["status"], "waiting");
}

#[tokio::test]
async fn while_codex_keeps_working_after_asking_the_card_stays_working() {
    let server = server_with_codex_rollout(&[
        codex_user_item("2026-10-23T08:00:00.000Z", "做登入功能"),
        codex_turn("2026-10-23T08:00:01.000Z", "task_started"),
        codex_question(),
    ])
    .await;

    assert_eq!(server.live().await["live"][0]["status"], "working");
}

#[tokio::test]
async fn once_the_person_answers_the_codex_card_is_no_longer_waiting() {
    let server = server_with_codex_rollout(&[
        codex_user_item("2026-10-23T08:00:00.000Z", "做登入功能"),
        codex_turn("2026-10-23T08:00:01.000Z", "task_started"),
        codex_question(),
        codex_turn("2026-10-23T08:01:05.000Z", "task_complete"),
        codex_user_item("2026-10-23T08:05:00.000Z", "選 A"),
        codex_turn("2026-10-23T08:05:01.000Z", "task_started"),
        codex_turn("2026-10-23T08:06:00.000Z", "task_complete"),
    ])
    .await;

    assert_eq!(server.live().await["live"][0]["status"], "idle");
}

#[tokio::test]
async fn other_codex_tool_calls_never_make_the_card_waiting() {
    let server = server_with_codex_rollout(&[
        codex_user_item("2026-10-23T08:00:00.000Z", "跑測試"),
        codex_turn("2026-10-23T08:00:01.000Z", "task_started"),
        codex_call("2026-10-23T08:00:20.000Z", "wait", json!({})),
        codex_turn("2026-10-23T08:01:05.000Z", "task_complete"),
    ])
    .await;

    assert_eq!(server.live().await["live"][0]["status"], "idle");
}

/// 假的 Ghostty 分頁清單：每行「工作資料夾 \t 標題」。
fn tabs(rows: &[(&str, &str)]) -> String {
    rows.iter()
        .map(|(cwd, title)| format!("{cwd}\t{title}\n"))
        .collect()
}

fn names(live: &Value) -> Vec<&str> {
    live["live"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect()
}

#[tokio::test]
async fn two_codex_processes_in_one_folder_are_told_apart_by_their_tab_titles() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", Some("工作 A"), 3000, "user", "done");
    server.codex_thread(T2, "/work/erp", Some("工作 B"), 2000, "user", "done");
    server.codex_thread(
        "01a1-old",
        "/work/erp",
        Some("舊的 C"),
        1000,
        "user",
        "done",
    );
    server.processes(&[
        (101, "ttys003", "codex", "/work/erp"),
        (102, "ttys004", "codex", "/work/erp"),
    ]);
    server.fake.script(
        "osascript",
        &tabs(&[
            ("/work/erp", "工作 B | erp"),
            ("/work/erp", "✳ 一個 Claude Code session"),
            ("/work/erp", "工作 A | erp"),
            ("/work/other", "舊的 C | other"),
        ]),
    );

    let live = server.live().await;

    assert_eq!(names(&live), vec!["工作 A", "工作 B"]);
}

#[tokio::test]
async fn a_single_process_follows_its_tab_even_when_a_newer_conversation_exists() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", Some("新的對話"), 3000, "user", "done");
    server.codex_thread(
        T2,
        "/work/erp",
        Some("從清單接續的舊對話"),
        1000,
        "user",
        "done",
    );
    server.processes(&[(101, "ttys003", "codex resume", "/work/erp")]);
    server.fake.script(
        "osascript",
        &tabs(&[("/work/erp", "從清單接續的舊對話 | erp")]),
    );

    let live = server.live().await;

    assert_eq!(names(&live), vec!["從清單接續的舊對話"]);
}

#[tokio::test]
async fn when_tab_titles_cannot_be_read_the_latest_conversation_is_shown_alone() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", Some("新的"), 3000, "user", "done");
    server.codex_thread(T2, "/work/erp", Some("舊的"), 1000, "user", "done");
    server.processes(&[
        (101, "ttys003", "codex", "/work/erp"),
        (102, "ttys004", "codex", "/work/erp"),
    ]);
    server.fake.failing("osascript");

    let live = server.live().await;

    assert_eq!(names(&live), vec!["新的"]);
}

#[tokio::test]
async fn ghostty_is_only_asked_when_the_folder_has_more_than_one_conversation() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", Some("唯一的對話"), 3000, "user", "done");
    server.processes(&[(101, "ttys003", "codex", "/work/erp")]);
    server.fake.script("osascript", "");

    let live = server.live().await;

    assert_eq!(names(&live), vec!["唯一的對話"]);
    assert!(server.fake.calls("osascript").is_empty());
}

#[tokio::test]
async fn clicking_a_codex_card_looks_for_a_tab_in_that_folder_that_starts_with_the_name() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(
        T1,
        "/work/mango-hsu",
        Some("比較模式"),
        1000,
        "user",
        "done",
    );
    server.processes(&[(101, "ttys003", "codex", "/work/mango-hsu")]);
    server.fake.script("osascript", "ok\n");

    let (status, body) = server
        .post_json(FOCUS, json!({"agent":"codex","id":T1}))
        .await;

    assert_eq!((status, &body["focused"]), (200, &json!(true)));
    let calls = server.fake.calls("osascript");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][2..], ["/work/mango-hsu", "比較模式", "starts"]);
}

#[tokio::test]
async fn a_codex_card_that_is_not_open_cannot_be_focused() {
    let server = LiveServer::start(Some("127.0.0.1")).await;
    server.codex_thread(T1, "/work/erp", Some("已經關掉的"), 1000, "user", "done");
    server.fake.script("osascript", "ok\n");

    let (status, _) = server
        .post_json(FOCUS, json!({"agent":"codex","id":T1}))
        .await;

    assert_eq!(status, 404);
    assert!(server.fake.calls("osascript").is_empty());
}
