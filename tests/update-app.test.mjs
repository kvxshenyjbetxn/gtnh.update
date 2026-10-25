import test from 'node:test';
import assert from 'node:assert/strict';
import { app, settings, idle } from './helpers/ui-harness.mjs';

async function ready(ui) {
  const { click, document, fixture } = ui;
  await click('Продовжити →', document.getElementById('client-step-0-content'));
  await click('▤ Збірка вже завантажена');
  fixture.dialogs.push('/new.zip');
  await click('Обрати ZIP…');
  await click('Продовжити →', document.getElementById('client-step-1-content'));
  await click('Продовжити →', document.getElementById('client-step-2-content'));
}
test('real app advances accordion, remembers Prism destination and keeps transaction locked across settings navigation', async () => {
  const ui = await app();
  try {
    assert.equal(ui.document.querySelector('#client-step-1-heading button').disabled, true);
    await ready(ui);
    assert.equal(ui.document.activeElement.id, 'client-step-3-heading');
    assert.equal(ui.document.getElementById('client-step-0-content').hidden, true);
    assert.equal(ui.document.getElementById('client-step-3-content').hidden, false);
    assert.match(ui.document.getElementById('client-step-3-content').textContent, /\/instances/);
    await ui.click('Підготувати й перевірити');
    const request = ui.fixture.commands.find(item => item.command === 'prepare_installation').args.request;
    assert.equal(request.parent, '/instances');
    assert.equal(request.source, '/old');
    assert.equal(request.localPath, '/new.zip');
    assert.equal(ui.button('▤Сервер').disabled, true);
    await ui.click('⚙Налаштування');
    assert.match(ui.document.querySelector('[role="status"]').textContent, /Встановлення активне/);
    const theme = ui.document.querySelector('select option[value="light"]').parentElement;
    theme.value = 'light'; theme.dispatchEvent(new ui.dom.window.Event('change', { bubbles: true }));
    await ui.settle();
    assert.equal(ui.document.documentElement.dataset.theme, 'light');
    assert.equal(request.rules[0].enabled, true);
    assert.equal(ui.fixture.listeners.size, 2, 'both catalog subscriptions stay alive');
    await ui.click('До оновлення');
    await ui.click('Скасувати');
    assert.equal(ui.button('▤Сервер').disabled, false);
    await ui.click('Повторити підготовку');
    assert.equal(ui.document.querySelector('.install-name input').value, 'new');
  } finally { await ui.close(); }
});
test('source/pack changes invalidate later readiness, cancelled native selection keeps pack, client/server remain separate', async () => {
  const ui = await app();
  try {
    await ready(ui);
    ui.document.querySelector('#client-step-1-heading button').click(); await ui.settle();
    await ui.click('Обрати ZIP…'); // null = cancel
    assert.match(ui.document.getElementById('client-step-1-content').textContent, /\/new.zip/);
    ui.fixture.dialogs.push('/replacement');
    await ui.click('Обрати папку…');
    assert.equal(ui.document.querySelector('#client-step-3-heading button').disabled, true);
    ui.document.querySelector('#client-step-0-heading button').click(); await ui.settle();
    ui.fixture.dialogs.push('/different-old'); await ui.click('Вибрати папку…');
    assert.equal(ui.document.querySelector('#client-step-1-heading button').disabled, true);
    await ui.click('▤Сервер');
    assert.match(ui.document.getElementById('server-step-0-content').textContent, /\/server/);
    assert.equal(ui.fixture.commands.filter(item => item.command === 'prepare_installation').length, 0);
  } finally { await ui.close(); }
});
test('failed preparation unlocks editing; retry captures changed rules and never reuses stale preview', async () => {
  const ui = await app();
  try {
    await ready(ui);
    const invoke = ui.fixture.invoke.bind(ui.fixture);
    let fail = true;
    ui.fixture.invoke = async (command, args) => {
      if (command === 'prepare_installation' && fail) {
        fail = false;
        ui.fixture.status = { ...idle(1), phase: 'failed', error: 'disk full' };
        throw new Error('disk full');
      }
      return invoke(command, args);
    };
    await ui.click('Підготувати й перевірити');
    assert.match(ui.document.querySelector('.installation-card [role="alert"]').textContent, /disk full/);
    assert.equal(ui.button('▤Сервер').disabled, false);
    ui.document.querySelector('#client-step-2-heading button').click(); await ui.settle();
    const checkbox = ui.document.querySelector('#client-step-2-content input[type="checkbox"]');
    checkbox.click(); await ui.settle();
    assert.equal(ui.document.querySelector('#client-step-3-heading button').disabled, true);
    await ui.click('Продовжити →', ui.document.getElementById('client-step-2-content'));
    await ui.click('Підготувати й перевірити');
    const request = ui.fixture.commands.find(item => item.command === 'prepare_installation').args.request;
    assert.equal(request.rules[0].enabled, false);
    assert.equal(ui.document.querySelectorAll('.transfer-list .rule-row').length, 0);
  } finally { await ui.close(); }
});
test('moved Prism preference is reported and never silently reused or erased', async () => {
  const ui = await app({ settings: { ...settings(), prismInstancesDirectory: '/moved' } });
  try {
    await ready(ui);
    assert.match(ui.document.querySelector('[role="alert"]').textContent, /missing directory/);
    assert.equal(ui.fixture.settings.prismInstancesDirectory, '/moved');
    assert.equal(ui.button('Підготувати й перевірити').disabled, true);
    ui.fixture.dialogs.push('/override'); await ui.click('Вибрати місце');
    await ui.click('Підготувати й перевірити');
    assert.equal(ui.fixture.commands.find(item => item.command === 'prepare_installation').args.request.parent, '/override');
    assert.equal(ui.fixture.settings.prismInstancesDirectory, '/moved');
  } finally { await ui.close(); }
});
