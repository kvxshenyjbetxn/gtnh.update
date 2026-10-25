import test from 'node:test';
import assert from 'node:assert/strict';
import { createDraft, changeSource, changePack, continueStep, openStep, buildInstallRequest, transitionDraft, transactionBusy } from '../src/update-flow.ts';

function ready() {
  let d = continueStep(createDraft('/old'));
  d = continueStep(changePack(d, { localPath: '/new.zip' }));
  return continueStep(d);
}
test('forward sections stay locked until explicit completion, even with a remembered source', () => {
  const d = createDraft('/old');
  assert.equal(openStep(d, 2).step, 0);
  assert.equal(continueStep(createDraft(null)).completed, 0);
  assert.equal(continueStep(d).step, 1);
  assert.equal(continueStep(continueStep(d)).step, 1);
  assert.equal(buildInstallRequest(d, true, []), null);
  assert.equal(continueStep(changePack(continueStep(d), { url: 'https://example.test/a.zip' })).step, 2);
});
test('opening completed sections preserves values, source changes invalidate downstream, modes are independent', () => {
  const client = ready();
  const server = createDraft('/server');
  assert.deepEqual(openStep(client, 0), { ...client, step: 0 });
  assert.deepEqual(changeSource(client, '/other'), { source: '/other', pack: null, step: 0, completed: 0 });
  assert.deepEqual(server, { source: '/server', pack: null, step: 0, completed: 0 });
  assert.equal(client.source, '/old');
  const replacement = changePack(client, { localPath: '/different' });
  assert.equal(replacement.completed, 1);
  assert.equal(replacement.step, 1);
  assert.equal(buildInstallRequest(replacement, true, []), null);
});
test('request captures rules deeply only after transfer step is explicitly completed', () => {
  const rule = { path: 'saves', enabled: true, conflict: 'replace', custom: false };
  const rules = [rule];
  const d = ready();
  const request = buildInstallRequest(d, true, rules);
  rule.enabled = false; rules.push({ ...rule, path: 'options.txt' });
  assert.deepEqual(request, { client: true, source: '/old', localPath: '/new.zip', rules: [{ path: 'saves', enabled: true, conflict: 'replace', custom: false }] });
  const downloading = changePack(d, { url: 'https://example.test/a.zip' });
  assert.equal(buildInstallRequest(downloading, false, []), null);
});
test('active transaction freezes draft transitions including initial sending, failures and cancelled preview unlock them', () => {
  const d = ready();
  for (const phase of ['preparing', 'preview', 'copying']) {
    assert.equal(transactionBusy(phase, false), true);
    assert.equal(transitionDraft(d, { kind: 'source', source: '/other' }, true), d);
    assert.equal(transitionDraft(d, { kind: 'pack', pack: { localPath: '/other.zip' } }, true), d);
  }
  assert.equal(transactionBusy('', true), true);
  for (const phase of ['', 'failed', 'completed']) assert.equal(transactionBusy(phase, false), false);
  assert.equal(transitionDraft(d, { kind: 'source', source: '/other' }, false).completed, 0);
});
