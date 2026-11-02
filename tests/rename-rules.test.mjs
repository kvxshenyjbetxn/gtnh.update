import test from 'node:test';
import assert from 'node:assert/strict';
import { app } from './helpers/ui-harness.mjs';
import { defaultRenameTarget, renameRuleFor, validRenameTarget, renamesValid } from '../src/rename-rules.ts';

const P = 'config/txloader/load/BetterQuesting[betterquesting]/lang/ru_RU.lang';
const cmd = (ui, name) => ui.fixture.commands.filter(item => item.command === name);

async function toStep3(ui) {
  const { click, document, fixture } = ui;
  await click('Продовжити →', document.getElementById('client-step-0-content'));
  await click('▤ Збірка вже завантажена', document.getElementById('client-step-1-content'));
  fixture.dialogs.push('/pack.zip');
  await click('Обрати ZIP…', document.getElementById('client-step-1-content'));
  await click('Продовжити →', document.getElementById('client-step-1-content'));
}

test('default rename target swaps ru_RU for en_US and otherwise keeps the name', () => {
  assert.equal(defaultRenameTarget(P), 'en_US.lang');
  assert.equal(defaultRenameTarget('config/a/other.lang'), 'other.lang');
  assert.deepEqual(renameRuleFor(P), { path: P, to: 'en_US.lang', enabled: true });
});

test('rename target validity: non-empty, one component, different from the file name', () => {
  assert.equal(validRenameTarget({ path: P, to: 'en_US.lang' }), true);
  assert.equal(validRenameTarget({ path: P, to: '' }), false);
  assert.equal(validRenameTarget({ path: P, to: 'a/b.lang' }), false);
  assert.equal(validRenameTarget({ path: P, to: 'a\\b.lang' }), false);
  assert.equal(validRenameTarget({ path: P, to: 'RU_ru.LANG' }), false);
  assert.equal(validRenameTarget({ path: 'other.lang', to: 'other.lang' }), false);
  assert.equal(renamesValid([{ path: P, to: 'en_US.lang', enabled: true }, { path: P, to: '', enabled: false }]), false);
});

test('client: add rename file, invalid name is not autosaved, valid name is saved and sent in the request', async () => {
  const ui = await app();
  try {
    await toStep3(ui);
    const section = [...ui.document.querySelectorAll('fieldset')].find(f => f.textContent.includes('Перейменування файлів'));
    ui.fixture.dialogs.push(['/old/extra.cfg']);
    await ui.click('+ Додати файл', section);
    // The default name equals the file name, so the row is invalid and nothing is saved yet.
    assert.match(section.textContent, /Вкажіть нову назву/);
    assert.deepEqual(ui.fixture.settings.clientRenames, []);
    const edit = value => {
      const input = ui.document.querySelector('input[aria-label="Нова назва: extra.cfg"]');
      input.value = value; input.dispatchEvent(new ui.dom.window.Event('input', { bubbles: true }));
      input.dispatchEvent(new ui.dom.window.Event('change', { bubbles: true }));
      return ui.settle();
    };
    await edit('bad/name');
    assert.deepEqual(ui.fixture.settings.clientRenames, []);
    await edit('extra.old');
    assert.doesNotMatch(section.textContent, /Вкажіть нову назву/);
    assert.deepEqual(ui.fixture.settings.clientRenames, [{ path: 'extra.cfg', to: 'extra.old', enabled: true }]);
    await ui.click('Продовжити →', ui.document.getElementById('client-step-2-content'));
    ui.fixture.clientRenamePreview = [{ path: 'extra.cfg', to: 'extra.old', found: true, replaces: true }];
    await ui.click('Підготувати й перевірити');
    assert.deepEqual(cmd(ui, 'prepare_installation')[0].args.request.renames, [{ path: 'extra.cfg', to: 'extra.old', enabled: true }]);
    const card = ui.document.querySelector('.installation-card').textContent;
    assert.match(card, /extra\.cfg → extra\.old/);
    assert.match(card, /Замінить наявний файл/);
  } finally { await ui.close(); }
});
