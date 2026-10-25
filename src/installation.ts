import type { Rule } from './types';

export type InstallRequest = {
  client: boolean;
  source: string;
  rules: Rule[];
} & ({ url: string; localPath?: never } | { url?: never; localPath: string });

export function packName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop()?.replace(/\.zip$/i, '') || 'GTNH';
}
export interface Transfer {
  path: string;
  exists: boolean;
  directory: boolean;
  conflict: Rule['conflict'];
  bytes: number;
}
export interface Preview { destination: string; files: Transfer[]; missing: string[] }
export interface InstallStatus {
  revision: number;
  phase: '' | 'preparing' | 'preview' | 'copying' | 'completed' | 'failed';
  url: string;
  client: boolean;
  progress: { stage: 'extracting' | 'preparingPack' | 'copying' | 'publishing'; done: number; total: number } | null;
  preview: Preview | null;
  destination: string | null;
  error: string | null;
  warning: string | null;
}
export function unresolvedConflicts(preview: Preview, decisions: Record<string, Rule['conflict']>): string[] {
  return preview.files.filter(file => file.exists && !file.directory && file.conflict === 'ask'
    && decisions[file.path] !== 'replace' && decisions[file.path] !== 'keep').map(file => file.path);
}

// One process-owned installation survives tab changes. Poll without overlapping requests;
// revisions also protect against a delayed response after a command updates the UI.
export function watchInstallation(transport: {
  status: () => Promise<InstallStatus>;
  repeat: (callback: () => void) => () => void;
}, accept: (status: InstallStatus) => void, onError: (error: unknown) => void): () => void {
  let disposed = false;
  let pending = false;
  let revision = -1;
  async function refresh() {
    if (disposed || pending) return;
    pending = true;
    try {
      const status = await transport.status();
      if (!disposed && status.revision > revision) { revision = status.revision; accept(status); }
    } catch (error) { if (!disposed) onError(error); }
    finally { pending = false; }
  }
  void refresh();
  const stop = transport.repeat(() => { void refresh(); });
  return () => { disposed = true; stop(); };
}
