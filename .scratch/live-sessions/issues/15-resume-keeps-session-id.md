# 15: 更正「接續會換 session id」的錯誤說明

**What to build:** 程式註解與規格都說「重開同一段對話（resume）會換 session id」，但實際上 `claude --resume <id>` 預設沿用原本的 id（`claude --help`：`--fork-session` 才會「create a new session ID instead of reusing the original」），`codex resume <id>` 也延續同一段對話（另有 `codex fork` 才是複製）。會換 id 的是 `/clear`。

需要更正的地方：

- `src/live.rs` 的 `recent_sessions`：「重開同一段對話會換 session id」的註解。
- `.scratch/live-sessions/spec.md` 的「最近結束」：「重開同一段對話（例如 resume 或 /clear 之後）會換一個 session id」。

同時確認「標題與資料夾都相同就當作同一段」這條去重規則在只剩 `/clear` 的情況下是否仍然需要、行為是否正確（`/clear` 之後的新 session 會不會沿用原本的標題）。

**Blocked by:** —

**Status:** needs-triage

**Spec:** ../spec.md

- [ ] 註解與規格改為正確的說法
- [ ] 確認去重規則仍有必要，或改寫／移除並補測試
