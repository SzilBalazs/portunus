import type { ComponentType } from 'react';
import type { CommandDescriptor, Config, ExtensionResult, SearchResult, ToastLevel } from '../types';
import type { ActionDescriptor } from '../actions/types';
import { matchesShortcut } from '../actions/shortcut';
import { effectiveActionShortcut, matchesBuiltin } from '../keybinds/store';

export interface LaunchContext {
  setQuery: (q: string) => void;
  setResults: (r: SearchResult[]) => void;
  requery: () => void;
  /** Queue a launcher toast (bottom-left stack, auto-dismissing). */
  pushToast: (message: string, level: ToastLevel) => void;
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

/** One statically-known bindable action, for the Settings Keybinds catalog. */
export interface BindableAction {
  /** Descriptor id ("file:copy-path") - the [keybinds.actions] key. */
  id: string;
  title: string;
  hint?: string;
  /** Canonical default chord, if the action ships one. */
  defaultChord?: string;
}

export interface ProviderPlugin {
  kinds: string[];
  Preview: ComponentType<PreviewProps> | null;
  handleLaunch?: (result: SearchResult, ctx: LaunchContext) => boolean;
  /** Executable actions for this result (kind-guarded inside - every plugin
   *  sees every result). A descriptor's `shortcut` drives both the chord
   *  dispatch (dispatchShortcut) and the action panel's kbd badge. */
  actions?: (result: SearchResult, ctx: LaunchContext) => ActionDescriptor[];
  /** Static catalog of this provider's rebindable actions (Settings). */
  bindableActions?: BindableAction[];
}

/** The copy chord shared by calc/dict/file providers - Ctrl+C by default,
 *  remappable as the builtin:copy chord family. */
export const isCopyKey = (e: KeyboardEvent): boolean =>
  matchesBuiltin(e, 'builtin:copy');

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

/** All provider-declared actions for a result, in registration order, with
 *  user keybind overrides applied. This is the single chokepoint where
 *  [keybinds.actions] lands, so dispatchShortcut and the action panel's rows,
 *  badges and chord matching all reflect remaps for free. */
export function collectResultActions(
  result: SearchResult | null,
  ctx: LaunchContext,
): ActionDescriptor[] {
  if (!result) return [];
  const out: ActionDescriptor[] = [];
  for (const p of plugins) {
    if (p.actions) out.push(...p.actions(result, ctx));
  }
  return out.map(a =>
    a.displayOnly ? a : { ...a, shortcut: effectiveActionShortcut(a.id, a.shortcut) },
  );
}

/** Every statically-declared provider action, for the Settings catalog. */
export function listProviderActionTargets(): BindableAction[] {
  return plugins.flatMap(p => p.bindableActions ?? []);
}

/** Runs the first provider action whose shortcut matches the chord.
 *  `displayOnly` descriptors are skipped - their keys are handled by bespoke
 *  App.tsx branches and the shortcut exists only for the panel badge. */
export function dispatchShortcut(
  e: KeyboardEvent,
  result: SearchResult | null,
  ctx: LaunchContext,
): boolean {
  for (const a of collectResultActions(result, ctx)) {
    if (a.shortcut && !a.displayOnly && matchesShortcut(e, a.shortcut)) {
      e.preventDefault();
      a.run(ctx);
      return true;
    }
  }
  return false;
}
