// 「進行中」卡片的識別（`agent:id`）與釘選、優先度的保留規則。

export function cardKey(session) {
  return `${session.agent}:${session.id}`;
}

// cardKey 的反向：id 本身不含冒號，以第一個冒號切開
export function parseCardKey(key) {
  const separator = key.indexOf(':');
  return { agent: key.slice(0, separator), id: key.slice(separator + 1) };
}

// 接續不會換 id：結束的 session 還在「最近結束」或收藏時保留，接續後才對得上。
// 進行中與最近結束都空時一律保留：後端讀取失敗也回空清單（live.rs 的 unwrap_or_else），不能因此清光
export function isStillListed(key, live, recent, favorites = []) {
  if (live.length === 0 && recent.length === 0) return true;
  return [...live, ...recent, ...favorites].some((session) => cardKey(session) === key);
}
