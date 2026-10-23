# 13: 收尾驗證

**What to build:** 確認整個「進行中」功能在真實環境下可用，並通過原作者規範的全部檢查。包含用真實資料在另一個 port 啟動、截圖確認深色、淺色與窄螢幕，以及請我本人在真的 Ghostty 手動試一次跳轉與接續，因為假腳本只能驗證「會下什麼指令」，無法證明真的跳過去。

**Blocked by:** 04, 05, 08, 09, 10, 12（等於全部完成）

**Status:** ready-for-agent

**Spec:** ../spec.md

- [x] `cargo fmt --check`、`cargo test --locked`、release 全目標建置零警告、`cargo clippy --all-targets --all-features -- -D warnings`、`npm test`、`git diff --check` 全部通過
- [x] 以真實資料在另一個 port 啟動：「進行中」清單與我實際開著的 Claude Code、Codex session 一致
- [x] 截圖確認深色、淺色與窄螢幕；沒有排版破裂或文字被截掉
- [ ] 由我本人在真的 Ghostty 手動確認：點 Claude Code 卡片跳到正確分頁、點 Codex 卡片跳到正確分頁、「接續」開出新分頁並執行正確指令；我回報結果後才勾選此項
- [x] 確認預設只監聽 `127.0.0.1`，從另一台裝置無法連到
- [x] 確認既有首頁、每日、月度、年度功能未受影響
- [x] 對既有檔案的修改僅限規格所列的三處（路由註冊、首頁連結、預設綁定位址），並把實際清單記錄下來
- [x] 把和上游 `main` 的差異整理出來，確認之後合併上游時可能衝突的檔案
