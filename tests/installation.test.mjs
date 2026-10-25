import test from 'node:test';
import assert from 'node:assert/strict';
import { packName, unresolvedConflicts, watchInstallation } from '../src/installation.ts';

test('pack names support URLs, Windows paths, folder paths and uppercase ZIP', () => {
  assert.equal(packName('https://example.test/GTNH_2.8.zip'), 'GTNH_2.8');
  assert.equal(packName('C:\\Downloads\\GTNH.ZIP'), 'GTNH');
  assert.equal(packName('/home/me/New Pack/'), 'New Pack');
  assert.equal(packName('/'), 'GTNH');
});

test('only existing regular files with ask policy require a decision', () => {
  const preview = { destination: '/new', missing: [], files: [
    { path: 'a', exists: true, directory: false, conflict: 'ask', bytes: 1 },
    { path: 'b', exists: false, directory: false, conflict: 'ask', bytes: 1 },
    { path: 'dir', exists: true, directory: true, conflict: 'ask', bytes: 0 },
    { path: 'c', exists: true, directory: false, conflict: 'replace', bytes: 1 },
  ] };
  assert.deepEqual(unresolvedConflicts(preview, {}), ['a']);
  assert.deepEqual(unresolvedConflicts(preview, { a: 'ask' }), ['a']);
  assert.deepEqual(unresolvedConflicts(preview, { a: 'keep' }), []);
  assert.deepEqual(unresolvedConflicts(preview, { a: 'replace' }), []);
});

test('installation watcher rejects stale polls and stops publishing after unmount', async () => {
  const pending = [];
  let tick;
  let stopped = false;
  const seen = [];
  const stop = watchInstallation({
    status: () => new Promise(resolve => pending.push(resolve)),
    repeat: callback => { tick = callback; return () => { stopped = true; }; },
  }, status => seen.push(status), error => assert.fail(String(error)));
  assert.equal(pending.length, 1);
  tick();
  assert.equal(pending.length, 1); // No overlapping filesystem-status calls.
  pending.shift()({ revision: 4, phase: 'preview' });
  await new Promise(resolve => setImmediate(resolve));
  tick();
  pending.shift()({ revision: 3, phase: 'preparing' });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(seen.map(s => s.phase), ['preview']);
  tick();
  stop();
  pending.shift()({ revision: 5, phase: 'completed' });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(stopped, true);
  assert.equal(seen.length, 1);
});
