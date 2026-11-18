import assert from 'node:assert/strict';
import test from 'node:test';

import { ago } from '../static/relative-time-utils.js';

const SECOND = 1000;
const HOUR = 3600 * SECOND;

test('relative time goes from seconds to minutes to hours to days', () => {
  assert.equal(ago(-5 * SECOND), '0s 前');
  assert.equal(ago(59 * SECOND), '59s 前');
  assert.equal(ago(60 * SECOND), '1m 前');
  assert.equal(ago(59 * 60 * SECOND), '59m 前');
  assert.equal(ago(HOUR), '1h 前');
  assert.equal(ago(23 * HOUR + 59 * 60 * SECOND), '23h 前');
  assert.equal(ago(24 * HOUR), '1d 前');
  assert.equal(ago(3 * 24 * HOUR + 5 * HOUR), '3d 前');
});
