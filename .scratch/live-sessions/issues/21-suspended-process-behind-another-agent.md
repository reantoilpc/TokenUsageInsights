# 21: 暫停中的程序在 /clear 後又多出一張卡片

**What to build:** 在同一個分頁以 Ctrl+Z 暫停 `claude`、`claude --resume` 接續同一個 id，之後又 `/clear`，卡片從一張變成兩張（2026-10-30 在本機觀察到）。暫停中的程序登記檔停在舊的 session id，`/clear` 讓還在用的那支換了新 id，17 的「同一個 `agent:id` 只回一張」就合不起來了。單純 `/clear` 不會多一張；縮小後的條件是「同一個終端機裡有一個暫停中的 agent，又有另一個 agent 在跑」。

改為：被暫停的程序若同一個終端機（tty）上還有沒暫停的 agent，它的 session 不算進行中。分頁裡沒有別的 agent 時照常顯示，因為可能等一下就 `fg` 回來。使用者 2026-10-30 選定。細節見規格的「實作決策 → 卡片內容」。

**Blocked by:** —

**Status:** ready-for-agent

**Spec:** ../spec.md

- [x] 暫停中、同一個終端機已有別的 agent 在跑的 session 不在「進行中」，改列在「最近結束」（測試，只在 Linux 跑）
- [x] 只按 Ctrl+Z、分頁裡沒有別的 agent 時卡片照常顯示，別的分頁裡的 agent 不影響它（測試，只在 Linux 跑）
- [x] 以真的 Claude Code 在 tmux 重現：對話 → Ctrl+Z → `claude --resume` → `/clear`，修正前兩張、修正後一張；本機原本重複的那組也只剩一張
- [ ] 在 Mac 實測：macOS 以 `/bin/ps` 查程序是否暫停與它的 tty（同 20，這台是 WSL）
- [x] 通過提交前檢查（同 01）
