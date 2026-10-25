<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import { messages } from './i18n';
  import { packName, unresolvedConflicts, watchInstallation, type InstallRequest, type InstallStatus } from './installation';
  import type { Rule } from './types';
  import { transactionBusy } from './update-flow';

  let { language, desktop, request = $bindable(null), busy = $bindable(false), defaultParent, onStatus }: {
    language: 'uk' | 'en'; desktop: boolean; request: InstallRequest | null; busy: boolean;
    defaultParent: string | null; onStatus: (status: InstallStatus | null) => void;
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
  let files = $derived(preview?.files.filter(file => !file.directory) ?? []);
  let unanswered = $derived(preview ? unresolvedConflicts(preview, decisions).length : 0);

  $effect(() => { busy = transactionBusy(status?.phase ?? '', sending); });
  $effect(() => { onStatus(status); });
  $effect(() => {
    const next = request;
    if (next) untrack(() => {
      name = packName(next.localPath ?? next.url);
      parent = defaultParent ?? ''; error = ''; decisions = {};
    });
  });
  async function pickDestination() {
    try {
      const selected = await open({ directory: true, multiple: false, title: request?.client ? t.prismDestination : t.serverDestination, defaultPath: parent || undefined });
      if (selected && !Array.isArray(selected)) parent = selected;
    } catch (e) { error = String(e); }
  }
  async function prepare() {
    if (!request || !parent || !name || sending) return;
    sending = true; error = ''; decisions = {};
    try {
      await invoke('prepare_installation', { request: { ...$state.snapshot(request), parent, name } });
      status = await invoke<InstallStatus>('installation_status');
      request = null;
    } catch (e) { error = String(e); }
    finally { sending = false; }
  }
  async function finish() {
    sending = true; error = '';
    try {
      await invoke('finish_installation', { decisions: Object.entries(decisions).map(([path, conflict]) => ({ path, conflict })) });
      status = await invoke<InstallStatus>('installation_status');
    } catch (e) { error = String(e); }
    finally { sending = false; }
  }
  async function close() {
    sending = true; error = '';
    try {
      await invoke('cancel_installation');
      status = await invoke<InstallStatus>('installation_status');
      request = null;
    } catch (e) { error = String(e); }
    finally { sending = false; }
  }
  onMount(() => {
    if (!desktop) return;
    return watchInstallation({
      status: () => invoke<InstallStatus>('installation_status'),
      repeat: callback => { const timer = setInterval(callback, 500); return () => clearInterval(timer); },
    }, next => {
      if (!status || next.revision > status.revision) status = next;
    }, e => { error = String(e); });
  });
</script>

{#if shown}
  <section class="card installation-card" aria-label={t.install}>
    <div class="card-heading"><h2>{t.install}</h2><button class="secondary" disabled={working || sending} onclick={close}>{active ? t.cancelDownload : t.close}</button></div>
    {#if error}<div class="error" role="alert">{error}</div>{/if}
    {#if request && !active}
      <code class="directory">{request.localPath ?? request.url}</code>
      <p class="muted">{request.client ? t.prismDestination : t.serverDestination}</p>
      <button class="secondary" disabled={sending} onclick={pickDestination}>{t.selectDestination}</button>
      {#if parent}<code class="directory">{parent}</code>{/if}
      <label class="install-name">{t.newFolderName}<input bind:value={name} disabled={sending} /></label>
      <p class="muted">{t.installSource}: <code>{request.source}</code></p>
      <p class="muted">{t.installRulesHint}</p>
      <button class="primary" disabled={!parent || !name || sending} onclick={prepare}>{t.prepareInstallation}</button>
    {/if}
    {#if working}
      <p role="status">{status?.progress ? t[status.progress.stage] : t.prepareInstallation}…</p>
      <progress aria-label={t.installProgress} max={status?.progress?.total || undefined} value={status?.progress?.total ? status.progress.done : undefined}></progress>
      {#if status?.progress}<p class="muted">{status.progress.done} / {status.progress.total}</p>{/if}
    {/if}
    {#if preview}
      <h3>{t.transferPreview}</h3><code class="directory">{preview.destination}</code>
      <p>{t.transferCount}: {files.length}</p>
      <p class="notice">{t.installWarning}</p>
      {#if preview.missing.length}<details><summary>{t.skippedMissing} ({preview.missing.length})</summary>{#each preview.missing as path}<code class="directory">{path}</code>{/each}</details>{/if}
      <details open={files.some(file => file.exists && file.conflict === 'ask')}><summary>{t.transferFiles}</summary>
        <div class="transfer-list">{#each files as file (file.path)}
          <div class="rule-row"><code class="rule-path">{file.path}</code>
            {#if file.exists && file.conflict === 'ask'}
              <select aria-label={`${t.conflict}: ${file.path}`} value={decisions[file.path] ?? 'ask'} onchange={e => { decisions[file.path] = e.currentTarget.value as Rule['conflict']; }}><option value="ask">{t.ask}</option><option value="replace">{t.replace}</option><option value="keep">{t.keep}</option></select>
            {:else}<span class="policy">{file.exists ? (file.conflict === 'keep' ? t.keep : t.replace) : t.copyFile}</span>{/if}
          </div>
        {/each}</div>
      </details>
      {#if unanswered}<p class="muted">{t.resolveConflicts}: {unanswered}</p>{/if}
      <button class="primary" disabled={sending || unanswered > 0} onclick={finish}>{t.confirmInstallation}</button>
    {/if}
    {#if !request && status?.phase === 'completed'}
      <p role="status">{t.installComplete}</p><code class="directory">{status.destination}</code>
      <p class="muted">{status.client ? t.prismReady : t.serverReady}</p>
      {#if status.warning}<div class="notice">{status.warning}</div>{/if}
    {:else if !request && status?.phase === 'failed'}
      <div class="error" role="alert">{t.installFailed}<pre>{status.error}</pre></div>
    {/if}
  </section>
{/if}
