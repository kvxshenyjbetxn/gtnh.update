import test from 'node:test';
import assert from 'node:assert/strict';
import { app, settings } from './helpers/ui-harness.mjs';

async function reachReview(ui) {
  await ui.click('Продовжити →', ui.document.getElementById('client-step-0-content'));
  await ui.click('▤ Збірка вже завантажена');
  ui.fixture.dialogs.push('/new.zip'); await ui.click('Обрати ZIP…');
  await ui.click('Продовжити →', ui.document.getElementById('client-step-1-content'));
  await ui.click('Продовжити →', ui.document.getElementById('client-step-2-content'));
}
test('enabled empty custom directory is sent to installation and is visible in transfer preview', async () => {
  const ui = await app({ settings: { ...settings(), clientRules: [{ path: 'hui', enabled: true, conflict: 'replace', custom: true }] } });
  try {
    await reachReview(ui);
    const invoke = ui.fixture.invoke.bind(ui.fixture);
    ui.fixture.invoke = async (command, args) => {
      const result = await invoke(command, args);
      if (command === 'prepare_installation') ui.fixture.status.preview.files = [{ path: 'hui', directory: true, exists: false, conflict: 'replace', bytes: 0 }];
      return result;
    };
    await ui.click('Підготувати й перевірити');
    assert.deepEqual(ui.fixture.commands.find(item => item.command === 'prepare_installation').args.request.rules, [{ path: 'hui', enabled: true, conflict: 'replace', custom: true }]);
    assert.match(ui.document.querySelector('.transfer-list').textContent, /hui/);
    assert.equal(ui.document.querySelector('.transfer-list').closest('details').open, true);
    assert.match(ui.document.querySelector('.installation-card').textContent, /Папок:\s*1/);
    await ui.click('Підтвердити перенесення');
    assert.ok(ui.fixture.commands.some(item => item.command === 'finish_installation'));
  } finally { await ui.close(); }
});
test('client review uses saved Prism directory without repeating its path or folder selection in the wizard', async () => {
  const ui = await app();
  try {
    await reachReview(ui);
    const card = ui.document.querySelector('.installation-card');
    assert.doesNotMatch(card.textContent, /\/instances|instances у Prism|Вибрати місце/);
    const wizard = [...ui.document.querySelectorAll('main p, main small')].filter(ui.visible).map(el => el.textContent).join(' ');
    assert.doesNotMatch(wizard, /стандартному вікні|вашої ОС|Використовуються правила|Зберігаємо ваші дані/);
    await ui.click('Підготувати й перевірити');
    assert.equal(ui.fixture.commands.find(item => item.command === 'prepare_installation').args.request.parent, '/instances');
  } finally { await ui.close(); }
});
