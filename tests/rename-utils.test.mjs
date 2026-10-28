import assert from 'node:assert/strict';
import test from 'node:test';

import { renameCommand } from '../static/rename-utils.js';

test('a valid name becomes the /rename command, trimmed', () => {
  assert.deepEqual(renameCommand('  修正登入頁 '), { command: '/rename 修正登入頁' });
});

test('an empty or blank name is refused', () => {
  for (const name of ['', '   ']) {
    const result = renameCommand(name);
    assert.equal(result.command, undefined, JSON.stringify(name));
    assert.match(result.error, /名稱/);
  }
});

test('names are limited to 200 characters, the same as Claude Code', () => {
  assert.equal(renameCommand('名'.repeat(200)).command, `/rename ${'名'.repeat(200)}`);
  const tooLong = renameCommand('名'.repeat(201));
  assert.equal(tooLong.command, undefined);
  assert.match(tooLong.error, /200/);
});

test('line breaks, control characters and invisible characters are refused', () => {
  const cases = {
    newline: '修正\n登入頁',
    tab: '修正\t登入頁',
    'zero-width space': '修正​登入頁',
    'right-to-left override': '修正‮登入頁',
    'line separator': '修正 登入頁',
  };
  for (const [label, name] of Object.entries(cases)) {
    const result = renameCommand(name);
    assert.equal(result.command, undefined, label);
    assert.ok(result.error, label);
  }
});

test('a name ending with a backslash is refused, a backslash inside is fine', () => {
  const trailing = renameCommand('C:\\work\\');
  assert.equal(trailing.command, undefined);
  assert.match(trailing.error, /反斜線/);
  assert.equal(renameCommand('C:\\work 修正').command, '/rename C:\\work 修正');
});
