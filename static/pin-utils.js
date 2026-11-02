// 「進行中」卡片的識別與釘選：釘選是 `agent:id` 的陣列，陣列順序就是釘選先後。

export function cardKey(session) {
  return `${session.agent}:${session.id}`;
}

// localStorage 的內容可能被手動改壞：只留下不重複的字串
export function parsePins(text) {
  try {
    const value = JSON.parse(text);
    return Array.isArray(value) ? [...new Set(value.filter((pin) => typeof pin === 'string'))] : [];
  } catch {
    return [];
  }
}

export function togglePin(pins, key) {
  return pins.includes(key) ? pins.filter((pin) => pin !== key) : [...pins, key];
}

// 釘住的依釘選先後排在前面，不看狀態；其餘保留傳入的順序
export function orderByPins(sessions, pins) {
  const pinned = pins.flatMap((pin) => sessions.filter((session) => cardKey(session) === pin));
  const rest = sessions.filter((session) => !pins.includes(cardKey(session)));
  return [...pinned, ...rest];
}

// 釘選與優先度共用的保留規則。接續不會換 id：結束的 session 還在「最近結束」時保留，接續後才對得上。
// 兩份都空時一律保留：後端讀取失敗也回空清單（live.rs 的 unwrap_or_default），不能因此清光
export function stillListed(live, recent) {
  if (live.length === 0 && recent.length === 0) return () => true;
  const known = new Set([...live, ...recent].map(cardKey));
  return (key) => known.has(key);
}

export function prunePins(pins, live, recent) {
  return pins.filter(stillListed(live, recent));
}
