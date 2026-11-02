// 「進行中」卡片的優先度：'high'、'normal'（一般）或 'low'。只記錄高與低，以 `agent:id` 對應；沒有記錄就是一般。
import { cardKey, isStillListed } from './card-utils.js';
import { orderByPins } from './pin-utils.js';

const RANK = { high: 0, normal: 1, low: 2 };

// localStorage 的內容可能被手動改壞：只留下值是 high 或 low 的項目
export function parsePriorities(text) {
  try {
    const value = JSON.parse(text);
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter(([, level]) => level === 'high' || level === 'low'));
  } catch {
    return {};
  }
}

export function priorityOf(priorities, session) {
  return priorities[cardKey(session)] ?? 'normal';
}

export function setPriority(priorities, key, level) {
  const next = { ...priorities };
  if (level === 'normal') delete next[key];
  else next[key] = level;
  return next;
}

// 釘住的依釘選先後排在最前面；其餘維持後端的狀態順序，只在同一種狀態裡依優先度排：
// 等你回覆的仍然最該先看，優先度只決定同樣在等的先回哪一個
export function orderCards(sessions, pins, priorities) {
  const rank = (session) => RANK[priorityOf(priorities, session)];
  const statuses = [...new Set(sessions.map((session) => session.status))];
  const byStatus = statuses.flatMap((status) =>
    sessions.filter((session) => session.status === status).sort((a, b) => rank(a) - rank(b)),
  );
  return orderByPins(byStatus, pins);
}

export function prunePriorities(priorities, live, recent) {
  return Object.fromEntries(Object.entries(priorities).filter(([key]) => isStillListed(key, live, recent)));
}
