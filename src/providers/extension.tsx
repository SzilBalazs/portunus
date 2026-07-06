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
    ctx.activateExtension({
      id: result.id,
      ext: result.ext,
      action: null,
      command: result.ext_command ?? null,
    });
    return true;
  },
});
