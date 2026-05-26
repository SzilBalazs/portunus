import { invoke } from '@tauri-apps/api/core';
import ClipboardPreview from '../components/ClipboardPreview';
import { registerProvider, type PreviewProps } from './registry';

function ClipboardPreviewWrapper({ result, onLaunch }: PreviewProps) {
  return <ClipboardPreview result={result} onPaste={onLaunch} />;
}

registerProvider({
  kinds: ['clipboard', 'clipboard-image'],
  Preview: ClipboardPreviewWrapper,

  handleLaunch: (result, ctx) => {
    if (result.exec?.startsWith('clipboard:copy:')) {
      invoke('paste_clipboard', { id: result.exec.slice('clipboard:copy:'.length) });
      ctx.setQuery('');
      ctx.setResults([]);
      return true;
    }
    return false;
  },
});
