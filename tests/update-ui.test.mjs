import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile, unlink } from 'node:fs/promises';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { JSDOM } from 'jsdom';

async function section() {
  const source = await readFile(new URL('../src/UpdateSection.svelte', import.meta.url), 'utf8');
  const { js } = compile(source, { generate: 'server', filename: 'UpdateSection.svelte' });
  const output = new URL(`../src/.section-test-${process.pid}.mjs`, import.meta.url);
  await writeFile(output, js.code);
  try { return (await import(output.href)).default; }
  finally { await unlink(output); }
}
test('focused accordion heading keeps its outline inside the header and leaves space before content', async () => {
  const Component = await section();
  const css = await readFile(new URL('../src/style.css', import.meta.url), 'utf8');
  const body = render(Component, { props: {
    id: 'pack', number: 2, title: 'New build', summary: 'Choose build',
    expanded: true, locked: false, onToggle() {},
  } }).body;
  const dom = new JSDOM(`<style>${css}</style>${body}`);
  try {
    const heading = dom.window.document.getElementById('pack-heading');
    heading.focus();
    assert.equal(dom.window.document.activeElement, heading);
    const headingStyle = dom.window.getComputedStyle(heading);
    // A negative offset must contain the entire 2px focus ring in the header.
    assert.ok(parseFloat(headingStyle.outlineOffset) <= -2, 'focus ring must not extend into the content');
    const contentStyle = dom.window.getComputedStyle(dom.window.document.getElementById('pack-content'));
    assert.ok(parseFloat(contentStyle.paddingTop) >= 12, 'content needs a visible gap below the heading');
  } finally {
    dom.window.close();
  }
});

test('accordion renders accessible expanded/collapsed and locked sections rather than hiding the sequence', async () => {
  const Component = await section();
  const props = { id: 'source', number: 1, title: 'Old build', summary: '/old', expanded: false, locked: false, onToggle() {} };
  const collapsed = render(Component, { props }).body;
  assert.match(collapsed, /Old build/);
  assert.match(collapsed, /\/old/);
  assert.match(collapsed, /aria-expanded="false"/);
  assert.match(collapsed, /aria-controls="source-content"/);
  assert.match(collapsed, /hidden/);
  const expanded = render(Component, { props: { ...props, expanded: true } }).body;
  assert.match(expanded, /aria-expanded="true"/);
  assert.doesNotMatch(expanded, / hidden/);
  const locked = render(Component, { props: { ...props, locked: true, lockedReason: 'Complete previous section' } }).body;
  assert.match(locked, /disabled/);
  assert.match(locked, /Complete previous section/);
});
