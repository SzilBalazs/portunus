import { invoke } from '@tauri-apps/api/core';
import FilePreview from '../components/FilePreview';
import { registerProvider, type PreviewProps } from './registry';
import type { ActionDescriptor } from '../actions/types';

function FilePreviewWrapper({ result, onLaunch, onReveal, terms, highlight, quicklook }: PreviewProps) {
  return <FilePreview result={result} onLaunch={onLaunch} onReveal={onReveal} terms={terms} highlight={highlight} quicklook={quicklook} />;
}

registerProvider({
  kinds: ['file', 'folder'],
  Preview: FilePreviewWrapper,

  bindableActions: [
    { id: 'file:copy-path', title: 'Copy Path', hint: 'Files and folders', defaultChord: 'ctrl+c' },
    { id: 'file:reveal', title: 'Reveal in File Manager', hint: 'Files', defaultChord: 'ctrl+enter' },
  ],

  actions: (result, ctx) => {
    if (result.kind !== 'file' && result.kind !== 'folder') return [];
    const path = result.subtitle ? `${result.subtitle}/${result.title}` : result.title;
    const acts: ActionDescriptor[] = [{
      id: 'file:copy-path',
      title: 'Copy Path',
      section: 'result',
      shortcut: { ctrl: true, key: 'c' },
      run: () => { navigator.clipboard.writeText(path); },
    }];
    if (result.kind === 'file') {
      acts.push({
        id: 'file:reveal',
        title: 'Reveal in File Manager',
        section: 'result',
        shortcut: { ctrl: true, key: 'enter' },
        run: () => {
          invoke('reveal_file', { path }).catch(e => console.error('[files] reveal_file failed:', e));
          ctx.setQuery('');
          ctx.setResults([]);
        },
      });
    }
    return acts;
  },
});
