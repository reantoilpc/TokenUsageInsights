# 17: 壓縮摘要被當成最後指令、同一個 session 出現兩張卡片

**What to build:** 修正「進行中」卡片的兩個錯誤（2026-10-29 以真實資料確認）：

- 對話自動壓縮後，Claude Code 把摘要（「This session is being continued from a previous conversation…」）以使用者訊息寫進紀錄，標記為 `isCompactSummary`。目前沒有排除，13 張卡片中有 2 張的最後一行是這段摘要。改為略過它，往前找人打的最後一句；「最近結束」也一樣。
- 同一個分頁以 Ctrl+Z 暫停 `claude` 後，又 `claude --resume` 同一個 id：兩個程序都登記同一個 sessionId，畫面出現兩張一樣的卡片。改為同一個 `agent:id` 只回一張，取登記檔更新時間最新的那個，跳轉也照它的資料找分頁。這是 16 的第一項。

細節見規格的「實作決策 → 卡片內容」與「對話紀錄尾端解析」。

**Blocked by:** —

**Status:** ready-for-agent

**Spec:** ../spec.md

- [x] 壓縮摘要不算最後指令，「進行中」與「最近結束」都往前找人打的最後一句（測試）
- [x] 同一個 `agent:id` 有多個程序時只回一張卡片，取登記檔更新時間最新的那個（測試）
- [x] 以真實資料確認：原本顯示壓縮摘要的卡片改顯示人打的話，重複的卡片只剩一張
- [x] 通過提交前檢查（同 01）
