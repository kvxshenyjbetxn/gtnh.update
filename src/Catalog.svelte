<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { messages } from './i18n';
  import type { Artifact, Release } from './releases';
  import { downloads, downloadsForArtifacts, watchDownloads, type DownloadStatus } from './downloads';
  import DownloadItem from './DownloadItem.svelte';
  let { language, mode, includePrereleases, desktop, canInstall, onInstall, onSummary }: {
    language: 'uk' | 'en'; mode: 'client' | 'server'; includePrereleases: boolean; desktop: boolean;
    canInstall: boolean; onInstall: (artifact: Artifact) => void; onSummary: (summary: string) => void;
  } = $props();
  let t = $derived(messages[language]);
  let releases = $state<Release[]>([]);
  let busy = $state(false);
  let error = $state('');
  let variants = $state<Record<string, number>>({});
  let statuses = $state<DownloadStatus[]>([]);
  let downloadReady = $state(false);
  let starting = $state<Record<string, boolean>>({});
  let cancelling = $state<Record<string, boolean>>({});
  let fileErrors = $state<Record<string, string>>({});
  let byUrl = $derived(new Map(statuses.map(status => [status.url, status])));
  let visible = $derived(releases.filter(r => includePrereleases || r.channel === 'stable'));
  $effect(() => {
    const urls = new Set(releases.flatMap(release => release.artifacts.filter(a => a.mode === mode).map(a => a.url)));
    const jobs = statuses.filter(status => urls.has(status.url));
    const running = jobs.filter(job => job.state === 'downloading');
    const ready = jobs.filter(job => job.state === 'completed').length;
    const failed = jobs.filter(job => job.state === 'failed').length;
    const total = running.reduce((sum, job) => sum + (job.total ?? 0), 0);
    const received = running.reduce((sum, job) => sum + job.received, 0);
    const percent = running.length && running.every(job => job.total) ? ` (${Math.min(100, Math.floor(received / total * 100))}%)` : '';
    onSummary([running.length ? `${t.downloadsActive}: ${running.length}${percent}` : '', ready ? `${t.downloadsReady}: ${ready}` : '', failed ? `${t.downloadsFailed}: ${failed}` : ''].filter(Boolean).join(' · '));
  });

  async function fetchCatalog() {
    if (!desktop || busy) return;
    busy = true; error = '';
    try { releases = await invoke<Release[]>('list_releases'); }
    catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function open(url: string) {
    try { await invoke('open_release_url', { url }); }
    catch (e) { error = String(e); }
  }
  async function startDownload(url: string) {
    if (starting[url] || byUrl.get(url)?.state === 'downloading' || !downloadReady) return;
    starting[url] = true; fileErrors[url] = '';
    try { await downloads.start(url); }
    catch (e) { fileErrors[url] = String(e); }
    finally { starting[url] = false; }
  }
  async function cancelDownload(status: DownloadStatus) {
    if (cancelling[status.id]) return;
    cancelling[status.id] = true; fileErrors[status.url] = '';
    try { await downloads.cancel(status.id); }
    catch (e) { fileErrors[status.url] = String(e); cancelling[status.id] = false; }
  }
  onMount(() => {
    void fetchCatalog();
    if (!desktop) return;
    return watchDownloads(downloads, snapshot => {
      statuses = snapshot;
      for (const status of snapshot) {
        if (status.state !== 'downloading') delete cancelling[status.id];
      }
    }, e => { error = String(e); }, () => { downloadReady = true; });
  });
</script>

<section class="card catalog-card">
  <div class="card-heading"><h2>{t.catalog}</h2><button class="secondary" disabled={!desktop || busy} onclick={fetchCatalog}>{busy ? t.loading : t.refresh}</button></div>
  {#if error}<div class="error" role="alert">{error}</div>{/if}
  {#if !visible.length && !busy && !error}<p class="muted">{t.empty}</p>{/if}
  <div class="release-list" aria-busy={busy}>
    {#each visible as release (release.version)}
      {@const artifacts = release.artifacts.filter(a => a.mode === mode)}
      {@const key = `${mode}:${release.version}`}
      {@const artifact = artifacts[variants[key] ?? 0] ?? artifacts[0]}
      {@const jobs = downloadsForArtifacts(statuses, release.artifacts, mode)}
      <article class="release-row">
        <div class="release-main">
          <div class="release-description"><strong>{release.version}</strong><div><span class="tag">{release.channel === 'stable' ? t.stable : t.betaLabel}</span><time>{release.date}</time></div></div>
          <div class="release-actions">
            {#if artifact}
              <select aria-label={`${t.selectJava}: ${release.version}`} value={variants[key] ?? 0} onchange={e => { variants[key] = Number(e.currentTarget.value); }}>{#each artifacts as file, index}<option value={index}>{file.java}</option>{/each}</select>
              <button class="secondary" disabled={!desktop || !downloadReady || starting[artifact.url] || byUrl.get(artifact.url)?.state === 'downloading'} onclick={() => startDownload(artifact.url)}>{t.download}</button>
            {/if}
            <button class="text-button" disabled={!desktop} onclick={() => open(release.pageUrl)}>{t.releasePage} ↗</button>
          </div>
        </div>
        {#each jobs as job (job.status.id)}
          <DownloadItem {language} java={job.artifact.java} status={job.status}
            starting={!downloadReady || !!starting[job.status.url]} cancelling={!!cancelling[job.status.id]}
            error={fileErrors[job.status.url] || ''}
            onRetry={() => startDownload(job.status.url)} onCancel={() => cancelDownload(job.status)}
            {canInstall} onInstall={() => onInstall(job.artifact)} />
        {/each}
        {#if artifact && fileErrors[artifact.url] && !jobs.some(job => job.status.url === artifact.url)}
          <div class="error" role="alert">{fileErrors[artifact.url]}</div>
        {/if}
      </article>
    {/each}
  </div>
</section>
