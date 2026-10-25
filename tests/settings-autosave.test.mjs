import test from 'node:test';
import assert from 'node:assert/strict';
import { app } from './helpers/ui-harness.mjs';

function change(ui, selector, value) {
  const input = ui.document.querySelector(selector);
  input.value = value;
  input.dispatchEvent(new ui.dom.window.Event('change', { bubbles: true }));
}
const writes = ui => ui.fixture.commands.filter(item => item.command === 'save_settings');

test('settings autosave without a Save button and initial load does not write', async () => {
  const ui = await app();
  try {
    assert.equal(writes(ui).length, 0);
    await ui.click('⚙Налаштування');
    assert.equal([...ui.document.querySelectorAll('button')].some(b => b.textContent.trim() === 'Зберегти'), false);
    change(ui, 'select:has(option[value="light"])', 'light'); await ui.settle();
    assert.equal(ui.fixture.settings.theme, 'light');
    ui.document.querySelector('.toggle-label input[type="checkbox"]').click(); await ui.settle();
    assert.equal(ui.fixture.settings.includePrereleases, true);
    ui.fixture.dialogs.push('/portable/instances'); await ui.click('Вибрати папку…');
    assert.equal(ui.fixture.settings.prismInstancesDirectory, '/portable/instances');
    await ui.click('Очистити');
    assert.equal(ui.fixture.settings.prismInstancesDirectory, null);
    change(ui, 'select:has(option[value="en"])', 'en'); await ui.settle();
    assert.equal(ui.fixture.settings.language, 'en');
    assert.match(ui.document.querySelector('footer').textContent, /Saved|All changes saved/);
  } finally { await ui.close(); }
});

test('source and transfer rules autosave outside the settings tab', async () => {
  const ui = await app();
  try {
    ui.fixture.dialogs.push('/another-old'); await ui.click('Вибрати папку…');
    assert.equal(ui.fixture.settings.clientSource, '/another-old');
    await ui.click('Продовжити →', ui.document.getElementById('client-step-0-content'));
    await ui.click('▤ Збірка вже завантажена');
    ui.fixture.dialogs.push('/new.zip'); await ui.click('Обрати ZIP…');
    await ui.click('Продовжити →', ui.document.getElementById('client-step-1-content'));
    ui.document.querySelector('#client-step-2-content input[type="checkbox"]').click(); await ui.settle();
    assert.equal(ui.fixture.settings.clientRules[0].enabled, false);
  } finally { await ui.close(); }
});

test('slow writes are serialized and their responses never overwrite newer edits', async () => {
  const ui = await app();
  let release;
  try {
    const invoke = ui.fixture.invoke.bind(ui.fixture);
    let started = 0;
    ui.fixture.invoke = async (command, args) => {
      if (command === 'save_settings') {
        started++;
        if (started === 1) await new Promise(resolve => { release = resolve; });
      }
      return invoke(command, args);
    };
    await ui.click('⚙Налаштування');
    change(ui, 'select:has(option[value="light"])', 'light'); await ui.settle();
    assert.equal(started, 1);
    assert.equal(ui.document.querySelector('select:has(option[value="en"])').matches(':disabled'), false);
    change(ui, 'select:has(option[value="en"])', 'en'); await ui.settle();
    assert.equal(started, 1, 'one request in flight');
    assert.equal(ui.button('◈Client').disabled, false, 'background saving does not lock navigation');
    release(); await ui.settle();
    assert.equal(started, 2);
    assert.equal(ui.fixture.settings.theme, 'light');
    assert.equal(ui.fixture.settings.language, 'en');
    assert.equal(ui.document.documentElement.lang, 'en');
    assert.equal(ui.document.documentElement.dataset.theme, 'light');
  } finally { release?.(); await ui.close(); }
});

test('failed save stays visible without retry loops and a later edit recovers', async () => {
  const ui = await app();
  try {
    const invoke = ui.fixture.invoke.bind(ui.fixture);
    let attempts = 0;
    ui.fixture.invoke = async (command, args) => {
      if (command === 'save_settings' && ++attempts === 1) throw new Error('disk read-only');
      return invoke(command, args);
    };
    await ui.click('⚙Налаштування');
    change(ui, 'select:has(option[value="light"])', 'light'); await ui.settle();
    assert.match(ui.document.querySelector('[role="alert"]')?.textContent ?? '', /disk read-only/);
    assert.doesNotMatch(ui.document.querySelector('footer').textContent, /Усі зміни збережено|^Збережено$/);
    await ui.settle(); assert.equal(attempts, 1);
    change(ui, 'select:has(option[value="en"])', 'en'); await ui.settle();
    assert.equal(ui.fixture.settings.theme, 'light');
    assert.equal(ui.fixture.settings.language, 'en');
    assert.equal(ui.document.querySelector('[role="alert"]'), null);
  } finally { await ui.close(); }
});

test('failed initial load does not write defaults', async () => {
  let saves = 0;
  const ui = await app({ async invoke(command) {
    if (command === 'save_settings') saves++;
    throw new Error('invalid configuration');
  } });
  try {
    await ui.settle();
    assert.equal(saves, 0);
    assert.match(ui.document.querySelector('[role="alert"]').textContent, /invalid configuration/);
  } finally { await ui.close(); }
});
