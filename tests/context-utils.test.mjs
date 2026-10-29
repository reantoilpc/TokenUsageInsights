import assert from 'node:assert/strict';
import test from 'node:test';

import { contextLabel } from '../static/context-utils.js';

const text = (tokens) => contextLabel({ tokens, justCompacted: false }).text;

test('thousands are rounded to whole K', () => {
  assert.equal(text(120_002), 'context 120K');
  assert.equal(text(1_499), 'context 1K');
  assert.equal(text(999_499), 'context 999K');
});

test('from a million on, M with one decimal', () => {
  assert.equal(text(1_234_567), 'context 1.2M');
  assert.equal(text(999_500), 'context 1.0M');
});

test('small sizes are shown as they are', () => {
  assert.equal(text(850), 'context 850');
});

test('the hover text explains what the number is', () => {
  assert.equal(contextLabel({ tokens: 120_002, justCompacted: false }).title, '最後一次送出的 context 大小');
});

test('right after compaction there is no number, only the size before it in the hover text', () => {
  assert.deepEqual(contextLabel({ tokens: 341_391, justCompacted: true }), {
    text: 'context 剛壓縮',
    title: '壓縮前 341K，下一次回覆後顯示新的大小',
  });
});
