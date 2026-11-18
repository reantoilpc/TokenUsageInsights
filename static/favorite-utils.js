// 收藏：讓 session 不受「最近結束」24 小時、10 筆的限制，隔幾天還能接續。
// 存成 { key: 'agent:id', name, folder, addedAt } 的陣列，新收藏的放最前面。
// 名稱與資料夾是上一次拿到的資料：後端暫時讀不到時照樣顯示。
import { cardKey } from './card-utils.js';

const DAY_MS = 24 * 3600 * 1000;
// Claude Code 依 cleanupPeriodDays 刪除舊對話紀錄，最後一週開始提醒
const WARN_DAYS = 7;
// 收藏的 key：claude: 或 codex: 開頭
const FAVORITE_KEY = /^(claude|codex):/;

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
      FAVORITE_KEY.test(item.key) &&
      typeof item.addedAt === 'number' &&
      !seen.has(item.key);
    if (!valid) return [];
    seen.add(item.key);
    const text = (field) => (typeof item[field] === 'string' ? item[field] : null);
    return [{ key: item.key, name: text('name'), folder: text('folder'), addedAt: item.addedAt }];
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
  return [{ key, name: session.name ?? null, folder: session.folder ?? null, addedAt: now }, ...favorites];
}

export function favoritesQuery(favorites) {
  return favorites.map((favorite) => favorite.key).join(',');
}

// 只移除後端確定接續不了（gone）的；讀不到（unknown）或這次沒回來的都留著，
// 一次暫時的錯誤不能把收藏清掉。留下的順便記住最新的名稱與資料夾，讀不到時與提示時用得到
export function dropGone(favorites, fromServer) {
  const byKey = new Map(fromServer.map((item) => [cardKey(item), item]));
  const kept = [];
  const removed = [];
  for (const favorite of favorites) {
    const item = byKey.get(favorite.key);
    if (item?.state === 'gone') {
      removed.push({ name: favorite.name, reason: item.reason });
    } else {
      kept.push({ ...favorite, name: item?.name || favorite.name, folder: item?.folder || favorite.folder });
    }
  }
  return { kept, removed };
}

// 左側清單每一列，依收藏先後。後端讀不到或還沒回這一筆時，用上一次記下的名稱與資料夾
export function favoriteRows(favorites, fromServer) {
  const byKey = new Map(fromServer.map((item) => [cardKey(item), item]));
  return favorites.map((favorite) => {
    const item = byKey.get(favorite.key);
    if (item && item.state !== 'unknown') return item;
    const separator = favorite.key.indexOf(':');
    return {
      agent: favorite.key.slice(0, separator),
      id: favorite.key.slice(separator + 1),
      state: 'unknown',
      name: favorite.name,
      folder: favorite.folder,
    };
  });
}

// 收合狀態：'open'、'collapsed'，沒記過是 null
export function parseSidebar(text) {
  return text === '"open"' || text === '"collapsed"' ? JSON.parse(text) : null;
}

// 只有已結束的 Claude Code 會被清除；Codex 不會自動刪紀錄，開著的 session 紀錄一直在更新
export function cleanupWarning(item, cleanupDays, now) {
  if (item.agent !== 'claude' || item.state !== 'ended' || typeof cleanupDays !== 'number') return null;
  const daysLeft = Math.ceil(cleanupDays - (now - item.updatedAt) / DAY_MS);
  if (daysLeft > WARN_DAYS) return null;
  return daysLeft <= 0 ? '對話紀錄隨時可能被 Claude Code 清除' : `約 ${daysLeft} 天後可能被 Claude Code 清除`;
}
