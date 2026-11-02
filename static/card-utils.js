// 「進行中」卡片的識別：釘選、優先度、關閉與改名都以 `agent:id` 對應到 session。

export function cardKey(session) {
  return `${session.agent}:${session.id}`;
}

// 釘選與優先度共用的保留規則。接續不會換 id：結束的 session 還在「最近結束」時保留，接續後才對得上。
// 兩份都空時一律保留：後端讀取失敗也回空清單（live.rs 的 unwrap_or_default），不能因此清光
export function isStillListed(key, live, recent) {
  if (live.length === 0 && recent.length === 0) return true;
  return [...live, ...recent].some((session) => cardKey(session) === key);
}
