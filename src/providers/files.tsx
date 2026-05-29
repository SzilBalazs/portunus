import { invoke } from '@tauri-apps/api/core';
import FilePreview from '../components/FilePreview';
import { registerProvider, type PreviewProps } from './registry';

function FilePreviewWrapper({ result, onLaunch, onReveal, terms }: PreviewProps) {
  return <FilePreview result={result} onLaunch={onLaunch} onReveal={onReveal} terms={terms} />;
}

registerProvider({
  kinds: ['file', 'folder'],
  Preview: FilePreviewWrapper,

  handleKeyDown: (e, result, ctx) => {
    if (e.ctrlKey && !e.altKey && e.key === 'c' && (result?.kind === 'file' || result?.kind === 'folder')) {
      e.preventDefault();
      const path = result.subtitle ? `${result.subtitle}/${result.title}` : result.title;
      navigator.clipboard.writeText(path);
      return true;
    }
    if (e.ctrlKey && !e.altKey && e.key === 'Enter' && result?.kind === 'file') {
      e.preventDefault();
      const parent = result.subtitle ?? '.';
      invoke('launch_app', { exec: `xdg-open "${parent}"`, id: undefined, kind: undefined });
      ctx.setQuery('');
      ctx.setResults([]);
      return true;
    }
    return false;
  },
});
