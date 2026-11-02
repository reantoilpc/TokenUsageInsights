import assert from 'node:assert/strict';
import test from 'node:test';

import { pinKey } from '../static/pin-utils.js';
import { orderCards, parsePriorities, priorityOf, prunePriorities, setPriority } from '../static/priority-utils.js';

const session = (agent, id, status = 'idle') => ({ agent, id, status });
const keys = (sessions) => sessions.map(pinKey);

test('within one status, high cards come first and low cards last; the rest keep the server order', () => {
  const live = [
    session('claude', 'a', 'waiting'),
    session('claude', 'b', 'waiting'),
    session('claude', 'c', 'working'),
    session('codex', 'd', 'working'),
    session('claude', 'e', 'idle'),
    session('claude', 'f', 'idle'),
    session('claude', 'g', 'idle'),
    session('claude', 'h', 'idle'),
  ];
  const priorities = { 'claude:b': 'high', 'claude:c': 'low', 'claude:f': 'low', 'claude:h': 'high' };
  assert.deepEqual(keys(orderCards(live, [], priorities)), [
    'claude:b', 'claude:a',
    'codex:d', 'claude:c',
    'claude:h', 'claude:e', 'claude:g', 'claude:f',
  ]);
});

test('priority never moves a card ahead of a status the server put first', () => {
  const live = [session('claude', 'asking', 'waiting'), session('claude', 'busy', 'working')];
  const priorities = { 'claude:asking': 'low', 'claude:busy': 'high' };
  assert.deepEqual(keys(orderCards(live, [], priorities)), ['claude:asking', 'claude:busy']);
});

test('pinned cards stay first in pin order whatever their priority', () => {
  const live = [session('claude', 'a', 'waiting'), session('claude', 'b', 'idle'), session('claude', 'c', 'idle')];
  const priorities = { 'claude:a': 'high', 'claude:b': 'low', 'claude:c': 'high' };
  assert.deepEqual(keys(orderCards(live, ['claude:b'], priorities)), ['claude:b', 'claude:a', 'claude:c']);
});

test('a priority only matches the same agent and id', () => {
  const priorities = { 'codex:a': 'high' };
  assert.equal(priorityOf(priorities, session('codex', 'a')), 'high');
  assert.equal(priorityOf(priorities, session('claude', 'a')), null);
});

test('choosing a level replaces the old one, normal removes it, and the stored map is not changed in place', () => {
  const stored = { 'claude:a': 'high' };
  assert.deepEqual(setPriority(stored, 'claude:a', 'low'), { 'claude:a': 'low' });
  assert.deepEqual(setPriority(stored, 'codex:b', 'high'), { 'claude:a': 'high', 'codex:b': 'high' });
  assert.deepEqual(setPriority(stored, 'claude:a', null), {});
  assert.deepEqual(stored, { 'claude:a': 'high' });
});

test('a priority is kept while the session is running or recently ended, and dropped once it is in neither', () => {
  const live = [session('claude', 'running')];
  const recent = [session('codex', 'ended')];
  const priorities = { 'claude:gone': 'high', 'codex:ended': 'low', 'claude:running': 'high' };
  assert.deepEqual(prunePriorities(priorities, live, recent), { 'codex:ended': 'low', 'claude:running': 'high' });
});

test('an answer with no sessions at all keeps every priority', () => {
  assert.deepEqual(prunePriorities({ 'claude:a': 'high' }, [], []), { 'claude:a': 'high' });
});

test('stored priorities survive missing or corrupt data', () => {
  assert.deepEqual(parsePriorities(null), {});
  assert.deepEqual(parsePriorities('not json'), {});
  assert.deepEqual(parsePriorities('["claude:a"]'), {});
  assert.deepEqual(parsePriorities('"high"'), {});
  assert.deepEqual(
    parsePriorities('{"claude:a":"high","claude:b":"urgent","claude:c":1,"claude:e":"toString","codex:d":"low"}'),
    { 'claude:a': 'high', 'codex:d': 'low' },
  );
});
