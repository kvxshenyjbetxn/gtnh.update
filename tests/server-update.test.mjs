import test from 'node:test';
import assert from 'node:assert/strict';
import { app, settings } from './helpers/ui-harness.mjs';

async function ready(ui, dialog = '/server-pack.zip') {
  const { click, document, fixture } = ui;
  await click('▤Сервер');
  await click('Продовжити →', document.getElementById('server-step-0-content'));
  await click('▤ Збірка вже завантажена', document.getElementById('server-step-1-content'));
  fixture.dialogs.push(dialog);
  await click('Обрати ZIP…', document.getElementById('server-step-1-content'));
  await click('Продовжити →', document.getElementById('server-step-1-content'));
  await click('Продовжити →', document.getElementById('server-step-2-content'));
}
test('settings default to empty server rules and mod rules', () => {
  const s = settings();
  assert.deepEqual([s.serverRules, s.clientMods, s.serverMods], [[], [], []]);
});
test('server flow prepares an in-place update with local pack and server folder, without destination picker', async () => {
  const ui = await app();
  try {
    await ready(ui);
    const card = ui.document.querySelector('#server-step-3-content .installation-card');
    assert.doesNotMatch(card.textContent, /Назва|Вибрати місце/);
    assert.match(ui.document.getElementById('server-step-2-content').textContent, /config/);
    assert.equal(ui.document.querySelectorAll('#server-step-2-content .rules-details').length, 1);
    assert.match(ui.document.getElementById('server-step-2-content').textContent, /Перенести зі старого сервера/);
    await ui.click('Підготувати оновлення');
    const request = ui.fixture.commands.find(item => item.command === 'prepare_server_update').args.request;
    assert.deepEqual(request, { server: '/server', localPath: '/server-pack.zip', rules: [], mods: [] });
    assert.equal(ui.fixture.commands.some(item => item.command === 'prepare_installation'), false);
    const text = ui.document.querySelector('#server-step-3-content .installation-card').textContent;
    assert.match(text, /Замінити/);
    assert.match(text, /Прибрати в резервну копію/);
    assert.match(text, /Додати/);
    assert.match(text, /Java 17/);
    assert.match(text, /Попереднє оновлення було перервано. Зміни скасовано/);
    assert.match(text, /\/server\.gtnh-backup/);
    assert.match(text, /Попередню резервну копію буде замінено/);
    const scripts = [...ui.document.querySelectorAll('#server-step-3-content details pre')].map(el => el.textContent);
    assert.deepEqual(scripts, ['java -Xmx4G', 'java -Xmx6G', 'old args']);
    await ui.click('Оновити сервер');
    const finish = ui.fixture.commands.find(item => item.command === 'finish_installation');
    assert.deepEqual(finish.args.decisions, []);
    const done = ui.document.querySelector('#server-step-3-content .installation-card').textContent;
    assert.match(done, /Сервер оновлено/);
    assert.match(done, /\/server\.gtnh-backup/);
  } finally { await ui.close(); }
});
