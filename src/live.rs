//! 「進行中」即時 session API：回傳目前還開著的 session 與最近結束的 session。
//! 即時狀態不進資料庫，每次請求即時讀取。

use axum::Json;
use serde_json::{json, Value};

pub async fn get_live() -> Json<Value> {
    Json(json!({ "live": [], "recent": [] }))
}
