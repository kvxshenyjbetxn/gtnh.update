import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as downloadModule from '../src/downloads.ts';
const { watchDownloads, downloadsForArtifacts } = downloadModule;

function deferred() {
  let resolve;
  const promise = new Promise(r => { resolve = r; });
  return { promise, resolve };
}
const CLIENT = 'https://downloads.gtnewhorizons.com/Multi_mc_downloads/client.zip';
const SERVER = 'https://downloads.gtnewhorizons.com/ServerPacks/server.zip';
const OTHER = 'https://downloads.gtnewhorizons.com/Multi_mc_downloads/java8.zip';
const status = (url, revision, overrides = {}) => ({ id: `job-${url}`, revision, url, state: 'downloading', received: revision, total: null, path: null, error: null, ...overrides });
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

test('late snapshots merge independent jobs; out-of-order events only affect their own URL', async () => {
  const query = deferred();
  let event;
  let removed = 0;
  let ready = false;
  let latest = [];
  const stop = watchDownloads({
    listen: async callback => { event = callback; return () => { removed++; }; },
    status: () => query.promise,
  }, value => { latest = value; }, error => assert.fail(String(error)), () => { ready = true; });
  await tick();
  event(status(CLIENT, 10));
  query.resolve([status(CLIENT, 1), status(SERVER, 2)]);
  await tick();
  assert.equal(latest.length, 2);
  assert.equal(latest.find(s => s.url === CLIENT).received, 10);
  assert.equal(latest.find(s => s.url === SERVER).received, 2);
  // A global high-water revision would incorrectly drop server progress.
  event(status(SERVER, 4));
  event(status(CLIENT, 9));
  event(status(SERVER, 3));
  assert.equal(latest.find(s => s.url === CLIENT).received, 10);
  assert.equal(latest.find(s => s.url === SERVER).received, 4);
  assert.equal(ready, true);
  stop();
  assert.equal(removed, 1);
  event(status(CLIENT, 11));
  assert.equal(latest.find(s => s.url === CLIENT).received, 10);
});

test('retry replaces only its file and stale terminal events cannot restore the old attempt', async () => {
  let event;
  let latest = [];
  const stop = watchDownloads({
    listen: async callback => { event = callback; return () => {}; },
    status: async () => [status(CLIENT, 3, { state: 'failed' }), status(SERVER, 2)],
  }, value => { latest = value; }, error => assert.fail(String(error)), () => {});
  await tick();
  event(status(CLIENT, 5, { id: 'retry', received: 0 }));
  event(status(CLIENT, 4, { state: 'failed' }));
  assert.equal(latest.length, 2);
  assert.equal(latest.find(s => s.url === CLIENT).id, 'retry');
  assert.equal(latest.find(s => s.url === CLIENT).state, 'downloading');
  assert.equal(latest.find(s => s.url === SERVER).received, 2);
  stop();
});

test('unmount before subscription resolves removes the listener and skips query', async () => {
  const subscription = deferred();
  let removed = 0;
  let queried = false;
  const stop = watchDownloads({
    listen: () => subscription.promise,
    status: async () => { queried = true; return []; },
  }, () => assert.fail('disposed update'), () => assert.fail('disposed error'), () => assert.fail('disposed ready'));
  stop();
  subscription.resolve(() => { removed++; });
  await tick();
  assert.equal(removed, 1);
  assert.equal(queried, false);
});

test('remount rehydrates all files; failed initialization is surfaced', async () => {
  let latest = [];
  const errors = [];
  const transport = { listen: async () => () => {}, status: async () => [status(CLIENT, 8), status(SERVER, 9)] };
  const mount = () => watchDownloads(transport, value => { latest = value; }, error => errors.push(error), () => {});
  const stop = mount();
  await tick();
  assert.equal(latest.length, 2);
  stop(); latest = [];
  const stopAgain = mount();
  await tick();
  assert.deepEqual(latest.map(s => s.received), [8, 9]);
  stopAgain();
  const stopFailed = watchDownloads({ listen: async () => { throw new Error('listener failed'); }, status: async () => [] }, () => {}, error => errors.push(String(error)), () => assert.fail('not ready'));
  await tick();
  assert.match(errors[0], /listener failed/);
  stopFailed();
});

test('each release row shows only its own files and current mode, keeping both Java variants visible', () => {
  const states = [status(CLIENT, 3), status(SERVER, 2), status(OTHER, 4)];
  const artifacts = [
    { mode: 'client', java: 'Java 17', url: CLIENT },
    { mode: 'server', java: 'Java 17', url: SERVER },
    { mode: 'client', java: 'Java 8', url: OTHER },
  ];
  assert.deepEqual(downloadsForArtifacts(states, artifacts, 'client').map(job => [job.artifact.java, job.status.url]), [['Java 17', CLIENT], ['Java 8', OTHER]]);
  assert.deepEqual(downloadsForArtifacts(states, artifacts, 'server').map(job => job.status.url), [SERVER]);
  assert.deepEqual(downloadsForArtifacts(states, [{ mode: 'client', java: 'Java 17', url: 'another-release.zip' }], 'client'), []);
});
