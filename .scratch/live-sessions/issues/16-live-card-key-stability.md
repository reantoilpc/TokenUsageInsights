# 16: 「進行中」卡片的 `agent:id` 可能重複或在輪詢間變動

**What to build:** 卡片以 `agent:id` 識別（跳轉、鍵盤焦點、釘選都靠它），但下列情況會讓同一個 key 出現兩張卡片，或讓同一個終端機的卡片換 key：

- **重複（2026-10-28 在本機觀察到）**：在同一個 shell 裡以 Ctrl+Z 暫停一個 `claude`，再執行 `claude --resume <同一個 id>`。兩個程序都登記同一個 sessionId，而 `kill(pid, 0)` 對暫停中的程序也會成功（`src/live/claude.rs` 的存活判斷），所以畫面上出現兩張 key 相同的卡片；`find_session`（`src/live.rs`）只取第一個符合的，跳轉可能跳到暫停的那個。
- **Codex 沒帶 resume id 時的猜測（讀程式碼推得，尚未實測）**：直接執行 `codex` 或 `codex resume --last` 時，以「該資料夾最近更新的對話」對應程序（`src/live/codex.rs`）。本機的 `state_5.sqlite` 沒有 `originator` 欄位，子 agent 與 `exec` 的對話 `thread_source` 為 NULL 也會被當成候選，輪詢之間可能對應到不同的對話。
- **次要**：`recent_sessions` 判斷「是否還開著」只用 id、沒有帶 agent。

**Blocked by:** —

**Status:** needs-triage

**Spec:** ../spec.md

- [ ] 決定暫停中的程序要不要算「進行中」，以及同一個 sessionId 有多個程序時怎麼呈現
- [ ] 實測 Codex 未帶 resume id 時，卡片的 id 是否會在輪詢間變動；會的話收斂候選條件
- [ ] 「是否還開著」的判斷改以 agent 加 id
