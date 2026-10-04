import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

test('documentation structure', () => {
  const example = dirname(fileURLToPath(import.meta.url));
  const command = process.env.PERFECT_DOC || 'perfect-doc';
  const result = spawnSync(command, ['check', resolve(example, 'docs')], {
    cwd: example,
    encoding: 'utf8',
  });

  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.status, 0, result.stderr || result.stdout);
});
