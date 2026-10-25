import test from 'node:test';
import assert from 'node:assert/strict';
import { chooseLocalPack, resolvePrismDirectory } from '../src/pack-dialog.ts';

test('ZIP and folder choices dispatch native dialog with correct filters and preserve OS path', async () => {
  for (const [kind, path] of [['zip', 'C:\\Packs\\GTNH.ZIP'], ['folder', '/home/me/GTNH New']]) {
    const result = await chooseLocalPack(kind, { open: async options => {
      assert.equal(options.directory, kind === 'folder');
      assert.equal(options.multiple, false);
      assert.equal(options.title, 'Choose');
      assert.equal(options.defaultPath, '/packs');
      assert.deepEqual(options.filters, kind === 'zip' ? [{ name: 'ZIP', extensions: ['zip'] }] : undefined);
      return path;
    } }, 'Choose', '/packs');
    assert.equal(result, path);
  }
});
test('cancel returns null; failures and unexpected multiple selections are not silently accepted', async () => {
  assert.equal(await chooseLocalPack('zip', { open: async () => null }, 'Choose'), null);
  await assert.rejects(chooseLocalPack('folder', { open: async () => { throw new Error('denied'); } }, 'Choose'), /denied/);
  await assert.rejects(chooseLocalPack('zip', { open: async () => ['/a.zip', '/b.zip'] }, 'Choose'), /single/i);
});
test('Prism preference is validated at point of use; missing preference has no filesystem side effects', async () => {
  assert.equal(await resolvePrismDirectory(null, async () => { throw new Error('must not run'); }), null);
  assert.equal(await resolvePrismDirectory('/saved', async path => { assert.equal(path, '/saved'); return '/canonical'; }), '/canonical');
  await assert.rejects(resolvePrismDirectory('/moved', async () => { throw new Error('missing'); }), /missing/);
});
