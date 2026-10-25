<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke } from '@tauri-apps/api/core';
  import { messages } from './i18n';
  import type { Rule } from './types';

  let { rules = $bindable(), source, language, disabled, client, desktop, onChange, picking = $bindable(false) }: {
    rules: Rule[]; source: string; language: 'uk' | 'en'; disabled: boolean; client: boolean;
    desktop: boolean; onChange: () => void; picking?: boolean;
  } = $props();
  let error = $state('');
  let notice = $state('');
  let t = $derived(messages[language]);
  async function pickRules(folder: boolean) {
    if (disabled || !source || !desktop || picking) return;
    picking = true; error = ''; notice = '';
    try {
      const selected = await open({ directory: folder, multiple: true, title: folder ? t.addFolder : t.addFile, defaultPath: source });
      if (!selected) return;
      const paths = await invoke<string[]>('relative_selections', { root: source, paths: Array.isArray(selected) ? selected : [selected] });
      let added = false;
      for (const path of paths) {
        if (rules.some(rule => rule.path.toLowerCase() === path.toLowerCase())) continue;
        rules = [...rules, { path, enabled: true, conflict: 'ask', custom: true }]; added = true;
      }
      if (added) onChange(); else notice = t.duplicate;
    } catch (e) { error = String(e); }
    finally { picking = false; }
  }
</script>

<fieldset disabled={disabled || picking}>
  <p class="muted">{client ? t.clientHint : t.serverHint}</p>
  <p class="transfer-summary">{t.enabledRules}: <strong>{rules.filter(rule => rule.enabled).length}</strong> / {rules.length}</p>
  <details class="rules-details">
    <summary>{t.allRules}</summary>
    <div class="rule-list">
      {#each rules as rule (rule.path)}
        <div class="rule-row">
          <label class="rule-path"><input type="checkbox" bind:checked={rule.enabled} onchange={onChange} /><code>{rule.path}</code></label>
          <span class="tag">{rule.custom ? t.custom : t.standard}</span>
          {#if rule.custom}
            <select aria-label={`${t.conflict}: ${rule.path}`} bind:value={rule.conflict} onchange={onChange}><option value="ask">{t.ask}</option><option value="replace">{t.replace}</option><option value="keep">{t.keep}</option></select>
            <button class="remove" aria-label={`${t.remove}: ${rule.path}`} onclick={() => { rules = rules.filter(item => item !== rule); onChange(); }}>×</button>
          {:else}<span class="policy">{t.replace}</span>{/if}
        </div>
      {/each}
    </div>
    <div class="add-form"><div class="picker-actions"><button class="secondary" disabled={!desktop || !source} onclick={() => pickRules(false)}>+ {t.addFile}</button><button class="secondary" disabled={!desktop || !source} onclick={() => pickRules(true)}>+ {t.addFolder}</button></div><p class="muted small">{t.selectionHint}</p></div>
  </details>
  {#if error}<div class="error" role="alert">{error}</div>{/if}
  {#if notice}<p class="muted" role="status">{notice}</p>{/if}
</fieldset>
