<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import { messages } from './i18n';
  import { formatBytes, groupTransfers, packName, unresolvedConflicts, watchInstallation, type InstallRequest, type InstallStatus, type Transfer } from './installation';
  import type { Rule } from './types';
  import { transactionBusy } from './update-flow';

  let { language, desktop, request = $bindable(null), busy = $bindable(false), defaultParent, onStatus, onSettings }: {
    language: 'uk' | 'en'; desktop: boolean; request: InstallRequest | null; busy: boolean;
    defaultParent: string | null; onStatus: (status: InstallStatus | null) => void; onSettings: () => void;
  } = $props();
  let t = $derived(messages[language]);
  let status = $state<InstallStatus | null>(null);
  let parent = $state('');
  let name = $state('');
  let error = $state('');
  let sending = $state(false);
  let decisions = $state<Record<string, Rule['conflict']>>({});
  let active = $derived(!!status && ['preparing', 'preview', 'copying'].includes(status.phase));
  let working = $derived(status?.phase === 'preparing' || status?.phase === 'copying');
  let shown = $derived(!!request || !!status?.phase);
  let preview = $derived(status?.phase === 'preview' ? status.preview : null);
  let entries = $derived(preview?.files ?? []);
  let files = $derived(entries.filter(file => !file.directory));
  let folders = $derived(entries.filter(file => file.directory));
  let asks = $derived(files.filter(file => file.exists && file.conflict === 'ask'));
  let groups = $derived(groupTransfers(entries));
  let listOpen = $state(false);
  let limit = $state(200);
  let unanswered = $derived(preview ? unresolvedConflicts(preview, decisions).length : 0);

  $effect(() => { busy = transactionBusy(status?.phase ?? '', sending); });
  $effect(() => { onStatus(status); });
  $effect(() => {
    const next = request;
    if (next) untrack(() => {
      name = packName(next.localPath ?? next.url);
      parent = defaultParent ?? ''; error = ''; decisions = {}; listOpen = false; limit = 200;
    });
  });
  $effect(() => {
    const savedParent = defaultParent;
    if (request?.client) parent = savedParent ?? '';
  });
  const policy = (file: Transfer) => file.directory ? t.copyFolder : file.exists ? (file.conflict === 'keep' ? t.keep : t.replace) : t.copyFile;
  async function pickDestination() {
    if (!desktop || !request || request.client || sending) return;
    try {
      const selected = await open({ directory: true, multiple: false, title: t.serverDestination, defaultPath: parent || undefined });
      if (selected && !Array.isArray(selected)) parent = selected;
    } catch (e) { error = String(e); }
  }
  async function prepare() {
    if (!request || !parent || !name || sending) return;
    sending = true; error = ''; decisions = {};
    try {
      await invoke('prepare_installation', { request: { ...$state.snapshot(request), parent, name } });
      status = await invoke<InstallStatus | null>('installation_status') ?? status;
      request = null;
    } catch (e) { error = String(e); }
    finally { sending = false; }
  }
  async function finish() {
    sending = true; error = '';
    try {
      await invoke('finish_installation', { decisions: Object.entries(decisions).map(([path, conflict]) => ({ path, conflict })) });
      status = await invoke<InstallStatus | null>('installation_status') ?? status;
    } catch (e) { error = String(e); }
    finally { sending = false; }
  }
  async function close() {
    sending = true; error = '';
    try {
      await invoke('cancel_installation');
      status = await invoke<InstallStatus | null>('installation_status') ?? status;
      request = null;
    } catch (e) { error = String(e); }
    finally { sending = false; }
  }
  onMount(() => {
    if (!desktop) return;
    return watchInstallation({
      status: since => invoke<InstallStatus | null>('installation_status', { since }),
      repeat: callback => { const timer = setInterval(callback, 500); return () => clearInterval(timer); },
    }, next => {
      if (!status || next.revision > status.revision) status = next;
    }, e => { error = String(e); });
  });
</script>

{#snippet row(file: Transfer)}
  <div class="rule-row"><code class="rule-path">{file.path}</code>
    {#if !file.directory && file.exists && file.conflict === 'ask'}
      <select aria-label={`${t.conflict}: ${file.path}`} value={decisions[file.path] ?? 'ask'} onchange={e => { decisions[file.path] = e.currentTarget.value as Rule['conflict']; }}><option value="ask">{t.ask}</option><option value="replace">{t.replace}</option><option value="keep">{t.keep}</option></select>
    {:else}<span class="policy">{policy(file)}</span>{/if}
  </div>
{/snippet}

{#if shown}
  <section class="card installation-card" aria-label={t.install}>
    <div class="card-heading"><h2>{t.install}</h2><button class="secondary" disabled={working || sending} onclick={close}>{active ? t.cancelDownload : t.close}</button></div>
    {#if error}<div class="error" role="alert">{error}</div>{/if}
    {#if request && !active}
      {#if !request.client}
        <button class="secondary" disabled={sending} onclick={pickDestination}>{t.selectDestination}</button>
        {#if parent}<code class="directory">{parent}</code>{/if}
      {:else if !parent}<button class="secondary" disabled={sending} onclick={onSettings}>{t.configurePrism}</button>{/if}
      <label class="install-name">{request.client ? t.newInstanceName : t.newFolderName}<input bind:value={name} disabled={sending} /></label>
      <button class="primary" disabled={!parent || !name || sending} onclick={prepare}>{t.prepareInstallation}</button>
    {/if}
    {#if working}
      <p role="status">{status?.progress ? t[status.progress.stage] : t.prepareInstallation}…</p>
      <progress aria-label={t.installProgress} max={status?.progress?.total || undefined} value={status?.progress?.total ? status.progress.done : undefined}></progress>
      {#if status?.progress}<p class="muted">{status.progress.done} / {status.progress.total}</p>{/if}
    {/if}
    {#if preview}
      <h3>{t.transferPreview}</h3><code class="directory">{preview.destination}</code>
      <p>{t.transferCount}: {files.length} · {t.transferFolders}: {folders.length}</p>
      <p class="notice">{t.installWarning}</p>
      {#if !entries.length}<p class="notice" role="status">{t.nothingToTransfer}</p>{/if}
      {#if preview.missing.length}<details open={!entries.length}><summary>{t.skippedMissing} ({preview.missing.length})</summary>{#each preview.missing as path}<code class="directory">{path}</code>{/each}</details>{/if}
      {#if asks.length}
        <h4>{t.needsDecision}</h4>
        <div class="transfer-list">{#each asks as file (file.path)}{@render row(file)}{/each}</div>
      {/if}
      {#if entries.length}
        <details open><summary>{t.transferTopLevel} ({groups.length})</summary>
          <div class="transfer-list">{#each groups as group (group.path)}
            <div class="rule-row"><code class="rule-path">{group.path}</code>
              {#if group.directory}<span class="policy">{t.copyFolder} · {group.files} {t.filesSummary} · {formatBytes(group.bytes, language)}</span>
              {:else}<span class="policy">{policy(group.file)}</span>{/if}
            </div>
          {/each}</div>
        </details>
        <details ontoggle={e => { listOpen = e.currentTarget.open; if (!listOpen) limit = 200; }}><summary>{t.allTransferFiles} ({entries.length})</summary>
          {#if listOpen}
            <div class="transfer-list">{#each entries.slice(0, limit) as file (file.path)}{@render row(file)}{/each}</div>
            {#if limit < entries.length}<button class="secondary" onclick={() => { limit += 500; }}>{t.showMore}</button>{/if}
          {/if}
        </details>
      {/if}
      {#if unanswered}<p class="muted">{t.resolveConflicts}: {unanswered}</p>{/if}
      <button class="primary" disabled={sending || unanswered > 0} onclick={finish}>{t.confirmInstallation}</button>
    {/if}
    {#if !request && status?.phase === 'completed'}
      <p role="status">{t.installComplete}</p><code class="directory">{status.destination}</code>
      <p class="muted">{status.client ? t.prismReady : t.serverReady}</p>
    {:else if !request && status?.phase === 'failed'}
      <div class="error" role="alert">{t.installFailed}<pre>{status.error}</pre></div>
    {/if}
  </section>
{/if}
