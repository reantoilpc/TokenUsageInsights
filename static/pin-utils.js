// 「進行中」卡片的釘選：釘選是 `agent:id` 的陣列，陣列順序就是釘選先後。
import { cardKey, isStillListed } from './card-utils.js';

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

export function prunePins(pins, live, recent) {
  return pins.filter((pin) => isStillListed(pin, live, recent));
}
