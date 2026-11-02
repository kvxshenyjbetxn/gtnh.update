<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { messages } from './i18n';
  import { formatBytes, groupTransfers, packName, unresolvedConflicts, watchInstallation, type InstallRequest, type InstallStatus, type ServerItem, type Transfer } from './installation';
  import type { ModPlan, Rule } from './types';
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
  let serverPreview = $derived(status?.phase === 'preview' && !status.client ? status.serverPreview : null);
  let current = $derived(preview ?? serverPreview);
  let entries = $derived(current?.files ?? []);
  let files = $derived(entries.filter(file => !file.directory));
  let folders = $derived(entries.filter(file => file.directory));
  let asks = $derived(files.filter(file => file.exists && file.conflict === 'ask'));
  let groups = $derived(groupTransfers(entries));
  let listOpen = $state(false);
  let limit = $state(200);
  let unanswered = $derived(current ? unresolvedConflicts(current, decisions).length : 0);

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
  const itemLabel = (item: ServerItem) => item.old && item.new ? t.itemReplace : item.old ? t.itemRemove : t.itemAdd;
  async function prepare() {
    if (!request || sending) return;
    const snapshot = $state.snapshot(request) as InstallRequest;
    if (snapshot.client && (!parent || !name)) return;
    sending = true; error = ''; decisions = {};
    try {
      if (snapshot.client) await invoke('prepare_installation', { request: { ...snapshot, parent, name } });
      else await invoke('prepare_server_update', { request: { server: snapshot.server, ...(snapshot.url ? { url: snapshot.url } : { localPath: snapshot.localPath }), rules: snapshot.rules, mods: snapshot.mods } });
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

{#snippet modsBlock(plan: ModPlan)}
  {#if plan.removed.length || plan.added.length || plan.duplicates.length}
    <h4>{t.myMods}</h4>
    {#if plan.duplicates.length}
      <div class="error" role="alert"><strong>{t.modDuplicates}</strong>
        {#each plan.duplicates as group, index (index)}<code class="directory">{group.join(' · ')}</code>{/each}
      </div>
    {/if}
    {#if plan.removed.length}
      <details open><summary>{t.modsRemovedTitle} ({plan.removed.length})</summary>
        {#each plan.removed as item, index (index)}
          <div class="mod-item"><strong>{item.name}</strong>
            {#if item.files.length}{#each item.files as file (file)}<code>{file}</code>{/each}
            {:else}<span class="notice" role="alert">{t.modNoMatch}</span>{/if}
          </div>
        {/each}
      </details>
    {/if}
    {#if plan.added.length}
      <details open><summary>{t.modsAddedTitle} ({plan.added.length})</summary>
        {#each plan.added as item, index (index)}
          <div class="mod-item"><strong>{item.name}</strong><code>{item.file}</code><span class="muted">{t.modSourceFile}: <code>{item.source}</code></span>
            {#if item.replaces.length}<span class="muted">{t.modReplaces}: {item.replaces.join(', ')}</span>{/if}
            {#if item.missing}<strong class="mod-warn" role="alert">{t.modMissing}</strong>{/if}
          </div>
        {/each}
      </details>
    {/if}
  {/if}
{/snippet}

{#snippet transfers(missing: string[], notifyEmpty: boolean)}
  <p>{t.transferCount}: {files.length} · {t.transferFolders}: {folders.length}</p>
  {#if notifyEmpty && !entries.length}<p class="notice" role="status">{t.nothingToTransfer}</p>{/if}
  {#if missing.length}<details open={!entries.length}><summary>{t.skippedMissing} ({missing.length})</summary>{#each missing as path}<code class="directory">{path}</code>{/each}</details>{/if}
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
{/snippet}

{#if shown}
  <section class="card installation-card" aria-label={t.install}>
    <div class="card-heading"><h2>{t.install}</h2><button class="secondary" disabled={working || sending} onclick={close}>{active ? t.cancelDownload : t.close}</button></div>
    {#if error}<div class="error" role="alert">{error}</div>{/if}
    {#if request && !active}
      {#if request.client}
        {#if !parent}<button class="secondary" disabled={sending} onclick={onSettings}>{t.configurePrism}</button>{/if}
        <label class="install-name">{t.newInstanceName}<input bind:value={name} disabled={sending} /></label>
        <button class="primary" disabled={!parent || !name || sending} onclick={prepare}>{t.prepareInstallation}</button>
      {:else}
        <code class="directory">{request.server}</code>
        <button class="primary" disabled={sending} onclick={prepare}>{t.prepareServerUpdate}</button>
      {/if}
    {/if}
    {#if working}
      <p role="status">{status?.progress ? t[status.progress.stage] : status?.client === false ? t.prepareServerUpdate : t.prepareInstallation}…</p>
      <progress aria-label={t.installProgress} max={status?.progress?.total || undefined} value={status?.progress?.total ? status.progress.done : undefined}></progress>
      {#if status?.progress}<p class="muted">{status.progress.done} / {status.progress.total}</p>{/if}
    {/if}
    {#if preview}
      <h3>{t.transferPreview}</h3><code class="directory">{preview.destination}</code>
      <p class="notice">{t.installWarning}</p>
      {@render modsBlock(preview.mods)}
      {#if preview.renames.length}
        <h4>{t.renameTitle}</h4>
        {#each preview.renames as item (item.path)}
          <div class="mod-item"><code>{item.path} → {item.to}</code>
            {#if !item.found}<span class="muted">{t.renameNotFound}</span>
            {:else if item.replaces}<span class="muted">{t.renameReplaces}</span>{/if}
          </div>
        {/each}
      {/if}
      {@render transfers(preview.missing, true)}
      <button class="primary" disabled={sending || unanswered > 0} onclick={finish}>{t.confirmInstallation}</button>
    {/if}
    {#if serverPreview}
      <h3>{t.serverPreviewTitle}</h3>
      <p class="muted">{t.serverFolderLabel}</p><code class="directory">{serverPreview.server}</code>
      {#if serverPreview.recovered}<p class="notice" role="status">{serverPreview.recovered === 'rolledBack' ? t.serverRolledBack : t.serverCompleted}</p>{/if}
      {#if serverPreview.javaChange}<p class="notice" role="alert">{serverPreview.javaChange === 'toJava17' ? t.javaToJava17 : t.javaToJava8}</p>{/if}
      <p class="notice">{t.serverStopWarning}</p>
      {#if serverPreview.replaced.length}
        <details open><summary>{t.serverReplacedItems} ({serverPreview.replaced.length})</summary>
          <div class="transfer-list">{#each serverPreview.replaced as item (item.path)}
            <div class="rule-row"><code class="rule-path">{item.path}{item.directory ? '/' : ''}</code><span class="policy">{itemLabel(item)}</span></div>
          {/each}</div>
        </details>
      {/if}
      {#if serverPreview.added.length}
        <details><summary>{t.serverAdded} ({serverPreview.added.length})</summary>
          <div class="transfer-list">{#each serverPreview.added as path (path)}
            <div class="rule-row"><code class="rule-path">{path}</code><span class="policy">{t.itemAdd}</span></div>
          {/each}</div>
        </details>
      {/if}
      {@render modsBlock(serverPreview.mods)}
      {#if serverPreview.files.length || serverPreview.missing.length}<h4>{t.serverFilesTitle}</h4>{/if}
      {@render transfers(serverPreview.missing, false)}
      {#if serverPreview.journeyMap}<p class="muted">{t.serverJourneyKept}</p>{/if}
      {#if serverPreview.scripts.length}
        <h4>{t.serverScripts} ({serverPreview.scripts.length})</h4>
        {#each serverPreview.scripts as script (script.path)}
          <details><summary>{script.path}{script.new === null ? ` · ${t.scriptRemoved}` : ''}</summary>
            <div class="script-diff">
              {#if script.old !== null}<div><strong>{t.scriptOld}</strong><pre>{script.old}</pre></div>{/if}
              {#if script.new !== null}<div><strong>{t.scriptNew}</strong><pre>{script.new}</pre></div>{/if}
            </div>
          </details>
        {/each}
        <p class="muted">{t.scriptsNote}</p>
      {/if}
      <p class="muted">{t.serverBackupLabel}</p><code class="directory">{serverPreview.backup}</code>
      {#if serverPreview.previousBackup}<p class="notice">{t.backupReplaced}</p>{/if}
      <button class="primary" disabled={sending || unanswered > 0} onclick={finish}>{t.confirmServerUpdate}</button>
    {/if}
    {#if !request && status?.phase === 'completed'}
      {#if status.client}
        <p role="status">{t.installComplete}</p><code class="directory">{status.destination}</code>
        <p class="muted">{t.prismReady}</p>
      {:else}
        <p role="status">{t.serverUpdated}</p><code class="directory">{status.destination}</code>
        {#if status.serverPreview}<p class="muted">{t.serverBackupLabel}</p><code class="directory">{status.serverPreview.backup}</code>{/if}
        <p class="muted">{t.serverDoneHint}</p>
      {/if}
    {:else if !request && status?.phase === 'failed'}
      <div class="error" role="alert">{status.client ? t.installFailed : t.serverUpdatedFailed}<pre>{status.error}</pre></div>
    {/if}
  </section>
{/if}
