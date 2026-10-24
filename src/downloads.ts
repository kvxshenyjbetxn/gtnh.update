import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Artifact } from './releases';

export interface DownloadStatus {
  id: string;
  revision: number;
  url: string;
  state: 'downloading' | 'completed' | 'cancelled' | 'failed';
  received: number;
  total: number | null;
  path: string | null;
  error: string | null;
}

interface DownloadTransport {
  listen: (callback: (status: DownloadStatus) => void) => Promise<() => void>;
  status: () => Promise<DownloadStatus[]>;
}

export const downloads = {
  listen: (callback: (status: DownloadStatus) => void) =>
    listen<DownloadStatus>('download-progress', event => callback(event.payload)),
  status: () => invoke<DownloadStatus[]>('download_status'),
  start: (url: string) => invoke<void>('start_download', { url }),
  cancel: (id: string) => invoke<void>('cancel_download', { id }),
};

// Match by file URL, not the selected variant or the most recent global job.
export function downloadsForArtifacts(
  statuses: readonly DownloadStatus[], artifacts: readonly Artifact[], mode: Artifact['mode'],
): { artifact: Artifact; status: DownloadStatus }[] {
  const byUrl = new Map(statuses.map(status => [status.url, status]));
  return artifacts.filter(artifact => artifact.mode === mode).flatMap(artifact => {
    const status = byUrl.get(artifact.url);
    return status ? [{ artifact, status }] : [];
  });
}

// Subscribe before querying; per-URL revisions protect against late snapshots/events.
// Backend state outlives every component, including switching to Settings.
export function watchDownloads(
  transport: DownloadTransport,
  onStatus: (statuses: DownloadStatus[]) => void,
  onError: (error: unknown) => void,
  onReady: () => void,
): () => void {
  let disposed = false;
  const latest = new Map<string, DownloadStatus>();
  let unlisten: (() => void) | undefined;
  function accept(status: DownloadStatus) {
    if (!disposed && status.revision > (latest.get(status.url)?.revision ?? -1)) {
      latest.set(status.url, status);
      onStatus([...latest.values()]);
    }
  }
  void (async () => {
    try {
      unlisten = await transport.listen(accept);
      if (disposed) { unlisten(); return; }
      for (const status of await transport.status()) accept(status);
      if (!disposed) onReady();
    } catch (error) {
      if (!disposed) onError(error);
    }
  })();
  return () => { disposed = true; unlisten?.(); };
}
