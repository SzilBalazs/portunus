import { ReactNode } from "react";
import { SearchResult } from "../types";
import { EnterIcon } from "../icons";

interface Props {
  selected: SearchResult | null;
  /** A ghost command-completion is showing, so Tab will accept it. */
  canComplete: boolean;
}

// Reusable hint atoms
const Nav = () => <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>;
const Esc = () => <span className="hint"><kbd>Esc</kbd> close</span>;
const Open = () => <span className="hint"><kbd><EnterIcon /></kbd> open</span>;
const Jump = () => <span className="hint"><kbd>alt</kbd><kbd>1–9</kbd> jump</span>;
const CopyPath = () => <span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy path</span>;
const PdfPageNav = () => <span className="hint"><kbd>ctrl</kbd><kbd>←→</kbd> page</span>;

const Complete = () => <span className="hint"><kbd>Tab</kbd> complete</span>;

function hints(selected: SearchResult | null, canComplete: boolean): ReactNode {
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

  if (k === "content-disabled") return <><span className="hint"><kbd><EnterIcon /></kbd> open settings</span><Esc /></>;
  if (k === "content-hint") return <><span className="hint"><kbd><EnterIcon /></kbd> search contents</span><Esc /></>;

  if (k === "file" || k === "folder") {
    const isPdf = selected?.title.toLowerCase().endsWith(".pdf") ?? false;
    return (
      <><Nav /><Open />
        {/* Drop "copy path" for PDFs to make room for the page-nav hint. */}
        {!isPdf && <CopyPath />}
        {k === "file" && <span className="hint"><kbd>ctrl</kbd><kbd><EnterIcon /></kbd> reveal</span>}
        {isPdf && <PdfPageNav />}
        {canComplete && <Complete />}<Esc />
      </>
    );
  }
  if (k === "app") return <><Nav /><span className="hint"><kbd><EnterIcon /></kbd> launch</span><Jump />{canComplete && <Complete />}<Esc /></>;

  // Default: generic result row
  return <><Nav /><Open /><Jump />{canComplete && <Complete />}<Esc /></>;
}

export default function FooterHints({ selected, canComplete }: Props) {
  return <div className="hints">{hints(selected, canComplete)}</div>;
}
