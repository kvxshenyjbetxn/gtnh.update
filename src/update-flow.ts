import type { Rule } from './types';
import type { InstallRequest, InstallStatus } from './installation';

export type UpdateStep = 0 | 1 | 2 | 3;
export type PackChoice = { localPath: string; url?: never } | { url: string; localPath?: never };
export interface UpdateDraft {
  source: string | null;
  pack: PackChoice | null;
  step: UpdateStep;
  completed: number;
}
export function createDraft(source: string | null): UpdateDraft {
  return { source, pack: null, step: 0, completed: 0 };
}
export function changeSource(draft: UpdateDraft, source: string): UpdateDraft {
  if (source === draft.source) return draft;
  return createDraft(source);
}
export function changePack(draft: UpdateDraft, pack: PackChoice): UpdateDraft {
  if (!draft.source || draft.completed < 1) return draft;
  return { ...draft, pack, step: 1, completed: 1 };
}
export function continueStep(draft: UpdateDraft): UpdateDraft {
  if (draft.step === 3 || !draft.source || (draft.step > 0 && !draft.pack)) return draft;
  return { ...draft, step: (draft.step + 1) as UpdateStep, completed: Math.max(draft.completed, draft.step + 1) };
}
export function openStep(draft: UpdateDraft, step: UpdateStep): UpdateDraft {
  return step <= draft.completed ? { ...draft, step } : draft;
}
export function buildInstallRequest(draft: UpdateDraft, client: boolean, rules: Rule[]): InstallRequest | null {
  if (draft.completed < 3 || !draft.source || !draft.pack) return null;
  return { ...draft.pack, client, source: draft.source, rules: rules.map(rule => ({ ...rule })) };
}
export function transactionBusy(phase: InstallStatus['phase'], sending: boolean): boolean {
  return sending || phase === 'preparing' || phase === 'preview' || phase === 'copying';
}
export type DraftAction = { kind: 'source'; source: string } | { kind: 'pack'; pack: PackChoice }
  | { kind: 'continue' } | { kind: 'open'; step: UpdateStep };
export function transitionDraft(draft: UpdateDraft, action: DraftAction, locked: boolean): UpdateDraft {
  if (locked) return draft;
  switch (action.kind) {
    case 'source': return changeSource(draft, action.source);
    case 'pack': return changePack(draft, action.pack);
    case 'continue': return continueStep(draft);
    case 'open': return openStep(draft, action.step);
  }
}
