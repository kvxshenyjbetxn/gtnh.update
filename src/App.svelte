<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import { messages } from './i18n';
  import Catalog from './Catalog.svelte';
  import Installation from './Installation.svelte';
  import LocalPackPicker from './LocalPackPicker.svelte';
  import type { InstallRequest } from './installation';
  import type { Artifact } from './releases';
  import type { Settings, Rule } from './types';

  let settings = $state<Settings>({ schemaVersion: 1, language: 'uk', theme: 'amoled', includePrereleases: false, clientSource: null, serverSource: null, prismInstancesDirectory: null, clientRules: [], serverRules: [] });
  let tab = $state<'client' | 'server' | 'settings'>('client');
  let loaded = $state(false);
  let loading = $state(true);
  let saving = $state(false);
  let picking = $state(false);
  let error = $state('');
  let notice = $state('');
  let directory = $state('');
  let savedSnapshot = $state('');
  let installRequest = $state<InstallRequest | null>(null);
  let installBusy = $state(false);
  let packPickerOpen = $state(false);
  let packPickerMode = $state<'client' | 'server'>('client');
  const desktop = isTauri();
  let t = $derived(messages[settings.language]);
  let rules = $derived(tab === 'server' ? settings.serverRules : settings.clientRules);
  let source = $derived(tab === 'server' ? settings.serverSource : settings.clientSource);
  let dirty = $derived(loaded && JSON.stringify(settings) !== savedSnapshot);

  $effect(() => {
    document.documentElement.dataset.theme = settings.theme;
    document.documentElement.lang = settings.language;
  });

  async function load() {
    loading = true; loaded = false; error = ''; notice = '';
    try {
      if (desktop) {
        const result = await invoke<Settings>('load_settings');
        settings = result; savedSnapshot = JSON.stringify(result); loaded = true;
        try { directory = await invoke<string>('config_directory'); }
        catch (e) { error = String(e); }
      }
    } catch (e) { error = String(e); }
    finally { loading = false; }
  }
  async function save() {
    saving = true; error = ''; notice = '';
    try {
      settings = await invoke<Settings>('save_settings', { settings: $state.snapshot(settings) });
      savedSnapshot = JSON.stringify(settings); notice = t.saved;
    } catch (e) { error = String(e); }
    finally { saving = false; }
  }
  async function openDirectory() {
    try { await invoke('open_config_directory'); }
    catch (e) { error = String(e); }
  }
  async function pickPrismDirectory() {
    picking = true; error = ''; notice = '';
    try {
      const path = await open({ directory: true, multiple: false, title: t.prismInstances, defaultPath: settings.prismInstancesDirectory ?? undefined });
      if (path) settings.prismInstancesDirectory = await invoke<string>('instances_directory', { path });
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
  async function pickSource() {
    picking = true; error = ''; notice = '';
    const mode = tab;
    try {
      const path = await open({ directory: true, multiple: false, title: t.source, defaultPath: source ?? undefined });
      if (!path) return;
      const root = await invoke<string>('source_directory', { path, client: mode === 'client' });
      if (mode === 'server') settings.serverSource = root; else settings.clientSource = root;
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
  async function pickRules(folder: boolean) {
    if (!source) return;
    picking = true; error = ''; notice = '';
    const mode = tab; const root = source;
    try {
      const selected = await open({ directory: folder, multiple: true, title: folder ? t.addFolder : t.addFile, defaultPath: root });
      if (!selected) return;
      const paths = await invoke<string[]>('relative_selections', { root, paths: Array.isArray(selected) ? selected : [selected] });
      const target = mode === 'server' ? settings.serverRules : settings.clientRules;
      let added = false;
      for (const path of paths) {
        if (target.some(r => r.path.toLowerCase() === path.toLowerCase())) continue;
        target.push({ path, enabled: true, conflict: 'ask', custom: true }); added = true;
      }
      if (!added) notice = t.duplicate;
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
  function removeRule(rule: Rule) {
    if (tab === 'server') settings.serverRules = settings.serverRules.filter(r => r !== rule);
    else settings.clientRules = settings.clientRules.filter(r => r !== rule);
  }
  function selectInstallation(artifact: Artifact) {
    const selectedSource = artifact.mode === 'client' ? settings.clientSource : settings.serverSource;
    if (!selectedSource) { error = t.selectFirst; return; }
    installRequest = {
      url: artifact.url, client: artifact.mode === 'client', source: selectedSource,
      rules: $state.snapshot(artifact.mode === 'client' ? settings.clientRules : settings.serverRules),
    };
  }
  function chooseLocalPack() {
    if (tab === 'settings' || !source || installBusy) return;
    packPickerMode = tab; packPickerOpen = true;
  }
  function selectLocalPack(path: string) {
    const selectedSource = packPickerMode === 'client' ? settings.clientSource : settings.serverSource;
    if (!selectedSource) { error = t.selectFirst; return; }
    installRequest = {
      localPath: path, client: packPickerMode === 'client', source: selectedSource,
      rules: $state.snapshot(packPickerMode === 'client' ? settings.clientRules : settings.serverRules),
    };
    packPickerOpen = false;
  }
  onMount(() => { void load(); });
</script>

{#if packPickerOpen}<LocalPackPicker language={settings.language} onSelect={selectLocalPack} onClose={() => { packPickerOpen = false; }} />{/if}
<div class="shell">
  <aside>
    <div class="brand"><span class="brand-icon">G</span><strong>GTNH <span>Updater</span></strong></div>
    <p class="nav-label">{t.workspace}</p>
    <nav aria-label={t.workspace}>
      {#each ['client', 'server', 'settings'] as item}
        <button disabled={picking || saving || packPickerOpen} class:active={tab === item} aria-current={tab === item ? 'page' : undefined} onclick={() => { tab = item as typeof tab; notice = ''; }}>
          <span class="nav-symbol" aria-hidden="true">{item === 'client' ? '◈' : item === 'server' ? '▤' : '⚙'}</span>{t[item as 'client' | 'server' | 'settings']}
        </button>
      {/each}
    </nav>
    <div class="aside-note">v0.1.0</div>
  </aside>
  <main>
    <section class="hero"><h1>{t[tab]}</h1>{#if tab === 'settings'}<p>{t.global}</p>{/if}</section>
    {#if loading}<p role="status">{t.loading}</p>{/if}
    {#if !desktop}<div class="notice">{t.browser}</div>{/if}
    {#if error}<div class="error" role="alert"><strong>{loaded ? t.error : t.loadError}</strong><pre>{error}</pre>{#if !loaded}<button onclick={load}>{t.retry}</button>{/if}</div>{/if}

    {#if tab !== 'settings' && loaded}
      <section class="card"><div class="card-heading"><h2>{t.alreadyDownloaded}</h2><button class="secondary" disabled={!desktop || !source || installBusy || picking} onclick={chooseLocalPack}>{t.choosePack}</button></div><p class="muted">{t.localPackHint}</p>{#if !source}<p class="muted">{t.selectFirst}</p>{/if}</section>
      <Catalog language={settings.language} mode={tab} includePrereleases={settings.includePrereleases} {desktop}
        canInstall={!!source && !installBusy} onInstall={selectInstallation} />
    {/if}
    <Installation language={settings.language} {desktop} bind:request={installRequest} bind:busy={installBusy} />
    <fieldset disabled={!loaded || saving || picking}>
      {#if tab === 'settings'}
        <section class="card"><h2>{t.appearance}</h2><div class="settings-grid">
          <label>{t.language}<select bind:value={settings.language}><option value="uk">Українська</option><option value="en">English</option></select></label>
          <label>{t.theme}<select bind:value={settings.theme}><option value="amoled">{t.amoled}</option><option value="light">{t.light}</option></select></label>
        </div></section>
        <section class="card"><h2>{t.channel}</h2><label class="toggle-label"><input type="checkbox" bind:checked={settings.includePrereleases} /><strong>{t.beta}</strong></label><p class="muted">{t.betaHint}</p>
        </section>
        <section class="card"><h2>{t.prismInstances}</h2><p class="muted">{t.prismInstancesHint}</p><code class="directory">{settings.prismInstancesDirectory || '—'}</code><div class="picker-actions"><button class="secondary" disabled={!desktop || installBusy} onclick={pickPrismDirectory}>{t.selectSource}</button><button class="secondary" disabled={!settings.prismInstancesDirectory || installBusy} onclick={() => { settings.prismInstancesDirectory = null; }}>{t.clear}</button></div></section>
        <section class="card"><h2>{t.storage}</h2><p class="muted">{t.storageHint}</p><code class="directory">{directory || '—'}</code><button class="secondary" onclick={openDirectory}>{t.open} ↗</button></section>
      {:else}
        <section class="card"><h2>{t.source}</h2><p class="muted">{tab === 'server' ? t.serverSourceHint : t.sourceHint}</p>{#if source}<code class="directory">{source}</code>{/if}<button class="secondary" onclick={pickSource}>{t.selectSource}</button></section>
        <section class="card rules-card">
          <div class="card-heading"><h2>{t.rules}</h2><span class="count">{rules.filter(r => r.enabled).length} / {rules.length}</span></div>
          <p class="muted">{tab === 'server' ? t.serverHint : t.clientHint}</p>
          <div class="rule-list">
            {#each rules as rule (rule.path)}
              <div class="rule-row">
                <label class="rule-path"><input type="checkbox" bind:checked={rule.enabled} /><code>{rule.path}</code></label>
                <span class="tag">{rule.custom ? t.custom : t.standard}</span>
                {#if rule.custom}<select aria-label={`${t.conflict}: ${rule.path}`} bind:value={rule.conflict}><option value="ask">{t.ask}</option><option value="replace">{t.replace}</option><option value="keep">{t.keep}</option></select><button class="remove" aria-label={`${t.remove}: ${rule.path}`} onclick={() => removeRule(rule)}>×</button>
                {:else}<span class="policy">{t.replace}</span>{/if}
              </div>
            {/each}
          </div>
          <div class="add-form"><div class="picker-actions"><button class="secondary" disabled={!source} onclick={() => pickRules(false)}>+ {t.addFile}</button><button class="secondary" disabled={!source} onclick={() => pickRules(true)}>+ {t.addFolder}</button></div><p class="muted small">{source ? t.selectionHint : t.selectFirst}</p></div>
        </section>
      {/if}
    </fieldset>
    <footer><span aria-live="polite">{dirty ? t.unsaved : notice || (loaded ? t.clean : '')}</span><button class="primary" disabled={!loaded || saving || picking || !dirty} onclick={save}>{saving ? t.saving : t.save}</button></footer>
  </main>
</div>
