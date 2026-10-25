import type { OpenDialogOptions } from '@tauri-apps/plugin-dialog';

export async function chooseLocalPack(
  kind: 'zip' | 'folder',
  transport: { open: (options: OpenDialogOptions) => Promise<string | string[] | null> },
  title: string,
  defaultPath?: string,
): Promise<string | null> {
  const selected = await transport.open({
    directory: kind === 'folder', multiple: false, title, defaultPath,
    ...(kind === 'zip' ? { filters: [{ name: 'ZIP', extensions: ['zip'] }] } : {}),
  });
  if (Array.isArray(selected)) throw new Error('Expected a single pack selection');
  return selected;
}

export async function resolvePrismDirectory(
  saved: string | null,
  validate: (path: string) => Promise<string>,
): Promise<string | null> {
  return saved ? validate(saved) : null;
}
