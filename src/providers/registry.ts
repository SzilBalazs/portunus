import type { ComponentType } from 'react';
import type { CommandDescriptor, Config, ExtensionResult, SearchResult } from '../types';

export interface LaunchContext {
  setQuery: (q: string) => void;
  setResults: (r: SearchResult[]) => void;
  requery: () => void;
  /** Invoke a command (enter its scope, seed its alias, or run its action). */
  runCommand: (command: CommandDescriptor) => void;
  /** Route an extension activation through the shared response flow
   *  (optimistic hide, forms, toast queue, refresh-results). */
  activateExtension: (req: {
    id: string;
    ext: ExtensionResult;
    action: string | null;
    command: string | null;
    formValues?: Record<string, unknown>;
    /** The action/command declared `opens_form` - skip the optimistic hide. */
    opensForm?: boolean;
  }) => void;
  config: Config | null;
}

export interface PreviewProps {
  result: SearchResult;
  onLaunch: () => void;
  onReveal?: () => void;
  /** Matched content-search terms to highlight in the preview (empty otherwise). */
  terms?: string[];
  /** Whether matched-term highlighting is enabled (Ctrl+H toggle; PDF overlay). */
  highlight?: boolean;
  /** Rendered inside the full-card Quicklook overlay - enables large/scrollable layouts. */
  quicklook?: boolean;
}

export interface ProviderPlugin {
  kinds: string[];
  Preview: ComponentType<PreviewProps> | null;
  handleLaunch?: (result: SearchResult, ctx: LaunchContext) => boolean;
  handleKeyDown?: (e: KeyboardEvent, result: SearchResult | null, ctx: LaunchContext) => boolean;
}

/** The copy chord shared by calc/dict/file providers: Ctrl+C (not Ctrl+Alt+C). */
export const isCopyKey = (e: KeyboardEvent): boolean =>
  e.ctrlKey && !e.altKey && e.key === 'c';

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
