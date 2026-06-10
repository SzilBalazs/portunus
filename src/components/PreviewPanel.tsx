import { getPreview } from '../providers/registry';
import type { SearchResult } from '../types';
import ExtensionPreview from './ExtensionPreview';

interface Props {
  result: SearchResult | null;
  onLaunch: () => void;
  onReveal?: () => void;
  /** Matched content-search terms to highlight in the preview. */
  terms?: string[];
}

export default function PreviewPanel({ result, onLaunch, onReveal, terms }: Props) {
  // Extension kinds are dynamic (`ext-<name>`), so they route by id prefix
  // instead of the kind registry.
  const Preview = result?.id.startsWith('ext:') ? ExtensionPreview : getPreview(result?.kind);
  if (!Preview || !result) return <div className="preview-empty" />;
  return <Preview key={result.kind} result={result} onLaunch={onLaunch} onReveal={onReveal} terms={terms} />;
}
