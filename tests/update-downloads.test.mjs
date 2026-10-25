import test from 'node:test';
import assert from 'node:assert/strict';
import { app } from './helpers/ui-harness.mjs';

test('rehydrated active downloads stay visible in collapsed and prerequisite-locked pack summaries', async () => {
  const url = 'https://downloads.gtnewhorizons.com/Multi_mc_downloads/GTNH.zip';
  const ui = await app({
    releases: [{ version: '2.8.0', channel: 'stable', date: '2026-10-01', pageUrl: 'https://example.test', artifacts: [{ mode: 'client', java: 'Java 21', url }] }],
    downloads: [{ id: 'one', revision: 1, url, state: 'downloading', received: 20, total: 100, path: null, error: null }],
  });
  try {
    assert.match(ui.document.getElementById('client-step-1-heading').textContent, /20%/);
    await ui.click('Продовжити →', ui.document.getElementById('client-step-0-content'));
    ui.document.querySelector('#client-step-0-heading button').click(); await ui.settle();
    assert.match(ui.document.getElementById('client-step-1-heading').textContent, /20%/);
    await ui.click('⚙Налаштування');
    assert.equal(ui.fixture.listeners.size, 2);
    await ui.click('◈Клієнт');
    assert.match(ui.document.getElementById('client-step-1-heading').textContent, /20%/);
  } finally { await ui.close(); }
});
