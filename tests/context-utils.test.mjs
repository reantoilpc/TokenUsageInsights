import assert from 'node:assert/strict';
import test from 'node:test';

import { contextLabel } from '../static/context-utils.js';

test('thousands are rounded to whole K', () => {
  assert.equal(contextLabel(120_002), 'context 120K');
  assert.equal(contextLabel(1_499), 'context 1K');
  assert.equal(contextLabel(999_499), 'context 999K');
});

test('from a million on, M with one decimal', () => {
  assert.equal(contextLabel(1_234_567), 'context 1.2M');
  assert.equal(contextLabel(999_500), 'context 1.0M');
});

test('small sizes are shown as they are', () => {
  assert.equal(contextLabel(850), 'context 850');
});
