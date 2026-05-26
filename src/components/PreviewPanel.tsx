import { getPreview } from '../providers/registry';
import type { SearchResult } from '../types';

interface Props {
  result: SearchResult | null;
  onLaunch: () => void;
  onStopTimer: () => void;
}

export default function PreviewPanel({ result, onLaunch, onStopTimer }: Props) {
  const Preview = getPreview(result?.kind);
  if (!Preview || !result) return <div className="preview-empty" />;
  return <Preview key={result.id} result={result} onLaunch={onLaunch} onStopTimer={onStopTimer} />;
}
