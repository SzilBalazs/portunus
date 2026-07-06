import { registerProvider } from './registry';

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
});
