// 「進行中」卡片的釘選：釘選是 `agent:id` 的陣列，陣列順序就是釘選先後。

export function pinKey(session) {
  return `${session.agent}:${session.id}`;
}

// localStorage 的內容可能被手動改壞：不是字串陣列就當作沒有釘選
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

// 釘住的依釘選先後排在前面，不看狀態；其餘保留後端排好的順序
export function orderByPins(sessions, pins) {
  const pinned = pins.flatMap((pin) => sessions.filter((session) => pinKey(session) === pin));
  const rest = sessions.filter((session) => !pins.includes(pinKey(session)));
  return [...pinned, ...rest];
}

// 接續不會換 id：結束的 session 還在「最近結束」時保留釘選，接續後才回得到原位。
// 兩份都空時不清：後端讀取失敗也回空清單（live.rs 的 unwrap_or_default），不能因此清光釘選
export function prunePins(pins, live, recent) {
  if (live.length === 0 && recent.length === 0) return pins;
  const known = new Set([...live, ...recent].map(pinKey));
  return pins.filter((pin) => known.has(pin));
}
