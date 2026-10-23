//! 「進行中」即時 session 頁面的整合測試。
//!
//! 唯一的測試接縫是真正編好的程式的 HTTP API：啟動 binary、指到暫存資料夾，
//! 並把假的 `ps`／`lsof`／`osascript` 放在 PATH 最前面（腳本依劇本回答，並記錄收到的參數）。
//! 輔助程式刻意獨立於 `additional_sources.rs`，避免與上游對該檔的修改衝突。

use serde_json::Value;
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
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl LiveServer {
    /// 啟動伺服器。`host` 為 `None` 時不設定 `HOST`，用來驗證預設綁定位址。
    async fn start(host: Option<&str>) -> Self {
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
            .stdout(Stdio::from(
                fs::File::create(root.join("server.log")).unwrap(),
            ))
            .stderr(Stdio::null());
        if let Some(host) = host {
            command.env("HOST", host);
        }
        let mut server = Self {
            child: command.spawn().unwrap(),
            root,
            url: format!("http://127.0.0.1:{port}"),
            port,
            fake,
        };
        server.wait_until_ready().await;
        server
    }

    async fn wait_until_ready(&mut self) {
        let client = client();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if client
                .get(format!("{}/api/version", self.url))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return;
            }
            assert!(
                Instant::now() < deadline && self.child.try_wait().unwrap().is_none(),
                "startup failed: {}",
                fs::read_to_string(self.root.join("server.log")).unwrap_or_default()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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
