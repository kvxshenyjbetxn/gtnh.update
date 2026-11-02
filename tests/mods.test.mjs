import test from 'node:test';
import assert from 'node:assert/strict';
import { app } from './helpers/ui-harness.mjs';
import { upsertModRule, modRuleFrom } from '../src/mod-rules.ts';

async function toStep3(ui, mode) {
  const { click, document, fixture } = ui;
  if (mode === 'server') await click('▤Сервер');
  await click('Продовжити →', document.getElementById(`${mode}-step-0-content`));
  await click('▤ Збірка вже завантажена', document.getElementById(`${mode}-step-1-content`));
  fixture.dialogs.push('/pack.zip');
  await click('Обрати ZIP…', document.getElementById(`${mode}-step-1-content`));
  await click('Продовжити →', document.getElementById(`${mode}-step-1-content`));
}
async function toReview(ui, mode) { await toStep3(ui, mode); await ui.click('Продовжити →', ui.document.getElementById(`${mode}-step-2-content`)); }
const card = ui => ui.document.querySelector('.installation-card').textContent;
const cmd = (ui, name) => ui.fixture.commands.filter(item => item.command === name);

test('upsertModRule replaces the same identity and keeps different mods', () => {
  const a = modRuleFrom({ name: 'A', modIds: ['a'], stem: 'a-1', file: '/a-1.jar' }, 'add');
  const a2 = modRuleFrom({ name: 'A', modIds: ['a'], stem: 'a-2', file: '/a-2.jar' }, 'add');
  const b = modRuleFrom({ name: 'B', modIds: [], stem: 'b', file: '/b.jar' }, 'add');
  const b2 = modRuleFrom({ name: 'B', modIds: [], stem: 'B', file: '/other/B.jar' }, 'add');
  let r = upsertModRule([], a); r = upsertModRule(r.rules, b);
  assert.equal(r.rules.length, 2);
  r = upsertModRule(r.rules, a2); assert.equal(r.replaced, true);
  assert.deepEqual(r.rules.map(x => x.file), ['/a-2.jar', '/b.jar']);
  r = upsertModRule(r.rules, b2); assert.deepEqual(r.rules.map(x => x.file), ['/a-2.jar', '/other/B.jar']);
  assert.equal(upsertModRule([a], modRuleFrom({ name: 'A', modIds: ['a'], stem: 'a', file: '/x' }, 'remove')).rules.length, 2);
});

test('client: add mods inspects each jar, replaces the same mod and sends mods in the request', async () => {
  const ui = await app();
  try {
    await toStep3(ui, 'client');
    ui.fixture.dialogs.push(['/d/alpha-1.0.jar', '/d/beta-2.3.jar']);
    await ui.click('+ Додати мод');
    const open = cmd(ui, 'open').at(-1).args;
    assert.equal(open.multiple, true);
    assert.deepEqual(JSON.parse(JSON.stringify(open.filters)), [{ name: 'Minecraft mod', extensions: ['jar'] }]);
    assert.deepEqual(cmd(ui, 'inspect_mod').map(c => c.args.path), ['/d/alpha-1.0.jar', '/d/beta-2.3.jar']);
    assert.equal(ui.fixture.settings.clientMods.length, 2);
    assert.equal(ui.fixture.settings.clientMods[0].file, '/d/alpha-1.0.jar');
    ui.fixture.dialogs.push(['/e/alpha-1.1.jar']);
    await ui.click('+ Додати мод');
    const mods = ui.fixture.settings.clientMods;
    assert.deepEqual(mods.map(m => m.file), ['/e/alpha-1.1.jar', '/d/beta-2.3.jar']);
    assert.match(ui.document.getElementById('client-step-2-content').textContent, /Правило для цього мода оновлено/);
    await ui.click('Продовжити →', ui.document.getElementById('client-step-2-content'));
    await ui.click('Підготувати й перевірити');
    const request = cmd(ui, 'prepare_installation')[0].args.request;
    assert.equal(request.mods.length, 2);
    assert.equal(request.mods[0].action, 'add');
  } finally { await ui.close(); }
});

test('client: remove-mod rule has no file, opens mods folder and invalidates the prepared request', async () => {
  const ui = await app();
  try {
    await toReview(ui, 'client');
    ui.document.querySelector('#client-step-2-heading button').click(); await ui.settle();
    ui.fixture.dialogs.push('/old/mods/journeymap-1.7.10-5.2.19.jar');
    await ui.click('− Прибрати мод');
    assert.equal(cmd(ui, 'open').at(-1).args.defaultPath, '/old/mods');
    assert.equal(cmd(ui, 'open').at(-1).args.multiple, false);
    const rule = ui.fixture.settings.clientMods[0];
    assert.equal(rule.action, 'remove'); assert.equal(rule.file, null);
    assert.equal(ui.document.querySelector('.installation-card'), null);
  } finally { await ui.close(); }
});

test('inspect_mod failure is shown and nothing is stored', async () => {
  const ui = await app();
  try {
    await toStep3(ui, 'client');
    ui.fixture.inspected['/d/readme.jar'] = null;
    ui.fixture.dialogs.push(['/d/readme.jar']);
    await ui.click('+ Додати мод');
    assert.deepEqual(ui.fixture.settings.clientMods, []);
    assert.match(ui.document.getElementById('client-step-2-content').textContent, /not a mod/);
  } finally { await ui.close(); }
});

test('mod without ids is shown by name with a hint', async () => {
  const ui = await app();
  try {
    await toStep3(ui, 'client');
    ui.fixture.inspected['/d/odd-1.jar'] = { name: 'odd', modIds: [], stem: 'odd-1', file: '/d/odd-1.jar' };
    ui.fixture.dialogs.push(['/d/odd-1.jar']);
    await ui.click('+ Додати мод');
    const text = ui.document.getElementById('client-step-2-content').textContent;
    assert.match(text, /за назвою: odd-1/);
    assert.match(text, /менш надійний/);
  } finally { await ui.close(); }
});

const plan = {
  removed: [{ name: 'JourneyMap', files: ['mods/journeymap-1.7.10-5.2.19.jar'] }, { name: 'Ghost', files: [] }],
  added: [{ name: 'Alpha', file: 'mods/alpha-1.1.jar', source: '/e/alpha-1.1.jar', replaces: ['mods/alpha-1.0.jar'], missing: false }, { name: 'Lost', file: 'mods/lost.jar', source: '/e/lost.jar', replaces: [], missing: true }],
  duplicates: [['mods/dup-1.jar', 'mods/dup-2.jar']],
};
function assertPlan(text) {
  assert.match(text, /Мої моди/);
  assert.match(text, /JourneyMap/); assert.match(text, /mods\/journeymap-1\.7\.10-5\.2\.19\.jar/);
  assert.match(text, /Ghost/); assert.match(text, /не знайдено файлів цього мода/);
  assert.match(text, /замінює: mods\/alpha-1\.0\.jar/);
  assert.match(text, /\/e\/alpha-1\.1\.jar/);
  assert.match(text, /Файл не знайдено, мод не буде додано/);
  assert.match(text, /Forge не запуститься з дублікатами/);
  assert.match(text, /mods\/dup-1\.jar · mods\/dup-2\.jar/);
}
test('client preview renders the mod plan', async () => {
  const ui = await app();
  try {
    ui.fixture.clientPlan = plan;
    await toReview(ui, 'client');
    await ui.click('Підготувати й перевірити');
    assertPlan(card(ui));
  } finally { await ui.close(); }
});

test('server preview renders the mod plan', async () => {
  const ui = await app();
  try {
    ui.fixture.serverPreview.mods = plan;
    await toReview(ui, 'server');
    await ui.click('Підготувати оновлення');
    assertPlan(card(ui));
  } finally { await ui.close(); }
});

test('server: rules and mods are sent, preview lists files and Ask conflicts gate confirmation', async () => {
  const ui = await app();
  try {
    ui.fixture.serverPreview.mods = { removed: [{ name: 'Old', files: ['mods/old-1.jar'] }], added: [], duplicates: [] };
    ui.fixture.serverPreview.files = [{ path: 'config/serverutilities.cfg', exists: true, directory: false, conflict: 'ask', bytes: 10 }];
    ui.fixture.serverPreview.missing = ['config/gone.cfg'];
    await toStep3(ui, 'server');
    ui.fixture.dialogs.push(['/server/config/serverutilities.cfg']);
    await ui.click('+ Додати файл');
    ui.fixture.dialogs.push(['/d/alpha-1.0.jar']);
    await ui.click('+ Додати мод');
    assert.equal(ui.fixture.settings.serverMods.length, 1);
    assert.equal(ui.fixture.settings.serverRules.length, 1);
    assert.deepEqual(ui.fixture.settings.clientMods, []);
    await ui.click('Продовжити →', ui.document.getElementById('server-step-2-content'));
    await ui.click('Підготувати оновлення');
    const request = cmd(ui, 'prepare_server_update')[0].args.request;
    assert.equal(request.rules.length, 1); assert.equal(request.mods.length, 1);
    const text = card(ui);
    assert.match(text, /old-1\.jar/);
    assert.match(text, /config\/gone\.cfg/);
    assert.match(text, /Потрібне ваше рішення/);
    assert.equal(ui.button('Оновити сервер').disabled, true);
    const select = ui.document.querySelector('.installation-card select');
    select.value = 'replace'; select.dispatchEvent(new ui.dom.window.Event('change', { bubbles: true })); await ui.settle();
    assert.equal(ui.button('Оновити сервер').disabled, false);
    await ui.click('Оновити сервер');
    assert.deepEqual(cmd(ui, 'finish_installation')[0].args.decisions, [{ path: 'config/serverutilities.cfg', conflict: 'replace' }]);
  } finally { await ui.close(); }
});
