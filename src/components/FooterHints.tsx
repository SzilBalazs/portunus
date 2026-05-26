import { SearchResult } from "../types";
import { EnterIcon } from "../icons";

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
        <span className="hint"><kbd><EnterIcon /></kbd> start timer</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "timer-create" && !selected.exec) {
    return (
      <div className="hints">
        <span className="hint">type a duration</span>
        <span className="hint"><kbd>5m</kbd> <kbd>1h30m</kbd> <kbd>30s</kbd></span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "timer-hint") {
    return (
      <div className="hints">
        <span className="hint"><kbd>|</kbd> start typing</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "timer-expired") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd><EnterIcon /></kbd> dismiss</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "clipboard" || selected?.kind === "clipboard-image") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd><EnterIcon /></kbd> paste</span>
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
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "dict-hint") {
    return (
      <div className="hints">
        <span className="hint"><kbd>|</kbd> start typing</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "dict") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy definition</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "file" || selected?.kind === "folder") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd><EnterIcon /></kbd> open</span>
        <span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy path</span>
        {selected.kind === "file" && <span className="hint"><kbd>ctrl</kbd><kbd><EnterIcon /></kbd> reveal</span>}
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  if (selected?.kind === "app") {
    return (
      <div className="hints">
        <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span className="hint"><kbd><EnterIcon /></kbd> launch</span>
        <span className="hint"><kbd>alt</kbd><kbd>1–9</kbd> jump</span>
        <span className="hint"><kbd>Esc</kbd> close</span>
      </div>
    );
  }
  return (
    <div className="hints">
      <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
      <span className="hint"><kbd><EnterIcon /></kbd> open</span>
      <span className="hint"><kbd>alt</kbd><kbd>1–9</kbd> jump</span>
      <span className="hint"><kbd>Esc</kbd> close</span>
    </div>
  );
}
