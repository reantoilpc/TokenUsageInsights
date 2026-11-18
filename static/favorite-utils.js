// 收藏：讓 session 不受「最近結束」24 小時、10 筆的限制，隔幾天還能接續。
// 存成 { key: 'agent:id', name, addedAt } 的陣列，新收藏的放最前面。
import { cardKey } from './card-utils.js';

const DAY_MS = 24 * 3600 * 1000;
// Claude Code 依 cleanupPeriodDays 刪除舊對話紀錄，最後一週開始提醒
const WARN_DAYS = 7;

// localStorage 的內容可能被手動改壞：只留下 agent 認得、時間是數字、不重複的項目
export function parseFavorites(text) {
  let value;
  try {
    value = JSON.parse(text);
  } catch {
    return [];
  }
  if (!Array.isArray(value)) return [];
  const seen = new Set();
  return value.flatMap((item) => {
    const valid =
      item !== null &&
      typeof item === 'object' &&
      typeof item.key === 'string' &&
      /^(claude|codex):/.test(item.key) &&
      typeof item.addedAt === 'number' &&
      !seen.has(item.key);
    if (!valid) return [];
    seen.add(item.key);
    return [{ key: item.key, name: typeof item.name === 'string' ? item.name : null, addedAt: item.addedAt }];
  });
}

export function isFavorite(favorites, session) {
  return favorites.some((favorite) => favorite.key === cardKey(session));
}

export function toggleFavorite(favorites, session, now) {
  const key = cardKey(session);
  if (favorites.some((favorite) => favorite.key === key)) {
    return favorites.filter((favorite) => favorite.key !== key);
  }
  return [{ key, name: session.name ?? null, addedAt: now }, ...favorites];
}

export function favoritesQuery(favorites) {
  return favorites.map((favorite) => favorite.key).join(',');
}

// 只移除後端確定接續不了（gone）的；讀不到（unknown）或這次沒回來的都留著，
// 一次暫時的錯誤不能把收藏清掉。留下的順便記住最新的名稱，之後提示時用得到
export function dropGone(favorites, fromServer) {
  const byKey = new Map(fromServer.map((item) => [cardKey(item), item]));
  const kept = [];
  const removed = [];
  for (const favorite of favorites) {
    const item = byKey.get(favorite.key);
    if (item?.state === 'gone') {
      removed.push({ name: favorite.name, reason: item.reason });
    } else {
      kept.push(item?.name ? { ...favorite, name: item.name } : favorite);
    }
  }
  return { kept, removed };
}

// 只有已結束的 Claude Code 會被清除；Codex 不會自動刪紀錄，開著的 session 紀錄一直在更新
export function cleanupWarning(item, cleanupDays, now) {
  if (item.agent !== 'claude' || item.state !== 'ended') return null;
  const daysLeft = Math.ceil(cleanupDays - (now - item.updatedAt) / DAY_MS);
  if (daysLeft > WARN_DAYS) return null;
  return daysLeft <= 0 ? '對話紀錄隨時可能被 Claude Code 清除' : `約 ${daysLeft} 天後可能被 Claude Code 清除`;
}
