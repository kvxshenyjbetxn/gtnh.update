import type { ModInfo, ModRule } from './types';

// Same identity: shared mod ids, or (when either side has no ids) the same file stem.
export function sameMod(a: Pick<ModRule, 'modIds' | 'stem'>, b: Pick<ModRule, 'modIds' | 'stem'>): boolean {
  if (a.modIds.length && b.modIds.length) {
    const ids = new Set(a.modIds.map(id => id.toLowerCase()));
    return b.modIds.some(id => ids.has(id.toLowerCase()));
  }
  return a.stem.toLowerCase() === b.stem.toLowerCase();
}
export function modRuleFrom(info: ModInfo, action: ModRule['action']): ModRule {
  return { action, enabled: true, name: info.name, modIds: [...info.modIds], stem: info.stem, file: action === 'add' ? info.file : null };
}
// A new rule replaces an existing rule of the same action and identity (the user is updating a mod version).
export function upsertModRule(rules: ModRule[], next: ModRule): { rules: ModRule[]; replaced: boolean } {
  const index = rules.findIndex(rule => rule.action === next.action && sameMod(rule, next));
  if (index < 0) return { rules: [...rules, next], replaced: false };
  return { rules: rules.map((rule, i) => i === index ? next : rule), replaced: true };
}
