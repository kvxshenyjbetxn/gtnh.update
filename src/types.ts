export type Conflict = 'ask' | 'replace' | 'keep';
export interface Rule {
  path: string;
  enabled: boolean;
  conflict: Conflict;
  custom: boolean;
}
export interface Settings {
  schemaVersion: number;
  language: 'uk' | 'en';
  theme: 'amoled' | 'light';
  includePrereleases: boolean;
  clientSource: string | null;
  serverSource: string | null;
  prismInstancesDirectory: string | null;
  clientRules: Rule[];
  serverRules: Rule[];
  clientMods: ModRule[];
  serverMods: ModRule[];
  clientRenames: RenameRule[];
}
export interface RenameRule { path: string; to: string; enabled: boolean }
export interface ModRule {
  action: 'add' | 'remove';
  enabled: boolean;
  name: string;
  modIds: string[];
  stem: string;
  file: string | null;
}
export interface RenamePreview { path: string; to: string; found: boolean; replaces: boolean }
export interface ModInfo { name: string; modIds: string[]; stem: string; file: string }
export interface ModRemoval { name: string; files: string[] }
export interface ModAddition { name: string; file: string; source: string; replaces: string[]; missing: boolean }
export interface ModPlan { removed: ModRemoval[]; added: ModAddition[]; duplicates: string[][] }
