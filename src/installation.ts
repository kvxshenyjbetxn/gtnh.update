import type { ModPlan, ModRule, RenamePreview, RenameRule, Rule } from './types';

type PackSource = { url: string; localPath?: never } | { url?: never; localPath: string };
export type ClientInstallRequest = { client: true; source: string; rules: Rule[]; mods: ModRule[]; renames: RenameRule[] } & PackSource;
export type ServerInstallRequest = { client: false; server: string; rules: Rule[]; mods: ModRule[] } & PackSource;
export type InstallRequest = ClientInstallRequest | ServerInstallRequest;

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
export interface ServerItem { path: string; directory: boolean; old: boolean; new: boolean }
export interface ScriptChange { path: string; old: string | null; new: string | null }
export interface ServerPreview {
  server: string; backup: string; previousBackup: boolean;
  replaced: ServerItem[]; added: string[]; journeyMap: boolean;
  scripts: ScriptChange[]; javaChange: 'toJava17' | 'toJava8' | null;
  recovered: 'rolledBack' | 'completed' | null;
  mods: ModPlan; files: Transfer[]; missing: string[];
}
export interface Preview { destination: string; files: Transfer[]; missing: string[]; mods: ModPlan; renames: RenamePreview[] }
export interface InstallStatus {
  revision: number;
  phase: '' | 'preparing' | 'preview' | 'copying' | 'completed' | 'failed';
  url: string;
  client: boolean;
  progress: { stage: 'extracting' | 'preparingPack' | 'copying' | 'replacing' | 'publishing'; done: number; total: number } | null;
  preview: Preview | null;
  serverPreview: ServerPreview | null;
  destination: string | null;
  error: string | null;
}
export function unresolvedConflicts(preview: { files: Transfer[] }, decisions: Record<string, Rule['conflict']>): string[] {
  return preview.files.filter(file => file.exists && !file.directory && file.conflict === 'ask'
    && decisions[file.path] !== 'replace' && decisions[file.path] !== 'keep').map(file => file.path);
}

// One process-owned installation survives tab changes. Poll without overlapping requests;
// revisions also protect against a delayed response after a command updates the UI.
export function watchInstallation(transport: {
  status: (since?: number) => Promise<InstallStatus | null>;
  repeat: (callback: () => void) => () => void;
}, accept: (status: InstallStatus) => void, onError: (error: unknown) => void): () => void {
  let disposed = false;
  let pending = false;
  let revision = -1;
  async function refresh() {
    if (disposed || pending) return;
    pending = true;
    try {
      const status = await transport.status(revision < 0 ? undefined : revision);
      if (!disposed && status && status.revision > revision) { revision = status.revision; accept(status); }
    } catch (error) { if (!disposed) onError(error); }
    finally { pending = false; }
  }
  void refresh();
  const stop = transport.repeat(() => { void refresh(); });
  return () => { disposed = true; stop(); };
}

export interface TransferGroup { path: string; directory: boolean; file: Transfer; files: number; bytes: number }
// Top-level entries are those with no ancestor path that is itself an entry.
// Descendant file counts and sizes are credited to every ancestor entry via the Map.
export function groupTransfers(files: Transfer[]): TransferGroup[] {
  const known = new Set<string>();
  for (const file of files) known.add(file.path);
  const groups = new Map<string, TransferGroup>();
  for (const file of files) {
    let top = file.path;
    // Highest ancestor that is itself an entry (intermediate folders may be absent from the list).
    for (let cut = file.path.lastIndexOf('/'); cut > 0; cut = file.path.lastIndexOf('/', cut - 1)) {
      const parent = file.path.slice(0, cut);
      if (known.has(parent)) top = parent;
    }
    let group = groups.get(top);
    if (!group) {
      const entry = top === file.path ? file : undefined;
      group = { path: top, directory: !!entry?.directory, file: entry as Transfer, files: 0, bytes: 0 };
      groups.set(top, group);
    }
    if (top === file.path) { group.file = file; group.directory = file.directory; }
    if (!file.directory) { group.files += 1; group.bytes += file.bytes; }
  }
  return [...groups.values()];
}
export function formatBytes(value: number, language: string): string {
  const units = ['B', 'KiB', 'MiB', 'GiB'];
  let size = value; let unit = 0;
  while (size >= 1024 && unit < units.length - 1) { size /= 1024; unit += 1; }
  return `${size.toLocaleString(language, { maximumFractionDigits: unit ? 1 : 0 })} ${units[unit]}`;
}
