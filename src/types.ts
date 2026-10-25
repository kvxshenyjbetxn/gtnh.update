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
}
