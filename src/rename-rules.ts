import type { RenameRule } from './types';

export const fileName = (path: string): string => path.slice(path.lastIndexOf('/') + 1);

// Default new name: the same file name, with a Russian locale switched to the English one.
export function defaultRenameTarget(path: string): string {
  return fileName(path).replace('ru_RU', 'en_US');
}
export function renameRuleFor(path: string): RenameRule {
  return { path, to: defaultRenameTarget(path), enabled: true };
}
// A rename keeps the file in its folder: one non-empty name that differs from the old one.
export function validRenameTarget(rule: Pick<RenameRule, 'path' | 'to'>): boolean {
  return rule.to !== '' && rule.to.trim() === rule.to && !/[\\/]/.test(rule.to)
    && rule.to.toLowerCase() !== fileName(rule.path).toLowerCase();
}
export const renamesValid = (rules: RenameRule[]): boolean => rules.every(validRenameTarget);
