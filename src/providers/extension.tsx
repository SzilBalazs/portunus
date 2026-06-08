import { invoke } from '@tauri-apps/api/core';
import { registerProvider } from './registry';

// Launch routing for WASM extension results. They carry no `exec` - Enter goes
// through the extension's own `activate` export (backend hides the window).
// Kinds are dynamic (`ext-<name>`), so matching is by id prefix, not kind.
registerProvider({
  kinds: [],
  Preview: null,
  handleLaunch: (result) => {
    if (!result.id.startsWith('ext:') || !result.ext) return false;
    invoke('extension_activate', { id: result.id, ext: result.ext, action: null })
      .catch(e => console.error(`[extension] activate failed: ${e}`));
    return true;
  },
});
