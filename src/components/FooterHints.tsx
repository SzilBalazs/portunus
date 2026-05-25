import { SearchResult } from "../types";

interface Props {
  selected: SearchResult | null;
}

export default function FooterHints({ selected }: Props) {
  if (selected?.kind === "timer-item") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd>Del</kbd> stop timer</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "timer-create" && selected.exec) {
    return (
      <div className="hints">
        <span className="hint"><kbd>↵</kbd> start timer</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "timer-expired") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd>↵</kbd> dismiss</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "clipboard" || selected?.kind === "clipboard-image") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd>↵</kbd> paste</span>
        <span className="hint"><kbd>alt</kbd><kbd>1–9</kbd> jump</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "calc") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy value</span>
        <span className="hint"><kbd>alt</kbd><kbd>1–9</kbd> jump</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "file" || selected?.kind === "folder") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd>↵</kbd> open</span>
        <span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy path</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  return (
    <div className="hints">
      <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
      <span className="hint"><kbd>↵</kbd> open</span>
      <span className="hint"><kbd>alt</kbd><kbd>1–9</kbd> jump</span>
      <span className="hint"><kbd>Esc</kbd> close</span>
    </div>
  );
}
