# 22: 留下的登記檔遇到 pid 被重複使用時會多一張卡片

**What to build:** 判斷 Claude Code 還開著時只檢查登記檔的 pid 還在不在（`kill(pid, 0)`），沒有比對登記檔的 `procStart`。Claude Code 被 SIGKILL 或當掉時登記檔會留下來（2026-10-30 實測：以 SIGKILL 結束暫停中的 `claude` 後，`<pid>.json` 還在）；之後這個 pid 被系統給了別的程式，那個舊 session 就會以「進行中」出現。讀程式碼推得，尚未在真實資料碰到。

可能的做法：比對登記檔的 `procStart` 與程序實際的啟動時間（Linux 上就是 `/proc/<pid>/stat` 第 22 欄，已以兩個真實程序比對相同），或至少確認程式名稱是 `claude`。關閉功能已經在送訊號前確認程式名稱，這裡是顯示卡片的判斷。

**Blocked by:** —

**Status:** needs-triage

**Spec:** ../spec.md

- [ ] 確認 macOS 上 `procStart` 的格式
- [ ] 決定以啟動時間或程式名稱判斷，並加測試
