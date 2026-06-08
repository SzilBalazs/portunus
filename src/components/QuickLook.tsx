import { useEffect, useRef } from 'react';
import { getPreview } from '../providers/registry';
import type { SearchResult } from '../types';

interface Props {
  result: SearchResult;
  onLaunch: () => void;
  onClose: () => void;
  /** Matched content-search terms to highlight in the preview. */
  terms?: string[];
}

// Scrollable viewport inside a preview, by kind: PDF reader, text/code/markdown/csv,
// or folder listing. Image previews fit-to-view and have nothing to scroll.
const VIEWPORT_SELECTOR = '.pdf-ql, .text-preview-wrap, .folder-contents';

/**
 * Full-card overlay that re-renders the selected result's preview at large size.
 * Reuses the same `getPreview(kind)` component the side panel uses - preview
 * renderers are flex-sized, so a bigger container just makes them bigger.
 *
 * Quicklook is modal: App.tsx suppresses result navigation while it's open, and
 * this component maps Arrow/Page/Home/End/Space to scrolling the preview. App
 * owns Esc / Shift+Enter (close/toggle); Ctrl combos (PDF page-flip, zoom) are
 * left to PdfPreview.
 */
export default function QuickLook({ result, onLaunch, onClose, terms }: Props) {
  const Preview = getPreview(result.kind);
  const innerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Don't hijack Ctrl/Meta/Alt combos - those drive PDF page-flip & zoom.
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const vp = innerRef.current?.querySelector<HTMLElement>(VIEWPORT_SELECTOR);
      if (!vp) return;
      const line = 80;
      const pageStep = vp.clientHeight * 0.9;
      switch (e.key) {
        case 'ArrowDown': vp.scrollTop += line; break;
        case 'ArrowUp':   vp.scrollTop -= line; break;
        case 'PageDown':  vp.scrollTop += pageStep; break;
        case 'PageUp':    vp.scrollTop -= pageStep; break;
        case ' ':         vp.scrollTop += e.shiftKey ? -pageStep : pageStep; break;
        case 'Home':      vp.scrollTop = 0; break;
        case 'End':       vp.scrollTop = vp.scrollHeight; break;
        default: return;
      }
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, []);

  if (!Preview) return null;
  return (
    <div
      className="quicklook-overlay"
      onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div className="quicklook-inner" ref={innerRef}>
        <Preview key={result.kind} result={result} onLaunch={onLaunch} onReveal={onClose} terms={terms} quicklook />
      </div>
    </div>
  );
}
