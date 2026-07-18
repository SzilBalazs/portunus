import { registerProvider } from './registry';
import type { ExtensionResult } from '../types';
import { parseChord } from '../keybinds/chord';

// Launch routing for WASM extension results. They carry no `exec` - Enter goes
// through the extension's own `activate` export via the shared activation
// flow (optimistic hide, forms, toasts). Kinds are dynamic (`ext-<name>`),
// so matching is by id prefix, not kind.
registerProvider({
  kinds: [],
  Preview: null,
  handleLaunch: (result, ctx) => {
    if (!result.id.startsWith('ext:') || !result.ext) return false;
    // Enter runs the default (first) action - honor its opens_form hint so
    // the launcher stays visible while the extension builds the form.
    ctx.activateExtension({
      id: result.id,
      ext: result.ext,
      action: null,
      command: result.ext_command ?? null,
      opensForm: result.ext.actions?.[0]?.opens_form === true,
    });
    return true;
  },

  // The extension's declared actions, default (first) action first. The Enter
  // shortcut on the default is badge-only: the plain-Enter launch fallback
  // (handleLaunch above) already runs it. Non-default actions carry the
  // extension's shipped chord (host-validated), keyed `ext:<name>:<action>`
  // so [keybinds.actions] overrides scope per extension.
  actions: (result, ctx) => {
    if (!result.id.startsWith('ext:') || !result.ext) return [];
    const ext: ExtensionResult = result.ext;
    const extName = result.id.split(':')[1];
    return (ext.actions ?? []).map((a, i) => ({
      id: `ext:${extName}:${a.id}`,
      title: a.label,
      hint: a.hint,
      section: 'result' as const,
      shortcut: i === 0
        ? { key: 'enter' }
        : a.shortcut ? parseChord(a.shortcut) ?? undefined : undefined,
      displayOnly: i === 0,
      run: () => ctx.activateExtension({
        id: result.id,
        ext,
        action: a.id,
        command: result.ext_command ?? null,
        opensForm: a.opens_form === true,
      }),
    }));
  },
});
