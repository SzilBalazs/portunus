import { ReactNode } from "react";
import { SearchResult } from "../types";
import { EnterIcon, DeleteIcon } from "../icons";

interface Props {
  selected: SearchResult | null;
  /** A ghost command-completion is showing, so Tab will accept it. */
  canComplete: boolean;
  /** The Quicklook overlay is open, so Esc closes it and Shift+Enter dismisses. */
  quicklookOpen?: boolean;
  /** In the dedicated clipboard browser; hints + paste-vs-copy wording change. */
  clipboardMode?: boolean;
  /** In the Tab-activated full-text "Contents" mode. */
  contentMode?: boolean;
  /** wtype is available, so Enter pastes into the focused window vs copy-only. */
  smartPaste?: boolean;
  /** The clipboard list is unfiltered + unsearched (idle), so show the Tab hint. */
  clipboardIdle?: boolean;
}

// Reusable hint atoms
const Nav = () => <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>;
const Esc = () => <span className="hint"><kbd>Esc</kbd> close</span>;
const Open = () => <span className="hint"><kbd><EnterIcon /></kbd> open</span>;
const Jump = () => <span className="hint"><kbd>alt</kbd><kbd>1-9</kbd> jump</span>;
const CopyPath = () => <span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy path</span>;
const PdfPageNav = () => <span className="hint"><kbd>ctrl</kbd><kbd>←→</kbd> page</span>;
const Peek = () => <span className="hint"><kbd>shift</kbd><kbd><EnterIcon /></kbd> peek</span>;

const Complete = () => <span className="hint"><kbd>Tab</kbd> complete</span>;

function hints(
  selected: SearchResult | null,
  canComplete: boolean,
  quicklookOpen: boolean,
  clipboardMode: boolean,
  contentMode: boolean,
  smartPaste: boolean,
  clipboardIdle: boolean,
): ReactNode {
  const k = selected?.kind;

  // Full-text "Contents" mode: reuses the file row, but Tab flips back to name
  // search and Esc backs out of the mode (not the window).
  if (contentMode) {
    const isPdf = selected?.title.toLowerCase().endsWith(".pdf") ?? false;
    return <>
      <Nav /><Open />
      {isPdf && <PdfPageNav />}
      <Peek />
      <span className="hint"><kbd>Tab</kbd> names</span>
      <span className="hint"><kbd>Esc</kbd> back</span>
    </>;
  }

  // Dedicated clipboard browser. Enter degrades to copy-and-close without wtype,
  // so the bar must say "copy" not "paste" (and drop the redundant ctrl+enter).
  if (clipboardMode) return <>
    <Nav />
    {smartPaste
      ? <><span className="hint"><kbd><EnterIcon /></kbd> paste</span><span className="hint"><kbd>ctrl</kbd><kbd><EnterIcon /></kbd> copy</span></>
      : <span className="hint"><kbd><EnterIcon /></kbd> copy</span>}
    <span className="hint"><kbd>shift</kbd><kbd><DeleteIcon /></kbd> delete</span>
    {clipboardIdle && <span className="hint"><kbd>Tab</kbd> filter</span>}
    <span className="hint"><kbd>Esc</kbd> back</span>
  </>;

  // While Quicklook is open the keys mean something different - keep the bar honest.
  if (quicklookOpen) return <>
    <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> scroll</span>
    <Open />
    <span className="hint"><kbd>shift</kbd><kbd><EnterIcon /></kbd> / <kbd>Esc</kbd> close</span>
  </>;

  if (k === "clipboard-mode") return <><Nav /><Open /><Esc /></>;

  if (k === "calc") return <><Nav /><span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy value</span><Esc /></>;

  if (k === "dict-hint") return <><span className="hint"><kbd>|</kbd> start typing</span><Esc /></>;
  if (k === "dict") return <><Nav /><span className="hint"><kbd>ctrl</kbd><kbd>C</kbd> copy definition</span><Esc /></>;

  if (k === "content-disabled") return <><span className="hint"><kbd><EnterIcon /></kbd> open settings</span><Esc /></>;
  if (k === "content-hint") return <><span className="hint"><kbd>Tab</kbd> search contents</span><Esc /></>;

  if (k === "file" || k === "folder") {
    const isPdf = selected?.title.toLowerCase().endsWith(".pdf") ?? false;
    return (
      <><Nav /><Open />
        {/* Drop "copy path" for PDFs to make room for the page-nav hint. */}
        {!isPdf && <CopyPath />}
        {k === "file" && <span className="hint"><kbd>ctrl</kbd><kbd><EnterIcon /></kbd> reveal</span>}
        {isPdf && <PdfPageNav />}
        <Peek />
        {canComplete && <Complete />}<Esc />
      </>
    );
  }
  if (k === "app") return <><Nav /><span className="hint"><kbd><EnterIcon /></kbd> launch</span><Jump />{canComplete && <Complete />}<Esc /></>;

  // Default: generic result row
  return <><Nav /><Open /><Jump />{canComplete && <Complete />}<Esc /></>;
}

export default function FooterHints({ selected, canComplete, quicklookOpen = false, clipboardMode = false, contentMode = false, smartPaste = false, clipboardIdle = false }: Props) {
  return <div className="hints">{hints(selected, canComplete, quicklookOpen, clipboardMode, contentMode, smartPaste, clipboardIdle)}</div>;
}
