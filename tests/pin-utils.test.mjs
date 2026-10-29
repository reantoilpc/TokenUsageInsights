import assert from 'node:assert/strict';
import test from 'node:test';

import { orderByPins, parsePins, pinKey, prunePins, togglePin } from '../static/pin-utils.js';

const session = (agent, id) => ({ agent, id });
const keys = (sessions) => sessions.map(pinKey);

test('pinned cards come first in pin order, the rest keep the server order', () => {
  const live = [session('claude', 'a'), session('codex', 'b'), session('claude', 'c'), session('claude', 'd')];
  const ordered = orderByPins(live, ['claude:d', 'codex:b']);
  assert.deepEqual(keys(ordered), ['claude:d', 'codex:b', 'claude:a', 'claude:c']);
});

test('a pin only matches the same agent and id', () => {
  const live = [session('claude', 'a'), session('codex', 'a')];
  assert.deepEqual(keys(orderByPins(live, ['codex:a'])), ['codex:a', 'claude:a']);
});

test('pins for sessions that are not running are ignored when ordering', () => {
  const live = [session('claude', 'a'), session('claude', 'b')];
  assert.deepEqual(keys(orderByPins(live, ['claude:gone', 'claude:b'])), ['claude:b', 'claude:a']);
});

test('two cards with the same key both sit at that pin position', () => {
  const live = [session('claude', 'a'), session('claude', 'dup'), session('claude', 'b'), session('claude', 'dup')];
  const ordered = orderByPins(live, ['claude:b', 'claude:dup']);
  assert.deepEqual(keys(ordered), ['claude:b', 'claude:dup', 'claude:dup', 'claude:a']);
});

test('pinning appends to the end, unpinning leaves the other pins in place', () => {
  assert.deepEqual(togglePin(['claude:a', 'claude:b'], 'codex:c'), ['claude:a', 'claude:b', 'codex:c']);
  assert.deepEqual(togglePin(['claude:a', 'claude:b', 'codex:c'], 'claude:b'), ['claude:a', 'codex:c']);
});

test('a pin is kept while the session is running or recently ended, and dropped once it is in neither', () => {
  const live = [session('claude', 'running')];
  const recent = [session('codex', 'ended')];
  const pins = ['claude:gone', 'codex:ended', 'claude:running', 'claude:ended'];
  assert.deepEqual(prunePins(pins, live, recent), ['codex:ended', 'claude:running']);
});

test('an answer with no sessions at all keeps every pin', () => {
  assert.deepEqual(prunePins(['claude:a', 'codex:b'], [], []), ['claude:a', 'codex:b']);
});

test('stored pins survive missing, corrupt or duplicated data', () => {
  assert.deepEqual(parsePins(null), []);
  assert.deepEqual(parsePins('not json'), []);
  assert.deepEqual(parsePins('{"claude:a":true}'), []);
  assert.deepEqual(parsePins('["claude:a", 1, null, "claude:a", "codex:b"]'), ['claude:a', 'codex:b']);
});
