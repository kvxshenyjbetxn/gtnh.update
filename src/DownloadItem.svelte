<script lang="ts">
  import { messages } from './i18n';
  import type { DownloadStatus } from './downloads';

  let { language, java, status, starting, cancelling, error, onRetry, onCancel }: {
    language: 'uk' | 'en'; java: string; status: DownloadStatus;
    starting: boolean; cancelling: boolean; error: string;
    onRetry: () => void; onCancel: () => void;
  } = $props();
  let t = $derived(messages[language]);
  let percent = $derived(status.total ? Math.min(100, Math.floor(status.received / status.total * 100)) : null);
  function bytes(value: number) {
    return `${(value / 1024 / 1024).toLocaleString(language, { maximumFractionDigits: 1 })} MiB`;
  }
</script>

<div class="download-status" aria-label={`${t.download}: ${java}`}>
  <strong class="download-variant">{java}</strong>
  <code class="download-name">{status.url.split('/').pop()}</code>
  {#if status.state === 'downloading'}
    <progress aria-label={`${t.downloadProgress}: ${java}`} max={status.total || undefined} value={status.total ? status.received : undefined}></progress>
    <div class="download-details">
      <span>{bytes(status.received)}{#if status.total} / {bytes(status.total)} ({percent}%){/if}</span>
      <button class="secondary" disabled={cancelling} onclick={onCancel}>{cancelling ? t.cancelling : t.cancelDownload}</button>
    </div>
  {:else if status.state === 'completed'}
    <p role="status">{t.downloadComplete}</p>
    <code class="directory">{status.path}</code>
  {:else if status.state === 'cancelled'}
    <p role="status">{t.downloadCancelled}</p>
    <button class="secondary" disabled={starting} onclick={onRetry}>{t.retry}</button>
  {:else if status.state === 'failed'}
    <div class="error" role="alert">{t.downloadFailed}<pre>{status.error}</pre></div>
    <button class="secondary" disabled={starting} onclick={onRetry}>{t.retry}</button>
  {/if}
  {#if error}<div class="error" role="alert">{error}</div>{/if}
</div>
