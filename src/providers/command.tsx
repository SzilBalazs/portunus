import { registerProvider } from './registry';

// Command entries (kind "command") - searchable rows like "Define Word" or
// "Clipboard History" that enter a mode (or run) on Enter. Launch is handled
// directly in App (runCommand). No preview during the current dev stage: the
// selection panel stays empty for command rows (getPreview returns null).

registerProvider({
  kinds: ['command'],
  Preview: null,
});
