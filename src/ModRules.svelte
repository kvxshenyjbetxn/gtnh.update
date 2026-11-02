<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { messages } from './i18n';
  import { modRuleFrom, upsertModRule } from './mod-rules';
  import type { ModInfo, ModRule } from './types';

  let { mods = $bindable(), modsDirectory, language, disabled, desktop, onChange, picking = $bindable(false) }: {
    mods: ModRule[]; modsDirectory: string; language: 'uk' | 'en'; disabled: boolean;
    desktop: boolean; onChange: () => void; picking?: boolean;
  } = $props();
  let error = $state('');
  let notice = $state('');
  let t = $derived(messages[language]);
  async function pickMods(action: ModRule['action']) {
    if (disabled || !desktop || picking) return;
    picking = true; error = ''; notice = '';
    try {
      const adding = action === 'add';
      const selected = await open({
        multiple: adding, title: adding ? t.addModTitle : t.removeModTitle,
        filters: [{ name: 'Minecraft mod', extensions: ['jar'] }],
        defaultPath: adding || !modsDirectory ? undefined : modsDirectory,
      });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      const errors: string[] = [];
      let changed = false; let replaced = false; let existing = false;
      for (const path of paths) {
        try {
          const info = await invoke<ModInfo>('inspect_mod', { path });
          const result = upsertModRule(mods, modRuleFrom(info, action));
          if (!adding && result.replaced) { existing = true; continue; }
          mods = result.rules; changed = true; replaced ||= result.replaced;
        } catch (e) { errors.push(`${path}: ${String(e)}`); }
      }
      error = errors.join('\n');
      if (changed) { onChange(); if (replaced) notice = t.modRuleUpdated; }
      else if (existing) notice = t.modRuleExists;
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
</script>

<fieldset disabled={disabled || picking}>
  <h4>{t.myMods}</h4>
  <p class="muted">{t.myModsHint}</p>
  <p class="transfer-summary">{t.modsEnabled}: <strong>{mods.filter(mod => mod.enabled).length}</strong> / {mods.length}</p>
  <div class="rule-list">
    {#each mods as mod, index (index)}
      <div class="rule-row mod-rule">
        <input type="checkbox" aria-label={`${mod.name}`} bind:checked={mod.enabled} onchange={onChange} />
        <span class="tag">{mod.action === 'add' ? t.modActionAdd : t.modActionRemove}</span>
        <div class="mod-main">
          <strong>{mod.name}</strong>
          {#if mod.modIds.length}<code>{mod.modIds.join(', ')}</code>
          {:else}<code>{t.modByName}: {mod.stem}</code><span class="muted">{t.modByNameHint}</span>{/if}
          {#if mod.action === 'add' && mod.file}<span class="muted">{t.modSourceFile}: <code>{mod.file}</code></span>{/if}
        </div>
        <button class="remove" aria-label={`${t.remove}: ${mod.name}`} onclick={() => { mods = mods.filter((_, i) => i !== index); onChange(); }}>×</button>
      </div>
    {/each}
  </div>
  <div class="add-form"><div class="picker-actions"><button class="secondary" disabled={!desktop} onclick={() => pickMods('add')}>+ {t.addMod}</button><button class="secondary" disabled={!desktop} onclick={() => pickMods('remove')}>− {t.removeMod}</button></div></div>
  {#if error}<div class="error" role="alert"><pre>{error}</pre></div>{/if}
  {#if notice}<p class="muted" role="status">{notice}</p>{/if}
</fieldset>
