import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ClipboardEntry, ClipboardCapabilities } from "../../types";
import ClipboardList from "./ClipboardList";
import ClipboardEntryPreview from "./ClipboardEntryPreview";
import { getDecoded, evictDecoded } from "./clipboardCache";
import { ClipboardGlyphIcon } from "./clipIcons";

type Filter = "all" | "text" | "image" | "url" | "color";

const TABS: { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "text", label: "Text" },
  { key: "image", label: "Images" },
  { key: "url", label: "Links" },
  { key: "color", label: "Colors" },
];

interface Props {
  /** Filter text from the shared search input. */
  query: string;
  capabilities: ClipboardCapabilities;
  /** Esc with an empty query: leave the browser (back to launcher / hide window). */
  onExit: () => void;
  /** Esc with a non-empty query: clear it (handled by the parent's input state). */
  onClearQuery: () => void;
  /** After paste/copy the launcher dismisses: clear query + exit mode. */
  onPasted: () => void;
}

function animationsOff(): boolean {
  return (
    document.documentElement.dataset.animateResults === "off" ||
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

export default function ClipboardMode({ query, capabilities, onExit, onClearQuery, onPasted }: Props) {
  const [entries, setEntries] = useState<ClipboardEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [deleting, setDeleting] = useState<Set<string>>(new Set());

  // Tab indicator geometry (sliding pill behind the active tab).
  const tabsRef = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<{ left: number; width: number } | null>(null);

  // Refs for the keydown handler so it reads live values without re-subscribing.
  const stateRef = useRef({ filter, selectedIndex, query, entries, deleting });

  const load = () => {
    invoke<ClipboardEntry[]>("clipboard_list", { limit: 250 })
      .then((e) => { setEntries(e); setError(null); })
      .catch((err) => { setEntries([]); setError(String(err)); });
  };

  useEffect(() => { load(); }, []);

  // Kick off the background OCR pass for copied images (cheap + coalesced on the
  // backend; cached entries are skipped). Reload when it finishes so freshly
  // OCR'd `ocr_text` becomes searchable.
  useEffect(() => {
    invoke("index_clipboard_ocr").catch(() => {});
    let unlisten: (() => void) | undefined;
    listen("clipboard-ocr-done", () => load()).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  const counts = useMemo(() => {
    const c: Record<Filter, number> = { all: 0, text: 0, image: 0, url: 0, color: 0 };
    for (const e of entries ?? []) {
      c.all++;
      if (e.kind === "image") c.image++; else c.text++;
      if (e.content_type === "url") c.url++;
      if (e.content_type === "color") c.color++;
    }
    return c;
  }, [entries]);

  const visible = useMemo(() => {
    const list = entries ?? [];
    const q = query.trim().toLowerCase();
    return list.filter((e) => {
      if (filter === "image" && e.kind !== "image") return false;
      if (filter === "text" && e.kind === "image") return false;
      if (filter === "url" && e.content_type !== "url") return false;
      if (filter === "color" && e.content_type !== "color") return false;
      if (q && !`${e.preview} ${e.ocr_text ?? ""}`.toLowerCase().includes(q)) return false;
      return true;
    });
  }, [entries, filter, query]);

  // Reset selection on query/filter change; clamp when the list shrinks.
  useEffect(() => { setSelectedIndex(0); }, [query, filter]);
  useEffect(() => {
    setSelectedIndex((i) => (i >= visible.length ? Math.max(visible.length - 1, 0) : i));
  }, [visible.length]);

  useEffect(() => {
    stateRef.current = { filter, selectedIndex, query, entries, deleting };
  }, [filter, selectedIndex, query, entries, deleting]);

  const selected = visible[selectedIndex] ?? null;

  // Idle prefetch of neighbours so sequential arrowing feels instant (and the
  // metadata footer is ready). Cache-deduped; fire-and-forget.
  useEffect(() => {
    if (!selected) return;
    const t = setTimeout(() => {
      for (const d of [-2, -1, 1, 2]) {
        const n = visible[selectedIndex + d];
        if (n) getDecoded(n.id).catch(() => {});
      }
    }, 150);
    return () => clearTimeout(t);
  }, [selected, selectedIndex, visible]);

  // Sliding tab indicator.
  useLayoutEffect(() => {
    const wrap = tabsRef.current;
    if (!wrap) return;
    const active = wrap.querySelector<HTMLElement>(".clip-tab.active");
    if (!active) { setIndicator(null); return; }
    setIndicator({ left: active.offsetLeft, width: active.offsetWidth });
  }, [filter, counts]);

  // The keydown handler is mounted once, so action helpers must read the live
  // filtered list from stateRef rather than the render-time `visible` closure.
  const liveList = (): ClipboardEntry[] => {
    const st = stateRef.current;
    return (st.entries ?? []).filter((e) => filterMatch(e, st.filter, st.query));
  };

  const paste = (idx: number, copyOnly: boolean) => {
    const e = liveList()[idx];
    if (!e) return;
    invoke("paste_clipboard", { id: e.id, copyOnly });
    onPasted();
  };

  const openUrl = (idx: number) => {
    const e = liveList()[idx];
    if (!e) return;
    getDecoded(e.id).then((d) => {
      if (d.kind === "text") {
        invoke("launch_app", { exec: `xdg-open "${d.text.trim()}"` });
        onPasted();
      }
    });
  };

  const remove = (idx: number) => {
    const e = liveList()[idx];
    if (!e || stateRef.current.deleting.has(e.id)) return;
    const id = e.id;
    invoke("clipboard_delete", { id }).catch(() => { load(); });
    evictDecoded(id);
    const commit = () => setEntries((prev) => (prev ? prev.filter((x) => x.id !== id) : prev));
    if (animationsOff()) { commit(); return; }
    setDeleting((prev) => new Set(prev).add(id));
    setTimeout(() => {
      commit();
      setDeleting((prev) => { const n = new Set(prev); n.delete(id); return n; });
    }, 150);
  };

  // Keyboard. App's global handler early-returns while in clipboard mode, so this
  // owns all keys here.
  useEffect(() => {
    // WebKitGTK reports Shift+Tab as e.key === "ISO_Left_Tab" with e.shiftKey
    // stripped, so neither e.key === "Tab" nor e.shiftKey fires. Match the
    // physical e.code === "Tab" and track Shift separately for direction.
    let shiftDown = false;
    const onKeyUp = (e: KeyboardEvent) => { if (e.key === "Shift") shiftDown = false; };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Shift") shiftDown = true;
      if (!document.hasFocus()) return;
      const st = stateRef.current;
      const list = (st.entries ?? []).filter((x) => filterMatch(x, st.filter, st.query));
      const max = Math.max(list.length - 1, 0);

      if (e.key === "ArrowDown") { e.preventDefault(); setSelectedIndex((i) => Math.min(i + 1, max)); }
      else if (e.key === "ArrowUp") { e.preventDefault(); setSelectedIndex((i) => Math.max(i - 1, 0)); }
      else if (e.key === "PageDown") { e.preventDefault(); setSelectedIndex((i) => Math.min(i + 10, max)); }
      else if (e.key === "PageUp") { e.preventDefault(); setSelectedIndex((i) => Math.max(i - 10, 0)); }
      // Home/End intentionally NOT handled: they belong to the search input's caret.
      else if (e.key === "Enter") {
        e.preventDefault();
        paste(st.selectedIndex, e.ctrlKey);
      }
      else if (e.ctrlKey && (e.key === "o" || e.key === "O")) { e.preventDefault(); openUrl(st.selectedIndex); }
      else if (e.shiftKey && (e.key === "Delete" || e.key === "Backspace")) { e.preventDefault(); remove(st.selectedIndex); }
      else if (e.key === "Tab" || e.key === "ISO_Left_Tab" || e.code === "Tab") {
        e.preventDefault();
        const back = e.shiftKey || shiftDown || e.key === "ISO_Left_Tab";
        const cur = TABS.findIndex((t) => t.key === st.filter);
        const next = (cur + (back ? TABS.length - 1 : 1)) % TABS.length;
        setFilter(TABS[next].key);
      }
      else if (e.ctrlKey && e.key >= "1" && e.key <= "5") {
        e.preventDefault();
        setFilter(TABS[parseInt(e.key) - 1].key);
      }
      else if (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const n = parseInt(e.key) - 1;
        if (list[n]) paste(n, false);
      }
      else if (e.key === "Escape") {
        e.preventDefault();
        if (st.query.trim()) onClearQuery();
        else onExit();
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKeyUp);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const tabs = (
    <div className="clip-tabs" ref={tabsRef}>
      {indicator && <div className="clip-tab-indicator" style={{ transform: `translateX(${indicator.left}px)`, width: indicator.width } as CSSProperties} />}
      {TABS.map((t) => (
        <button
          key={t.key}
          className={`clip-tab${filter === t.key ? " active" : ""}`}
          onClick={() => setFilter(t.key)}
          tabIndex={-1}
        >
          {t.label}
          {counts[t.key] > 0 && <span className="clip-tab-count">{counts[t.key]}</span>}
        </button>
      ))}
    </div>
  );

  // ── content ──
  let content: ReactNode;
  if (error) {
    content = (
      <div className="clip-empty clip-empty-full">
        <div className="clip-empty-icon"><ClipboardGlyphIcon size={24} /></div>
        <div className="clip-empty-title">Clipboard history needs cliphist</div>
        <div className="clip-empty-sub">Portunus uses cliphist + wl-clipboard to store history.</div>
        <code className="app-preview-exec">yay -S cliphist wl-clipboard</code>
      </div>
    );
  } else if (entries === null) {
    content = <div className="clip-content" />;
  } else if (entries.length === 0) {
    content = (
      <div className="clip-empty clip-empty-full">
        <div className="clip-empty-icon"><ClipboardGlyphIcon size={24} /></div>
        <div className="clip-empty-title">Clipboard history is empty</div>
        <div className="clip-empty-sub">Copy something and it will appear here.</div>
      </div>
    );
  } else {
    content = (
      <div className="clip-content">
        {visible.length === 0 ? (
          <div className="clip-list">
            <div className="clip-empty">
              <div className="clip-empty-title">No matches</div>
              <div className="clip-empty-sub">
                {query.trim() ? `Nothing matches "${query.trim()}"` : "Nothing in this filter"}
              </div>
            </div>
          </div>
        ) : (
          <ClipboardList
            entries={visible}
            selectedIndex={selectedIndex}
            deleting={deleting}
            onSelect={setSelectedIndex}
            onActivate={(i) => paste(i, false)}
          />
        )}
        <div className="clip-preview-col">
          {selected
            ? <ClipboardEntryPreview
                entry={selected}
                smartPaste={capabilities.smart_paste}
                onPaste={() => paste(selectedIndex, false)}
                onCopy={() => paste(selectedIndex, true)}
                onOpenUrl={() => openUrl(selectedIndex)}
              />
            : <div className="preview-empty" />}
        </div>
      </div>
    );
  }

  return (
    <div className="clip-mode">
      {tabs}
      {content}
    </div>
  );
}

function filterMatch(e: ClipboardEntry, filter: Filter, query: string): boolean {
  if (filter === "image" && e.kind !== "image") return false;
  if (filter === "text" && e.kind === "image") return false;
  if (filter === "url" && e.content_type !== "url") return false;
  if (filter === "color" && e.content_type !== "color") return false;
  const q = query.trim().toLowerCase();
  if (q && !`${e.preview} ${e.ocr_text ?? ""}`.toLowerCase().includes(q)) return false;
  return true;
}
