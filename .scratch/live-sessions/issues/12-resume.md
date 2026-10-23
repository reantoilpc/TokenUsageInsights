# 12: 接續已結束的 session

**What to build:** 「最近結束」的每一筆有「接續」按鈕。按下去，系統在最前面的 Ghostty 視窗開一個新分頁（沒有視窗就開新視窗），以 Ghostty 的初始工作資料夾設定原資料夾，並以初始輸入把 `claude --resume <id>` 或 `codex resume <id>` 打進 shell，因此 agent 結束後分頁仍保有一般 shell。不跳確認視窗。還開著的 session 不提供接續，避免兩個程式搶同一份對話紀錄。

**Blocked by:** 06, 11

**Status:** ready-for-agent

**Spec:** ../spec.md

- [ ] 假的 `osascript` 收到的輸入文字只有兩種固定形式：`claude --resume <id>` 或 `codex resume <id>`；資料夾由初始工作資料夾設定，不出現在輸入文字裡（測試）
- [ ] id 必須通過 UUID 格式檢查，且是「最近結束」清單上已知的 session；未知、格式錯誤、或屬於進行中 session 的 id 一律拒絕（測試）
- [ ] 請求內帶資料夾或指令文字時被忽略或拒絕，不影響實際執行（測試）
- [ ] 沿用工單 06 的來源檢查與 JSON 內容類型要求；不符一律拒絕且沒有任何 `osascript` 呼叫（測試）
- [ ] 沒有 Ghostty 視窗時改為開新視窗（測試）
- [ ] 資料夾路徑含空白或特殊字元時仍正確（測試）
- [ ] 「進行中」卡片上不出現接續按鈕
- [ ] 通過提交前檢查（同 01）
