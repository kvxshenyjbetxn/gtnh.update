export interface Artifact { mode: 'client' | 'server'; java: string; url: string }
export interface Release { version: string; channel: 'stable' | 'beta'; date: string; pageUrl: string; artifacts: Artifact[] }
