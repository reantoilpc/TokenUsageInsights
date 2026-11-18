import assert from 'node:assert/strict';
import test from 'node:test';

import {
  cleanupWarning,
  dropGone,
  favoritesQuery,
  isFavorite,
  parseFavorites,
  toggleFavorite,
} from '../static/favorite-utils.js';

const DAY = 24 * 3600 * 1000;
const session = (agent, id, name = null) => ({ agent, id, name });
const keys = (favorites) => favorites.map((favorite) => favorite.key);

test('a new favorite goes to the top and remembers its name and when it was added', () => {
  let favorites = toggleFavorite([], session('claude', 'a', '登入頁修正'), 100);
  favorites = toggleFavorite(favorites, session('codex', 'b'), 200);
  assert.deepEqual(favorites, [
    { key: 'codex:b', name: null, addedAt: 200 },
    { key: 'claude:a', name: '登入頁修正', addedAt: 100 },
  ]);
});

test('toggling a favorite again removes only that one, matching agent and id', () => {
  const favorites = [
    { key: 'codex:a', name: null, addedAt: 2 },
    { key: 'claude:a', name: null, addedAt: 1 },
  ];
  assert.deepEqual(keys(toggleFavorite(favorites, session('claude', 'a'), 3)), ['codex:a']);
  assert.equal(isFavorite(favorites, session('codex', 'a')), true);
  assert.equal(isFavorite(favorites, session('codex', 'b')), false);
});

test('stored favorites that were edited by hand are cleaned up instead of breaking the page', () => {
  assert.deepEqual(parseFavorites('not json'), []);
  assert.deepEqual(parseFavorites('{"key":"claude:a"}'), []);
  assert.deepEqual(parseFavorites(null), []);
  const text = JSON.stringify([
    { key: 'claude:a', name: '甲', addedAt: 2 },
    { key: 'claude:a', name: '重複', addedAt: 1 },
    { key: 'vim:x', addedAt: 1 },
    { key: 'claude:b', addedAt: 'yesterday' },
    { key: 'codex:c', name: 42, addedAt: 3 },
    'claude:d',
  ]);
  assert.deepEqual(parseFavorites(text), [
    { key: 'claude:a', name: '甲', addedAt: 2 },
    { key: 'codex:c', name: null, addedAt: 3 },
  ]);
});

test('the query lists every favorite as agent:id', () => {
  const favorites = [
    { key: 'codex:b', name: null, addedAt: 2 },
    { key: 'claude:a', name: null, addedAt: 1 },
  ];
  assert.equal(favoritesQuery(favorites), 'codex:b,claude:a');
  assert.equal(favoritesQuery([]), '');
});

test('only favorites the server says are gone are removed, with the reason to tell the user', () => {
  const favorites = [
    { key: 'claude:gone', name: '舊名稱', addedAt: 4 },
    { key: 'claude:unknown', name: null, addedAt: 3 },
    { key: 'codex:ended', name: null, addedAt: 2 },
    { key: 'claude:not-in-response', name: null, addedAt: 1 },
  ];
  const fromServer = [
    { agent: 'claude', id: 'gone', state: 'gone', reason: '對話紀錄不存在' },
    { agent: 'claude', id: 'unknown', state: 'unknown' },
    { agent: 'codex', id: 'ended', state: 'ended', name: '比較模式' },
  ];
  const { kept, removed } = dropGone(favorites, fromServer);
  assert.deepEqual(keys(kept), ['claude:unknown', 'codex:ended', 'claude:not-in-response']);
  assert.deepEqual(removed, [{ name: '舊名稱', reason: '對話紀錄不存在' }]);
});

test('kept favorites pick up the latest name from the server for later notices', () => {
  const favorites = [{ key: 'codex:ended', name: null, addedAt: 2 }];
  const { kept } = dropGone(favorites, [{ agent: 'codex', id: 'ended', state: 'ended', name: '比較模式' }]);
  assert.deepEqual(kept, [{ key: 'codex:ended', name: '比較模式', addedAt: 2 }]);
});

test('a Claude Code favorite warns in its last week before Claude Code cleans up the transcript', () => {
  const now = 100 * DAY;
  const ended = (daysAgo) => ({ agent: 'claude', state: 'ended', updatedAt: now - daysAgo * DAY });
  assert.equal(cleanupWarning(ended(22), 30, now), null);
  assert.equal(cleanupWarning(ended(23), 30, now), '約 7 天後可能被 Claude Code 清除');
  assert.equal(cleanupWarning(ended(29.5), 30, now), '約 1 天後可能被 Claude Code 清除');
  assert.equal(cleanupWarning(ended(31), 30, now), '對話紀錄隨時可能被 Claude Code 清除');
  assert.equal(cleanupWarning(ended(8), 14, now), '約 6 天後可能被 Claude Code 清除');
});

test('open sessions and Codex conversations never get the cleanup warning', () => {
  const now = 100 * DAY;
  assert.equal(cleanupWarning({ agent: 'claude', state: 'open', updatedAt: now - 40 * DAY }, 30, now), null);
  assert.equal(cleanupWarning({ agent: 'codex', state: 'ended', updatedAt: now - 40 * DAY }, 30, now), null);
  assert.equal(cleanupWarning({ agent: 'claude', state: 'unknown' }, 30, now), null);
});
