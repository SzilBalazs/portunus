import { getPreview } from '../providers/registry';
import type { SearchResult } from '../types';

interface Props {
  result: SearchResult | null;
  onLaunch: () => void;
  onStopTimer: () => void;
  onReveal?: () => void;
  /** Matched content-search terms to highlight in the preview. */
  terms?: string[];
}

export default function PreviewPanel({ result, onLaunch, onStopTimer, onReveal, terms }: Props) {
  const Preview = getPreview(result?.kind);
  if (!Preview || !result) return <div className="preview-empty" />;
  return <Preview key={result.kind} result={result} onLaunch={onLaunch} onStopTimer={onStopTimer} onReveal={onReveal} terms={terms} />;
}
