<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import { messages } from './i18n';
  import Catalog from './Catalog.svelte';
  import Installation from './Installation.svelte';
  import UpdateSection from './UpdateSection.svelte';
  import TransferRules from './TransferRules.svelte';
  import { packName, type InstallRequest, type InstallStatus } from './installation';
  import { chooseLocalPack, resolvePrismDirectory } from './pack-dialog';
  import { buildInstallRequest, createDraft, transitionDraft, type DraftAction, type UpdateStep } from './update-flow';
  import type { Artifact } from './releases';
  import type { Settings } from './types';

  type Mode = 'client' | 'server';
  const modes: Mode[] = ['client', 'server'];
  const desktop = isTauri();
  let settings = $state<Settings>({ schemaVersion: 1, language: 'uk', theme: 'amoled', includePrereleases: false, clientSource: null, serverSource: null, prismInstancesDirectory: null, clientRules: [], serverRules: [] });
  let drafts = $state({ client: createDraft(null), server: createDraft(null) });
  let tab = $state<Mode | 'settings'>('client');
  let lastMode = $state<Mode>('client');
  let loaded = $state(false);
  let loading = $state(true);
  let saving = $state(false);
  let picking = $state(false);
  let error = $state('');
  let notice = $state('');
  let directory = $state('');
  let savedSnapshot = $state('');
  let savedPrism = $state<string | null>(null);
  let installRequest = $state<InstallRequest | null>(null);
  let defaultParent = $state<string | null>(null);
  let installBusy = $state(false);
  let installStatus = $state<InstallStatus | null>(null);
  let packMethod = $state<Record<Mode, 'download' | 'local' | null>>({ client: null, server: null });
  let downloadSummary = $state<Record<Mode, string>>({ client: '', server: '' });
  let t = $derived(messages[settings.language]);
  let dirty = $derived(loaded && JSON.stringify(settings) !== savedSnapshot);
  let locked = $derived(!loaded || installBusy || picking || saving);
  let currentMode = $derived(tab === 'settings' ? lastMode : tab);

  $effect(() => {
    document.documentElement.dataset.theme = settings.theme;
    document.documentElement.lang = settings.language;
  });
  async function focusStep(mode: Mode, step: UpdateStep) {
    await tick();
    document.getElementById(`${mode}-step-${step}-heading`)?.focus();
  }
  function act(mode: Mode, action: DraftAction) {
    drafts[mode] = transitionDraft(drafts[mode], action, locked);
  }
  function openStep(mode: Mode, step: UpdateStep) {
    if (locked) return;
    act(mode, { kind: 'open', step });
    void focusStep(mode, drafts[mode].step);
  }
  async function load() {
    loading = true; loaded = false; error = ''; notice = '';
    try {
      if (desktop) {
        settings = await invoke<Settings>('load_settings');
        savedSnapshot = JSON.stringify(settings); savedPrism = settings.prismInstancesDirectory;
        drafts = { client: createDraft(settings.clientSource), server: createDraft(settings.serverSource) };
        loaded = true;
        try { directory = await invoke<string>('config_directory'); } catch (e) { error = String(e); }
      }
    } catch (e) { error = String(e); }
    finally { loading = false; }
  }
  async function save() {
    if (locked) return;
    saving = true; error = ''; notice = '';
    try {
      settings = await invoke<Settings>('save_settings', { settings: $state.snapshot(settings) });
      savedSnapshot = JSON.stringify(settings); savedPrism = settings.prismInstancesDirectory; notice = t.saved;
    } catch (e) { error = String(e); }
    finally { saving = false; }
  }
  async function openDirectory() {
    try { await invoke('open_config_directory'); } catch (e) { error = String(e); }
  }
  async function prismParent() {
    return resolvePrismDirectory(savedPrism, path => invoke<string>('instances_directory', { path }));
  }
  async function pickPrismDirectory() {
    if (locked || !desktop) return;
    picking = true; error = ''; notice = '';
    try {
      let initial: string | undefined;
      if (settings.prismInstancesDirectory) {
        try { initial = await invoke<string>('instances_directory', { path: settings.prismInstancesDirectory }); }
        catch (e) { error = `${t.prismUnavailable}\n${String(e)}`; }
      }
      const path = await open({ directory: true, multiple: false, title: t.prismInstances, defaultPath: initial });
      if (path && !Array.isArray(path)) settings.prismInstancesDirectory = await invoke<string>('instances_directory', { path });
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
  async function pickSource(mode: Mode) {
    if (locked || !desktop) return;
    picking = true; error = ''; notice = '';
    try {
      let initial = drafts[mode].source ?? undefined;
      if (mode === 'client' && savedPrism) {
        try { initial = await prismParent() ?? undefined; }
        catch (e) { initial = undefined; error = `${t.prismUnavailable}\n${String(e)}`; }
      }
      const path = await open({ directory: true, multiple: false, title: t.oldBuild, defaultPath: initial });
      if (!path || Array.isArray(path)) return;
      const root = await invoke<string>('source_directory', { path, client: mode === 'client' });
      drafts[mode] = transitionDraft(drafts[mode], { kind: 'source', source: root }, false);
      if (mode === 'client') settings.clientSource = root; else settings.serverSource = root;
      installRequest = null;
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
  async function pickLocal(mode: Mode, kind: 'zip' | 'folder') {
    if (locked || !desktop || drafts[mode].completed < 1) return;
    picking = true; error = '';
    try {
      const path = await chooseLocalPack(kind, { open }, kind === 'zip' ? t.chooseZip : t.chooseFolder);
      if (path) {
        drafts[mode] = transitionDraft(drafts[mode], { kind: 'pack', pack: { localPath: path } }, false);
        installRequest = null;
      }
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
  function selectDownload(artifact: Artifact) {
    if (locked) return;
    act(artifact.mode, { kind: 'pack', pack: { url: artifact.url } });
    installRequest = null;
  }
  async function continueDraft(mode: Mode) {
    if (locked) return;
    picking = true; error = '';
    try {
      if (drafts[mode].step === 0 && drafts[mode].source) {
        const root = await invoke<string>('source_directory', { path: drafts[mode].source, client: mode === 'client' });
        drafts[mode] = transitionDraft(drafts[mode], { kind: 'source', source: root }, false);
        if (mode === 'client') settings.clientSource = root; else settings.serverSource = root;
      }
      if (drafts[mode].step === 2) {
        defaultParent = null;
        if (mode === 'client') {
          try { defaultParent = await prismParent(); }
          catch (e) { error = `${t.prismUnavailable}\n${String(e)}`; }
        }
      }
      drafts[mode] = transitionDraft(drafts[mode], { kind: 'continue' }, false);
      if (drafts[mode].step === 3) setRequest(mode);
      await focusStep(mode, drafts[mode].step);
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
  async function continueFromReview() {
    if (locked) return;
    drafts[currentMode] = { ...drafts[currentMode], step: 2, completed: 2 };
    await continueDraft(currentMode);
  }
  function setRequest(mode: Mode) {
    installRequest = buildInstallRequest(drafts[mode], mode === 'client', mode === 'client' ? settings.clientRules : settings.serverRules);
  }
  function rulesChanged(mode: Mode) {
    if (installBusy) return;
    drafts[mode] = { ...drafts[mode], completed: Math.min(drafts[mode].completed, 2), step: 2 };
    installRequest = null;
  }
  function switchTab(next: typeof tab) {
    if (picking || saving || (installBusy && next !== 'settings' && next !== lastMode)) return;
    if (next !== 'settings' && next !== lastMode) {
      installRequest = null; defaultParent = null;
      // An already completed draft returns to its transfer step to rebuild its request/destination.
      if (drafts[next].completed === 3) drafts[next] = { ...drafts[next], step: 2, completed: 2 };
    }
    tab = next; if (next !== 'settings') lastMode = next;
    notice = '';
  }
  function packSummary(mode: Mode) {
    const pack = drafts[mode].pack;
    const selected = pack ? packName(pack.localPath ?? pack.url) : t.packNotSelected;
    return [selected, downloadSummary[mode]].filter(Boolean).join(' · ');
  }
  onMount(() => { void load(); });
</script>

<div class="shell">
  <aside>
    <div class="brand"><span class="brand-icon">G</span><strong>GTNH <span>Updater</span></strong></div>
    <p class="nav-label">{t.workspace}</p>
    <nav aria-label={t.workspace}>
      {#each ['client', 'server', 'settings'] as item}
        <button disabled={picking || saving || (installBusy && item !== 'settings' && item !== lastMode)} class:active={tab === item} aria-current={tab === item ? 'page' : undefined} onclick={() => switchTab(item as typeof tab)}>
          <span class="nav-symbol" aria-hidden="true">{item === 'client' ? '◈' : item === 'server' ? '▤' : '⚙'}</span>{t[item as 'client' | 'server' | 'settings']}
        </button>
      {/each}
    </nav>
    <div class="aside-note">v0.1.0</div>
  </aside>
  <main>
    <section class="hero"><p class="eyebrow">{tab === 'settings' ? t.settings : `${t.updateBuild} · ${t[tab]}`}</p><h1>{tab === 'settings' ? t.settings : t.updateHeading}</h1><p>{tab === 'settings' ? t.global : t.updateHint}</p></section>
    {#if loading}<p role="status">{t.loading}</p>{/if}
    {#if !desktop}<div class="notice">{t.browser}</div>{/if}
    {#if error}<div class="error" role="alert"><strong>{loaded ? t.error : t.loadError}</strong><pre>{error}</pre>{#if !loaded}<button onclick={load}>{t.retry}</button>{/if}</div>{/if}
    {#if installBusy && tab === 'settings'}<div class="notice" role="status">{t.installInProgress} <button class="secondary" onclick={() => switchTab(lastMode)}>{t.backToUpdate}</button></div>{/if}

    {#if loaded}
      {#each modes as mode}
        <div hidden={tab !== mode} class="update-flow">
          <UpdateSection id={`${mode}-step-0`} number={1} title={t.oldBuild} summary={drafts[mode].source || t.selectFirst} expanded={drafts[mode].step === 0} locked={locked} onToggle={() => openStep(mode, 0)}>
            <p class="muted">{mode === 'client' ? t.sourceHint : t.serverSourceHint}</p>
            {#if drafts[mode].source}<code class="directory">{drafts[mode].source}</code>{/if}
            <button class="secondary" disabled={locked || !desktop} onclick={() => pickSource(mode)}>{t.selectSource}…</button>
            <div class="step-actions"><small>{t.sourceUntouched}</small><button class="primary" disabled={locked || !drafts[mode].source} onclick={() => continueDraft(mode)}>{t.continue} →</button></div>
          </UpdateSection>
          <UpdateSection id={`${mode}-step-1`} number={2} title={t.newBuild} summary={packSummary(mode)} expanded={drafts[mode].step === 1} locked={locked || drafts[mode].completed < 1} lockedReason={drafts[mode].completed < 1 ? t.completePrevious : ''} onToggle={() => openStep(mode, 1)}>
            <p class="muted">{t.newBuildHint}</p>
            <div class="pack-methods">
              <button class="pack-method" class:selected={packMethod[mode] === 'download'} disabled={locked} aria-pressed={packMethod[mode] === 'download'} onclick={() => { packMethod[mode] = 'download'; }}><strong>↓ {t.download}</strong><span>{t.downloadChoiceHint}</span></button>
              <button class="pack-method" class:selected={packMethod[mode] === 'local'} disabled={locked} aria-pressed={packMethod[mode] === 'local'} onclick={() => { packMethod[mode] = 'local'; }}><strong>▤ {t.alreadyDownloaded}</strong><span>{t.localChoiceHint}</span></button>
            </div>
            <div hidden={packMethod[mode] !== 'download'}>
              <Catalog language={settings.language} {mode} includePrereleases={settings.includePrereleases} {desktop} canInstall={!locked && drafts[mode].completed >= 1} onInstall={selectDownload} onSummary={summary => { downloadSummary[mode] = summary; }} />
            </div>
            <div hidden={packMethod[mode] !== 'local'} class="local-choice"><div class="picker-actions"><button class="secondary" disabled={locked || !desktop} onclick={() => pickLocal(mode, 'zip')}>{t.chooseZip}…</button><button class="secondary" disabled={locked || !desktop} onclick={() => pickLocal(mode, 'folder')}>{t.chooseFolder}…</button></div><p class="muted">{t.localPackHint}</p></div>
            {#if drafts[mode].pack}<code class="directory">{drafts[mode].pack?.localPath ?? drafts[mode].pack?.url}</code>{/if}
            <div class="step-actions"><small>{t.nativeDialogHint}</small><button class="primary" disabled={locked || !drafts[mode].pack} onclick={() => continueDraft(mode)}>{t.continue} →</button></div>
          </UpdateSection>
          <UpdateSection id={`${mode}-step-2`} number={3} title={t.whatToTransfer} summary={`${t.enabledRules}: ${(mode === 'client' ? settings.clientRules : settings.serverRules).filter(rule => rule.enabled).length}`} expanded={drafts[mode].step === 2} locked={locked || drafts[mode].completed < 2} lockedReason={drafts[mode].completed < 2 ? t.completePrevious : ''} onToggle={() => openStep(mode, 2)}>
            {#if mode === 'client'}<TransferRules bind:rules={settings.clientRules} source={drafts[mode].source || ''} language={settings.language} disabled={locked} client={true} {desktop} onChange={() => rulesChanged(mode)} bind:picking />
            {:else}<TransferRules bind:rules={settings.serverRules} source={drafts[mode].source || ''} language={settings.language} disabled={locked} client={false} {desktop} onChange={() => rulesChanged(mode)} bind:picking />{/if}
            <div class="step-actions"><small>{t.transferHint}</small><button class="primary" disabled={locked || !drafts[mode].pack} onclick={() => continueDraft(mode)}>{t.continue} →</button></div>
          </UpdateSection>
        </div>
      {/each}
    {/if}
    <div hidden={tab === 'settings' || !loaded}>
      <UpdateSection id={`${currentMode}-step-3`} number={4} title={t.reviewInstall} summary={installBusy ? t.installInProgress : t.reviewHint} expanded={drafts[currentMode].step === 3} locked={locked || drafts[currentMode].completed < 3} lockedReason={drafts[currentMode].completed < 3 ? t.completePrevious : ''} onToggle={() => openStep(currentMode, 3)}>
        <dl class="review-summary"><div><dt>{t.installSource}</dt><dd>{drafts[currentMode].source || '—'}</dd></div><div><dt>{t.newBuild}</dt><dd>{packSummary(currentMode)}</dd></div><div><dt>{t.enabledRules}</dt><dd>{(currentMode === 'client' ? settings.clientRules : settings.serverRules).filter(rule => rule.enabled).length}</dd></div></dl>
        <Installation language={settings.language} {desktop} bind:request={installRequest} bind:busy={installBusy} {defaultParent} onStatus={status => { installStatus = status; }} />
        {#if !installRequest && !installBusy && drafts[currentMode].completed === 3 && installStatus?.phase !== 'completed'}<button class="primary" disabled={locked} onclick={() => { void continueFromReview(); }}>{t.retryPreparation}</button>{/if}
      </UpdateSection>
    </div>
    <div hidden={tab !== 'settings'}>
      <fieldset disabled={!loaded || saving || picking}>
        <section class="card"><h2>{t.appearance}</h2><div class="settings-grid"><label>{t.language}<select bind:value={settings.language}><option value="uk">Українська</option><option value="en">English</option></select></label><label>{t.theme}<select bind:value={settings.theme}><option value="amoled">{t.amoled}</option><option value="light">{t.light}</option></select></label></div></section>
        <section class="card"><h2>{t.channel}</h2><label class="toggle-label"><input type="checkbox" bind:checked={settings.includePrereleases} /><strong>{t.beta}</strong></label><p class="muted">{t.betaHint}</p></section>
        <section class="card"><h2>{t.prismInstances}</h2><p class="muted">{t.prismInstancesHint}</p><code class="directory">{settings.prismInstancesDirectory || '—'}</code><div class="picker-actions"><button class="secondary" disabled={!desktop || installBusy} onclick={pickPrismDirectory}>{t.selectSource}…</button><button class="secondary" disabled={!settings.prismInstancesDirectory || installBusy} onclick={() => { settings.prismInstancesDirectory = null; }}>{t.clear}</button></div></section>
        <section class="card"><h2>{t.storage}</h2><p class="muted">{t.storageHint}</p><code class="directory">{directory || '—'}</code><button class="secondary" disabled={!desktop} onclick={openDirectory}>{t.open} ↗</button></section>
      </fieldset>
    </div>
    <footer><span aria-live="polite">{dirty ? t.unsaved : notice || (loaded ? t.clean : '')}</span><button class="primary" disabled={locked || !dirty} onclick={save}>{saving ? t.saving : t.save}</button></footer>
  </main>
</div>
