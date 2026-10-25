import { build } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { JSDOM } from 'jsdom';
import { fileURLToPath } from 'node:url';

let bundle;
async function compiledApp() {
  if (!bundle) bundle = (async () => {
    const result = await build({
      configFile: false, logLevel: 'silent',
      plugins: [{
        name: 'ui-test-boundaries', enforce: 'pre',
        resolveId(id) {
          if (id === 'virtual:ui-entry') return '\0ui-entry';
          if (id.startsWith('@tauri-apps/')) return '\0tauri-boundary';
        },
        load(id) {
          if (id === '\0ui-entry') return `import { mount, unmount } from 'svelte'; import App from ${JSON.stringify(fileURLToPath(new URL('../../src/App.svelte', import.meta.url)))}; const app = mount(App, {target: document.getElementById('root')}); window.stopApp = () => unmount(app);`;
          if (id === '\0tauri-boundary') return `export const isTauri = () => true; export const invoke = async (command,args) => { const value = await window.fixture.invoke(command,args); return value === undefined ? undefined : JSON.parse(JSON.stringify(value)); }; export const open = options => window.fixture.open(options); export const listen = async (name, callback) => { window.fixture.listeners.add(callback); return () => window.fixture.listeners.delete(callback); };`;
        },
      }, svelte()],
      build: { rollupOptions: { input: 'virtual:ui-entry' }, write: false, minify: false, target: 'es2022', lib: { entry: 'virtual:ui-entry', name: 'TestUI', formats: ['iife'] } },
    });
    const output = Array.isArray(result) ? result[0].output : result.output;
    return output.find(item => item.type === 'chunk').code;
  })();
  return bundle;
}
export const settings = () => ({ schemaVersion: 1, language: 'uk', theme: 'amoled', includePrereleases: false, clientSource: '/old', serverSource: '/server', prismInstancesDirectory: '/instances', clientRules: [{ path: 'saves', enabled: true, conflict: 'replace', custom: false }], serverRules: [{ path: 'config/JourneyMapServer', enabled: true, conflict: 'replace', custom: false }] });
export function idle(revision = 0) { return { revision, phase: '', url: '', client: true, progress: null, preview: null, destination: null, error: null, warning: null }; }
export async function app(options = {}) {
  const fixture = {
    settings: settings(), status: idle(), commands: [], dialogs: [], listeners: new Set(), releases: [], downloads: [],
    async open(opts) { this.commands.push({ command: 'open', args: opts }); return this.dialogs.shift() ?? null; },
    async invoke(command, args) {
      this.commands.push({ command, args: structuredClone(args) });
      switch (command) {
        case 'load_settings': return structuredClone(this.settings);
        case 'save_settings': this.settings = structuredClone(args.settings); return structuredClone(this.settings);
        case 'config_directory': return '/config';
        case 'source_directory': return args.path;
        case 'instances_directory': if (args.path === '/moved') throw new Error('missing directory'); return args.path;
        case 'relative_selections': return ['extra.cfg'];
        case 'list_releases': return this.releases;
        case 'download_status': return this.downloads;
        case 'installation_status': return structuredClone(this.status);
        case 'prepare_installation': this.status = { ...idle(this.status.revision + 1), phase: 'preview', client: args.request.client, preview: { destination: '/instances/new', files: [], missing: [] } }; return;
        case 'cancel_installation': this.status = idle(this.status.revision + 1); return;
        case 'finish_installation': this.status = { ...idle(this.status.revision + 1), phase: 'completed', destination: '/instances/new' }; return;
        default: throw new Error(`Unexpected command: ${command}`);
      }
    }, ...options,
  };
  const dom = new JSDOM('<!DOCTYPE html><html><body><div id="root"></div></body></html>', { runScripts: 'outside-only', pretendToBeVisual: true, url: 'http://localhost' });
  dom.window.fixture = fixture;
  dom.window.structuredClone = structuredClone;
  dom.window.eval(await compiledApp());
  const settle = () => new Promise(resolve => setTimeout(resolve, 25));
  await settle();
  const document = dom.window.document;
  const visible = element => !element.closest('[hidden]');
  function button(text, scope = document) {
    const matches = [...scope.querySelectorAll('button')].filter(el => visible(el) && (el.textContent.trim() === text || el.querySelector('strong')?.textContent.trim() === text));
    if (matches.length !== 1) throw new Error(`Expected one visible '${text}', got ${matches.length}`);
    return matches[0];
  }
  async function click(text, scope) { const el = button(text, scope); if (el.disabled) throw new Error(`Disabled button: ${text}`); el.click(); await settle(); }
  async function close() { await dom.window.stopApp(); dom.window.close(); }
  return { dom, document, fixture, settle, visible, button, click, close };
}
