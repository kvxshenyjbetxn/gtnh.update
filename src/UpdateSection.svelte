<script lang="ts">
  import type { Snippet } from 'svelte';
  let { id, number, title, summary, expanded, locked, lockedReason = '', onToggle, children }: {
    id: string; number: number; title: string; summary: string; expanded: boolean;
    locked: boolean; lockedReason?: string; onToggle: () => void; children?: Snippet;
  } = $props();
</script>

<section class="update-section" class:expanded class:locked aria-labelledby={`${id}-heading`}>
  <h2 id={`${id}-heading`} tabindex="-1" class="step-heading">
    <button class="step-toggle" aria-expanded={expanded} aria-controls={`${id}-content`} disabled={locked} onclick={onToggle}>
      <span class="step-number" aria-hidden="true">{number}</span>
      <span class="step-label"><strong>{title}</strong><span class="step-summary">{locked && lockedReason ? `${lockedReason} · ${summary}` : summary}</span></span>
      <span class="step-chevron" aria-hidden="true">{expanded ? '−' : '+'}</span>
    </button>
  </h2>
  <div id={`${id}-content`} class="step-content" hidden={!expanded}>
    {@render children?.()}
  </div>
</section>
