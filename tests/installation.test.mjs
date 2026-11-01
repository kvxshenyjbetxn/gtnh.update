import test from 'node:test';
import assert from 'node:assert/strict';
import { groupTransfers, packName, unresolvedConflicts, watchInstallation } from '../src/installation.ts';

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

test('installation watcher sends last accepted revision and ignores unchanged (null) responses', async () => {
  const pending = [];
  const requested = [];
  let tick;
  const seen = [];
  const stop = watchInstallation({
    status: since => new Promise(resolve => { requested.push(since); pending.push(resolve); }),
    repeat: callback => { tick = callback; return () => {}; },
  }, status => seen.push(status), error => assert.fail(String(error)));
  pending.shift()({ revision: 2, phase: 'preview' });
  await new Promise(resolve => setImmediate(resolve));
  tick();
  pending.shift()(null);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(requested, [undefined, 2]);
  assert.deepEqual(seen.map(s => s.revision), [2]);
  stop();
});

test('top-level transfer groups summarize descendants and stay fast for 12000 entries', () => {
  const dir = (path) => ({ path, exists: true, directory: true, conflict: 'replace', bytes: 0 });
  const file = (path, bytes) => ({ path, exists: false, directory: false, conflict: 'replace', bytes });
  const small = groupTransfers([dir('saves'), dir('saves/a'), file('saves/a/x', 5), file('saves/y', 7), file('options.txt', 3)]);
  assert.deepEqual(small.map(g => [g.path, g.directory, g.files, g.bytes]), [['saves', true, 2, 12], ['options.txt', false, 1, 3]]);
  const big = [dir('saves')];
  for (let i = 0; i < 12000; i++) big.push(file(`saves/w${i % 50}/f${i}`, 1));
  const started = performance.now();
  const groups = groupTransfers(big);
  assert.ok(performance.now() - started < 500);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].files, 12000);
});
