import { ReactNode } from "react";
import { SearchResult } from "../types";
import { EnterIcon } from "../icons";

interface Props {
  selected: SearchResult | null;
  isContentSearch: boolean;
}

// Reusable hint atoms
const Nav = () => <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>;
const Esc = () => <span className="hint"><kbd>Esc</kbd> close</span>;
const Open = () => <span className="hint"><kbd><EnterIcon /></kbd> open</span>;
const Jump = () => <span className="hint"><kbd>alt</kbd><kbd>1–9</kbd> jump</span>;
const CopyPath = () => <span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy path</span>;

const TabHint = ({ isContentSearch }: { isContentSearch: boolean }) => (
  <span className="hint"><kbd>Tab</kbd>{isContentSearch ? " back" : " search contents"}</span>
);

function hints(selected: SearchResult | null, isContentSearch: boolean): ReactNode {
  const k = selected?.kind;

  if (k === "timer-item") return <><Nav /><span className="hint"><kbd>Del</kbd> stop timer</span><Esc /></>;
  if (k === "timer-create" && selected?.exec) return <><span className="hint"><kbd><EnterIcon /></kbd> start timer</span><Esc /></>;
  if (k === "timer-create") return <><span className="hint">type a duration</span><span className="hint"><kbd>5m</kbd> <kbd>1h30m</kbd> <kbd>30s</kbd></span><Esc /></>;
  if (k === "timer-hint") return <><span className="hint"><kbd>|</kbd> start typing</span><Esc /></>;
  if (k === "timer-expired") return <><Nav /><span className="hint"><kbd><EnterIcon /></kbd> dismiss</span><Esc /></>;

  if (k === "clipboard" || k === "clipboard-image") return <><Nav /><span className="hint"><kbd><EnterIcon /></kbd> paste</span><Jump /><Esc /></>;

  if (k === "calc") return <><Nav /><span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy value</span><Esc /></>;

  if (k === "dict-hint") return <><span className="hint"><kbd>|</kbd> start typing</span><Esc /></>;
  if (k === "dict") return <><Nav /><span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy definition</span><Esc /></>;

  if (k === "content-disabled") return <><span className="hint"><kbd><EnterIcon /></kbd> open settings</span><span className="hint"><kbd>Tab</kbd> back</span><Esc /></>;
  if (k === "content-hint") return <><span className="hint"><kbd>Tab</kbd> or <kbd><EnterIcon /></kbd> search contents</span><Esc /></>;

  if (k === "file" || k === "folder") return (
    <><Nav /><Open /><CopyPath />
      {k === "file" && <span className="hint"><kbd>ctrl</kbd><kbd><EnterIcon /></kbd> reveal</span>}
      <TabHint isContentSearch={isContentSearch} /><Esc />
    </>
  );
  if (k === "app") return <><Nav /><span className="hint"><kbd><EnterIcon /></kbd> launch</span><Jump /><TabHint isContentSearch={isContentSearch} /><Esc /></>;

  // Default: generic result row
  return <><Nav /><Open /><Jump /><TabHint isContentSearch={isContentSearch} /><Esc /></>;
}

export default function FooterHints({ selected, isContentSearch }: Props) {
  return <div className="hints">{hints(selected, isContentSearch)}</div>;
}
