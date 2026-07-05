import { ReactNode } from "react";
import { SearchResult } from "../types";
import { EnterIcon, DeleteIcon } from "../icons";
import { isPreviewable } from "../utils";

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
  /** Whether PDF term-highlighting is on (Ctrl+H); drives the Contents-mode hint. */
  pdfHighlight?: boolean;
  /** The extension action picker is open; it owns the keys. */
  actionPickerOpen?: boolean;
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
  pdfHighlight: boolean,
  actionPickerOpen: boolean,
): ReactNode {
  const k = selected?.kind;

  if (actionPickerOpen) return <>
    <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> choose</span>
    <span className="hint"><kbd><EnterIcon /></kbd> run</span>
    <span className="hint"><kbd>Esc</kbd> back</span>
  </>;
  const isPdf = selected?.title.toLowerCase().endsWith(".pdf") ?? false;
  const Highlight = () => (
    <span className="hint"><kbd>ctrl</kbd><kbd>H</kbd> highlight {pdfHighlight ? "off" : "on"}</span>
  );

  // Full-text "Contents" mode: reuses the file row. Tab flips back to name
  // search and Esc backs out of the mode (not the window) - both kept off the
  // bar to stop the footer overflowing the brand on the right.
  if (contentMode) {
    return <>
      <Nav /><Open />
      {isPdf && <PdfPageNav />}
      {isPdf && <Highlight />}
      {selected && isPreviewable(selected) && <Peek />}
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

  if (k === "ext-error") return <><span className="hint"><kbd><EnterIcon /></kbd> open logs</span><Esc /></>;
  if (k === "content-disabled") return <><span className="hint"><kbd><EnterIcon /></kbd> open settings</span><Esc /></>;
  if (k === "content-hint") return <><span className="hint"><kbd>Tab</kbd> search contents</span><Esc /></>;

  if (k === "file" || k === "folder") {
    return (
      <><Nav /><Open />
        {/* Drop "copy path" for PDFs to make room for the page-nav hint. */}
        {!isPdf && <CopyPath />}
        {k === "file" && <span className="hint"><kbd>ctrl</kbd><kbd><EnterIcon /></kbd> reveal</span>}
        {isPdf && <PdfPageNav />}
        {selected && isPreviewable(selected) && <Peek />}
        {canComplete && <Complete />}<Esc />
      </>
    );
  }
  if (k === "app") return <><Nav /><span className="hint"><kbd><EnterIcon /></kbd> launch</span><Jump />{canComplete && <Complete />}<Esc /></>;

  // Extension result: show its real default-action label, and advertise the
  // picker only when there's more than one action to pick from.
  if (selected?.ext) {
    const actions = selected.ext.actions ?? [];
    const defaultLabel = actions[0]?.label?.toLowerCase() ?? "open";
    return <>
      <Nav />
      <span className="hint"><kbd><EnterIcon /></kbd> {defaultLabel}</span>
      {actions.length > 1 && <span className="hint"><kbd>alt</kbd><kbd><EnterIcon /></kbd> actions</span>}
      <Esc />
    </>;
  }

  // Default: generic result row
  return <><Nav /><Open /><Jump />{canComplete && <Complete />}<Esc /></>;
}

export default function FooterHints({ selected, canComplete, quicklookOpen = false, clipboardMode = false, contentMode = false, smartPaste = false, clipboardIdle = false, pdfHighlight = true, actionPickerOpen = false }: Props) {
  return <div className="hints">{hints(selected, canComplete, quicklookOpen, clipboardMode, contentMode, smartPaste, clipboardIdle, pdfHighlight, actionPickerOpen)}</div>;
}
