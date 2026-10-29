import assert from 'node:assert/strict';
import test from 'node:test';

import { CONFIRM_MS, closeNotice, isArmed, pressClose } from '../static/close-utils.js';

test('the first press only arms the button', () => {
  const first = pressClose(null, 'claude:a', 1000);
  assert.equal(first.send, false);
  assert.ok(isArmed(first.armed, 'claude:a', 1000));
});

test('a second press on the same card within the window sends', () => {
  const { armed } = pressClose(null, 'claude:a', 1000);
  const second = pressClose(armed, 'claude:a', 1000 + CONFIRM_MS - 1);
  assert.equal(second.send, true);
  assert.equal(second.armed, null);
});

test('after the window the next press arms again instead of sending', () => {
  const { armed } = pressClose(null, 'claude:a', 1000);
  assert.equal(isArmed(armed, 'claude:a', 1000 + CONFIRM_MS), false);
  const late = pressClose(armed, 'claude:a', 1000 + CONFIRM_MS);
  assert.equal(late.send, false);
  assert.ok(isArmed(late.armed, 'claude:a', 1000 + CONFIRM_MS));
});

test('pressing another card moves the arming there without sending', () => {
  const { armed } = pressClose(null, 'claude:a', 1000);
  const other = pressClose(armed, 'codex:b', 1500);
  assert.equal(other.send, false);
  assert.equal(isArmed(other.armed, 'claude:a', 1500), false);
  assert.ok(isArmed(other.armed, 'codex:b', 1500));
});

test('the notice says what happened to the process and the tab', () => {
  assert.equal(closeNotice({ ended: true, tabClosed: true }), '已關閉 session 與分頁');
  assert.equal(closeNotice({ ended: true, tabClosed: false }), '已結束，但找不到它的分頁，分頁沒有關');
  assert.equal(closeNotice({ ended: false, tabClosed: true }), '已送出結束訊號，程序還沒結束');
});
