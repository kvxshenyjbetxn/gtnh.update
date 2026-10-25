<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { messages } from './i18n';

  interface Entry { name: string; path: string; directory: boolean }
  interface Listing { path: string; parent: string | null; entries: Entry[]; locations: string[] }
  let { language, onSelect, onClose }: {
    language: 'uk' | 'en'; onSelect: (path: string) => void; onClose: () => void;
  } = $props();
  let t = $derived(messages[language]);
  let dialog: HTMLDialogElement;
  let listing = $state<Listing | null>(null);
  let selected = $state<Entry | null>(null);
  let busy = $state(false);
  let error = $state('');
  async function browse(path?: string) {
    if (busy) return;
    busy = true; error = '';
    try {
      listing = await invoke<Listing>('browse_packs', { path: path ?? null });
      selected = null;
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  function choose() {
    if (!listing || busy) return;
    onSelect(selected?.path ?? listing.path);
  }
  onMount(() => { dialog.showModal(); void browse(); });
</script>

<dialog bind:this={dialog} class="pack-picker" oncancel={onClose} aria-label={t.choosePack}>
  <div class="card-heading"><h2>{t.choosePack}</h2><button class="secondary" onclick={onClose}>{t.close}</button></div>
  <p class="muted">{t.localPackHint}</p>
  <div class="picker-actions">
    <button class="secondary" disabled={busy || !listing?.parent} onclick={() => browse(listing?.parent ?? undefined)}>↑ {t.parentFolder}</button>
    <select aria-label={t.packLocations} disabled={busy} value="" onchange={e => { if (e.currentTarget.value) void browse(e.currentTarget.value); }}>
      <option value="">{t.packLocations}</option>
      {#each listing?.locations ?? [] as path}<option value={path}>{path}</option>{/each}
    </select>
    {#if selected?.directory}<button class="secondary" disabled={busy} onclick={() => browse(selected?.path)}>{t.enterFolder}</button>{/if}
  </div>
  {#if listing}<code class="directory">{listing.path}</code>{/if}
  {#if error}<div class="error" role="alert">{error}<button class="secondary" onclick={() => browse(listing?.path)}>{t.retry}</button></div>{/if}
  {#if busy}<p role="status">{t.loading}</p>{/if}
  <div class="pack-entries" aria-busy={busy}>
    {#each listing?.entries ?? [] as entry (entry.path)}
      <button class="pack-entry" class:selected={selected?.path === entry.path} aria-pressed={selected?.path === entry.path} disabled={busy}
        onclick={() => { selected = entry; }} ondblclick={() => { if (entry.directory) void browse(entry.path); else choose(); }}>
        <span aria-hidden="true">{entry.directory ? '▤' : 'ZIP'}</span><span>{entry.name}</span>
      </button>
    {/each}
  </div>
  <p class="muted">{t.selectedPack}: <code>{selected?.path ?? listing?.path ?? '—'}</code></p>
  <button class="primary" disabled={!listing || busy} onclick={choose}>{t.choosePack}</button>
</dialog>
