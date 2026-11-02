// 「進行中」卡片的優先度：以 `agent:id` 對應 'high' 或 'low'，沒有記錄的就是一般。
import { orderByPins, pinKey, prunePins } from './pin-utils.js';

const RANK = { high: 0, low: 2 }; // 一般是 1

// localStorage 的內容可能被手動改壞：只留下值是 high 或 low 的項目
export function parsePriorities(text) {
  try {
    const value = JSON.parse(text);
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter(([, level]) => Object.hasOwn(RANK, level)));
  } catch {
    return {};
  }
}

export function priorityOf(priorities, session) {
  return priorities[pinKey(session)] ?? null;
}

// level 是 null 時回到一般
export function setPriority(priorities, key, level) {
  const next = { ...priorities };
  if (level) next[key] = level;
  else delete next[key];
  return next;
}

// 釘住的依釘選先後排在最前面。其餘維持後端的狀態順序，同一種狀態裡高的在前、低的在後：
// 等你回覆的 session 仍然最該先看，優先度只決定同樣在等的先回哪一個。sort 是穩定排序，同一級維持後端的順序
export function orderCards(sessions, pins, priorities) {
  const rank = (session) => RANK[priorityOf(priorities, session)] ?? 1;
  const statuses = [...new Set(sessions.map((session) => session.status))];
  const byStatus = statuses.flatMap((status) =>
    sessions.filter((session) => session.status === status).sort((a, b) => rank(a) - rank(b)),
  );
  return orderByPins(byStatus, pins);
}

// 保留規則同釘選：session 還在「進行中」或「最近結束」就保留，兩份都空時不清
export function prunePriorities(priorities, live, recent) {
  const kept = new Set(prunePins(Object.keys(priorities), live, recent));
  return Object.fromEntries(Object.entries(priorities).filter(([key]) => kept.has(key)));
}
