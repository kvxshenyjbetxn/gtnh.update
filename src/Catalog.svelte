<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { messages } from './i18n';
  import type { Release } from './releases';
  let { language, mode, includePrereleases, desktop }: {
    language: 'uk' | 'en'; mode: 'client' | 'server'; includePrereleases: boolean; desktop: boolean;
  } = $props();
  let t = $derived(messages[language]);
  let releases = $state<Release[]>([]);
  let busy = $state(false);
  let error = $state('');
  let variants = $state<Record<string, number>>({});
  let visible = $derived(releases.filter(r => includePrereleases || r.channel === 'stable'));

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
  onMount(() => { void fetchCatalog(); });
</script>

<section class="card catalog-card">
  <div class="card-heading"><h2>{t.catalog}</h2><button class="secondary" disabled={!desktop || busy} onclick={fetchCatalog}>{busy ? t.loading : t.refresh}</button></div>
  <p class="muted">{t.catalogHint}</p>
  {#if error}<div class="error" role="alert">{error}</div>{/if}
  {#if !visible.length && !busy && !error}<p class="muted">{t.empty}</p>{/if}
  <div class="release-list" aria-busy={busy}>
    {#each visible as release (release.version)}
      {@const artifacts = release.artifacts.filter(a => a.mode === mode)}
      {@const key = `${mode}:${release.version}`}
      {@const artifact = artifacts[variants[key] ?? 0] ?? artifacts[0]}
      <article class="release-row">
        <div class="release-description"><strong>{release.version}</strong><div><span class="tag">{release.channel === 'stable' ? t.stable : t.betaLabel}</span><time>{release.date}</time></div></div>
        <div class="release-actions">
          {#if artifact}
            <select aria-label={`${t.selectJava}: ${release.version}`} value={variants[key] ?? 0} onchange={e => { variants[key] = Number(e.currentTarget.value); }}>{#each artifacts as file, index}<option value={index}>{file.java}</option>{/each}</select>
            <button class="secondary" disabled={!desktop} onclick={() => open(artifact.url)}>{t.download} ↗</button>
          {/if}
          <button class="text-button" disabled={!desktop} onclick={() => open(release.pageUrl)}>{t.releasePage} ↗</button>
        </div>
      </article>
    {/each}
  </div>
</section>
