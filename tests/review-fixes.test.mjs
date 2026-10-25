import test from 'node:test';
import assert from 'node:assert/strict';
import { app, idle } from './helpers/ui-harness.mjs';

async function review(ui) {
  await ui.click('Продовжити →', ui.document.getElementById('client-step-0-content'));
  await ui.click('▤ Збірка вже завантажена');
  ui.fixture.dialogs.push('/new.zip'); await ui.click('Обрати ZIP…');
  await ui.click('Продовжити →', ui.document.getElementById('client-step-1-content'));
  await ui.click('Продовжити →', ui.document.getElementById('client-step-2-content'));
}
async function until(predicate) {
  const deadline = Date.now() + 2500;
  while (!predicate()) {
    if (Date.now() > deadline) assert.fail('UI did not publish updated installation status');
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}
test('unchanged transfer navigation preserves operation-only destination and custom folder name', async () => {
  const ui = await app();
  try {
    await review(ui);
    ui.fixture.dialogs.push('/my-destination'); await ui.click('Вибрати місце');
    const input = ui.document.querySelector('.install-name input');
    input.value = 'My custom instance';
    input.dispatchEvent(new ui.dom.window.Event('input', { bubbles: true }));
    await ui.settle();
    ui.document.querySelector('#client-step-2-heading button').click(); await ui.settle();
    await ui.click('Продовжити →', ui.document.getElementById('client-step-2-content'));
    assert.equal(ui.document.querySelector('.install-name input').value, 'My custom instance');
    await ui.click('Підготувати й перевірити');
    const request = ui.fixture.commands.find(item => item.command === 'prepare_installation').args.request;
    assert.equal(request.parent, '/my-destination');
    assert.equal(request.name, 'My custom instance');
  } finally { await ui.close(); }
});
test('expanded section really collapses and reopening retains selection and catalog subscriptions', async () => {
  const ui = await app();
  try {
    await review(ui);
    const header = ui.document.querySelector('#client-step-1-heading button');
    header.click(); await ui.settle();
    assert.equal(header.getAttribute('aria-expanded'), 'true');
    header.click(); await ui.settle();
    assert.equal(header.getAttribute('aria-expanded'), 'false');
    assert.equal(ui.document.getElementById('client-step-1-content').hidden, true);
    assert.equal(ui.fixture.listeners.size, 2);
    header.click(); await ui.settle();
    assert.match(ui.document.getElementById('client-step-1-content').textContent, /\/new.zip/);
  } finally { await ui.close(); }
});
test('settings shows actual phase/progress and completion or failure while watcher stays mounted', async () => {
  const ui = await app();
  try {
    await review(ui); await ui.click('Підготувати й перевірити');
    await ui.click('⚙Налаштування');
    ui.fixture.status = { ...idle(2), phase: 'copying', progress: { stage: 'copying', done: 3, total: 10 } };
    await until(() => [...ui.document.querySelectorAll('[role="status"]')].some(el => ui.visible(el) && /3\s*\/\s*10/.test(el.textContent)));
    const progress = [...ui.document.querySelectorAll('progress')].find(ui.visible);
    assert.equal(progress.value, 3); assert.equal(progress.max, 10);
    ui.fixture.status = { ...idle(3), phase: 'completed', destination: '/my-destination/new' };
    await until(() => [...ui.document.querySelectorAll('[role="status"]')].some(el => ui.visible(el) && el.textContent.includes('/my-destination/new')));
    ui.fixture.status = { ...idle(4), phase: 'failed', error: 'disk disconnected' };
    await until(() => [...ui.document.querySelectorAll('[role="alert"]')].some(el => ui.visible(el) && el.textContent.includes('disk disconnected')));
  } finally { await ui.close(); }
});
