# 20: 從卡片關閉 session 與它的終端機分頁

**What to build:** 閒置或等你回覆的卡片加一個 ✕，點兩下（第一下變成「確定關閉？」，3 秒內再點一次）後，看板關掉那個 session 的終端機分頁，agent 收到掛斷訊號後像 `/exit` 一樣收尾結束，釋放記憶體。Claude Code 與 Codex 都做；Windows Terminal 關整個分頁，Ghostty 關 agent 所在的終端機。分頁關了程序還在就補送 SIGTERM；找不到分頁時直接送 SIGTERM，只結束 agent。同一個 session 被 Ctrl+Z 暫停的舊程序以 SIGKILL 一起結束。工作中不給關；同一個資料夾開了多個沒帶 id 的 Codex 時分不出是哪個程序，拒絕。

使用者 2026-10-29 選定（grilling 四輪）。細節見規格的「實作決策 → 關閉」。

**Blocked by:** —

**Status:** ready-for-agent

**Spec:** ../spec.md

- [x] `POST /api/live/close`：關分頁、確認程序結束、必要時補送 SIGTERM，回傳 `{ended, tabClosed}`（測試）
- [x] Windows Terminal：剛好一個分頁符合才按它的關閉鈕；同名分頁或找不到時只結束 agent（測試）
- [x] Ghostty：1.4 以 tty 找終端機，1.3 以資料夾與標題找，關掉那個終端機（測試只驗證參數）
- [x] 不理 SIGTERM 的程序回報還在，不強制結束（測試）
- [x] 暫停中的舊程序一起結束；工作中、Codex 分不出程序、程式名稱不對、來源不符時拒絕（測試）
- [x] 前端 ✕、點兩下確認、關閉中與結果提示（`close-utils.js` 測試）
- [x] 在這台的 Windows Terminal 實測：分頁關掉、Claude Code 與 MCP 子程序結束、登記檔刪除
- [ ] 在 Mac 實測：Ghostty 關終端機，以及以 `/bin/ps` 查程序現況（這台是 WSL，兩者都沒跑過）
- [x] 深色、淺色與窄螢幕以假資料截圖確認
- [x] 通過提交前檢查（同 01）
