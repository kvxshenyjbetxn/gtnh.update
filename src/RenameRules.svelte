<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { messages } from './i18n';
  import { renameRuleFor, validRenameTarget } from './rename-rules';
  import type { RenameRule } from './types';

  let { rules = $bindable(), source, language, disabled, desktop, onChange, picking = $bindable(false) }: {
    rules: RenameRule[]; source: string; language: 'uk' | 'en'; disabled: boolean;
    desktop: boolean; onChange: () => void; picking?: boolean;
  } = $props();
  let error = $state('');
  let notice = $state('');
  let t = $derived(messages[language]);
  async function pickFiles() {
    if (disabled || !source || !desktop || picking) return;
    picking = true; error = ''; notice = '';
    try {
      const selected = await open({ directory: false, multiple: true, title: t.addFile, defaultPath: source });
      if (!selected) return;
      const paths = await invoke<string[]>('relative_selections', { root: source, paths: Array.isArray(selected) ? selected : [selected] });
      let added = false;
      for (const path of paths) {
        if (rules.some(rule => rule.path.toLowerCase() === path.toLowerCase())) continue;
        rules = [...rules, renameRuleFor(path)]; added = true;
      }
      if (added) onChange(); else notice = t.duplicate;
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
</script>

<fieldset disabled={disabled || picking}>
  <h4>{t.renameTitle}</h4>
  <p class="muted">{t.renameHint}</p>
  <div class="rule-list">
    {#each rules as rule (rule.path)}
      <div class="rule-row">
        <input type="checkbox" aria-label={rule.path} bind:checked={rule.enabled} onchange={onChange} />
        <code class="rule-path">{rule.path}</code>
        <span aria-hidden="true">→</span>
        <input type="text" aria-label={`${t.renameNewName}: ${rule.path}`} aria-invalid={!validRenameTarget(rule)} bind:value={rule.to} onchange={onChange} />
        <button class="remove" aria-label={`${t.remove}: ${rule.path}`} onclick={() => { rules = rules.filter(item => item !== rule); onChange(); }}>×</button>
        {#if !validRenameTarget(rule)}<span class="mod-warn" role="alert">{t.renameInvalid}</span>{/if}
      </div>
    {/each}
  </div>
  <div class="add-form"><div class="picker-actions"><button class="secondary" disabled={!desktop || !source} onclick={pickFiles}>+ {t.addFile}</button></div></div>
  {#if error}<div class="error" role="alert">{error}</div>{/if}
  {#if notice}<p class="muted" role="status">{notice}</p>{/if}
</fieldset>
