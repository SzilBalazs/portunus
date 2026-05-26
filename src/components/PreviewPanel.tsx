import { SearchResult } from "../types";
import AppPreview from "./AppPreview";
import FilePreview from "./FilePreview";
import ClipboardPreview from "./ClipboardPreview";
import { TimerPreview, TimerCreatePreview, TimerExpiredPreview } from "./TimerPreviews";

interface Props {
  result: SearchResult | null;
  onLaunch: () => void;
  onStopTimer: () => void;
}

export default function PreviewPanel({ result, onLaunch, onStopTimer }: Props) {
  if (result?.kind === "app") {
    return <AppPreview result={result} onLaunch={onLaunch} />;
  }
  if (result?.kind === "file" || result?.kind === "folder") {
    return <FilePreview result={result} />;
  }
  if (result?.kind === "timer-item") {
    return <TimerPreview key={result.id} result={result} onStop={onStopTimer} />;
  }
  if (result?.kind === "timer-create") {
    return <TimerCreatePreview result={result} onStart={onLaunch} />;
  }
  if (result?.kind === "timer-expired") {
    return <TimerExpiredPreview label={result.title} onDismiss={onLaunch} />;
  }
  if (result?.kind === "clipboard" || result?.kind === "clipboard-image") {
    return <ClipboardPreview result={result} onPaste={onLaunch} />;
  }
  return <div className="preview-empty" />;
}
