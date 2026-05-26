import type { ComponentType } from 'react';
import type { SearchResult } from '../types';

export interface LaunchContext {
  setQuery: (q: string) => void;
  setResults: (r: SearchResult[]) => void;
  requery: () => void;
  removeExpiredTimer: (id: number) => void;
}

export interface PreviewProps {
  result: SearchResult;
  onLaunch: () => void;
  onStopTimer?: () => void;
}

export interface ProviderPlugin {
  kinds: string[];
  Preview: ComponentType<PreviewProps> | null;
  handleLaunch?: (result: SearchResult, ctx: LaunchContext) => boolean;
  handleKeyDown?: (e: KeyboardEvent, result: SearchResult | null, ctx: LaunchContext) => boolean;
}

const plugins: ProviderPlugin[] = [];

export function registerProvider(p: ProviderPlugin): void {
  plugins.push(p);
}

export function getPreview(kind: string | undefined): ComponentType<PreviewProps> | null {
  if (!kind) return null;
  for (const p of plugins) {
    if (p.Preview && p.kinds.includes(kind)) return p.Preview;
  }
  return null;
}

export function dispatchLaunch(result: SearchResult, ctx: LaunchContext): boolean {
  for (const p of plugins) {
    if (p.handleLaunch && p.handleLaunch(result, ctx)) return true;
  }
  return false;
}

export function dispatchKeyDown(
  e: KeyboardEvent,
  result: SearchResult | null,
  ctx: LaunchContext,
): boolean {
  for (const p of plugins) {
    if (p.handleKeyDown && p.handleKeyDown(e, result, ctx)) return true;
  }
  return false;
}
