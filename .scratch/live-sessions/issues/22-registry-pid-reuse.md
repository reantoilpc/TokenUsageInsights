# 22: 留下的登記檔遇到 pid 被重複使用時會多一張卡片

**What to build:** 原本判斷 Claude Code 還開著時只檢查登記檔的 pid 還在不在（`kill(pid, 0)`），沒有確認那個 pid 還是 claude。Claude Code 被 SIGKILL 或當掉時登記檔會留下來（2026-10-30 實測：以 SIGKILL 結束暫停中的 `claude` 後，`<pid>.json` 還在）；之後這個 pid 被系統給了別的程式，那個舊 session 就會以「進行中」出現。2026-11-09 在真實資料碰到：WSL 重開後，舊登記檔 `1745.json`（狀態 busy）的 pid 被給了一直在跑的 `/init`，卡片停在「工作中」，點了顯示「找不到這個 session 的分頁」，也沒有關閉鈕。

可能的做法：比對登記檔的 `procStart` 與程序實際的啟動時間（Linux 上就是 `/proc/<pid>/stat` 第 22 欄，已以兩個真實程序比對相同），或至少確認程式名稱是 `claude`。關閉功能已經在送訊號前確認程式名稱，這裡是顯示卡片的判斷。

**Blocked by:** —

**Status:** ready-for-agent

**Spec:** ../spec.md

- [x] 決定以程式名稱判斷（`kill(pid, 0)` 先篩，pid 還在的才查名稱），並加測試：與關閉、暫停判斷用同一個 `process::inspect`，macOS 也適用；不比對 `procStart`（原本要先確認它在 macOS 的格式，這項因此不做），因為 macOS 上的格式未確認，且舊版 Claude Code 的登記檔沒有這個欄位。pid 被別的 `claude` 拿到的情況不必處理：新的 Claude Code 會寫同一個 `<pid>.json`，蓋掉舊的登記檔。
