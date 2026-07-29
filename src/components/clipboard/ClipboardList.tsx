import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import type { ClipboardEntry } from "../../types";
import { getDecoded, peekDecoded } from "./clipboardCache";
import { TextLinesIcon, JsonIcon, LinkIcon, ImageGlyphIcon } from "./clipIcons";

const RENDER_CAP = 150;

interface Props {
  entries: ClipboardEntry[];
  selectedIndex: number;
  deleting: Set<string>;
  onSelect: (i: number) => void;
  onActivate: (i: number) => void;
}

/** Lazily-decoded 28px thumbnail for image rows. Decodes only when scrolled into
 *  view; the decode is shared (via clipboardCache) with the large preview, so
 *  hovering the list pre-warms previews. */
function ClipThumb({ id }: { id: string }) {
  const [url, setUrl] = useState<string | null>(() => {
    const d = peekDecoded(id);
    return d?.kind === "image" ? d.url : null;
  });
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (url) return;
    const el = ref.current;
    if (!el) return;
    let cancelled = false;
    const io = new IntersectionObserver((ents) => {
      if (ents.some((e) => e.isIntersecting)) {
        io.disconnect();
        getDecoded(id)
          .then((d) => { if (!cancelled && d.kind === "image") setUrl(d.url); })
          .catch(() => {});
      }
    }, { rootMargin: "200px 0px" });
    io.observe(el);
    return () => { cancelled = true; io.disconnect(); };
  }, [id, url]);

  return (
    <div className="clip-row-icon clip-thumb" ref={ref}>
      {url
        ? <img src={url} alt="" className="clip-thumb-img" draggable={false} />
        : <ImageGlyphIcon />}
    </div>
  );
}

function rowIcon(e: ClipboardEntry) {
  if (e.kind === "image") return <ClipThumb id={e.id} />;
  if (e.content_type === "color") {
    return (
      <div className="clip-row-icon clip-swatch" style={{ background: accentColor(e) ?? "#000" }} />
    );
  }
  const glyph =
    e.content_type === "url" ? <LinkIcon /> :
    e.content_type === "json" ? <JsonIcon /> :
    <TextLinesIcon />;
  return <div className="clip-row-icon clip-glyph">{glyph}</div>;
}

function rowSnippet(e: ClipboardEntry): string {
  if (e.kind === "image") {
    const dims = e.dimensions ? `${e.dimensions[0]}×${e.dimensions[1]}` : "";
    const fmt = e.format ? e.format.toUpperCase() : "Image";
    return [fmt, dims].filter(Boolean).join(" · ");
  }
  return e.preview;
}

/** Colour that may be substituted into the selection tint. The value is
 *  clipboard content, so keep it to the shapes the backend detector emits
 *  (`#hex`, `rgb()`, `rgba()`) — anything else stays untinted rather than
 *  becoming an arbitrary token stream inside a CSS custom property. */
const CSS_COLOR = /^(#[0-9a-f]{3,8}|rgba?\([0-9,.%\s]+\))$/i;

function accentColor(e: ClipboardEntry | undefined): string | undefined {
  if (!e || e.content_type !== "color" || !e.color) return undefined;
  return CSS_COLOR.test(e.color.trim()) ? e.color.trim() : undefined;
}

export default function ClipboardList({ entries, selectedIndex, deleting, onSelect, onActivate }: Props) {
  const colRef = useRef<HTMLDivElement>(null);
  const selectedRef = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<{ top: number; height: number; snap: boolean } | null>(null);

  const shown = entries.slice(0, RENDER_CAP);
  const selectedColor = accentColor(entries[selectedIndex]);

  useLayoutEffect(() => {
    const el = selectedRef.current;
    const col = colRef.current;
    if (!el || !col) { setIndicator(null); return; }
    const before = col.scrollTop;
    const top = el.offsetTop;
    const bottom = top + el.offsetHeight;
    if (top < col.scrollTop) col.scrollTop = top;
    else if (bottom > col.scrollTop + col.clientHeight) col.scrollTop = bottom - col.clientHeight;
    const scrolled = col.scrollTop !== before;
    setIndicator({ top, height: el.offsetHeight, snap: scrolled });
  }, [selectedIndex, entries]);

  return (
    <div className="clip-list" ref={colRef} role="listbox">
      {indicator && shown.length > 0 && (
        <div
          className={`clip-selection-bg${selectedColor ? " has-accent" : ""}`}
          aria-hidden="true"
          style={{
            transform: `translateY(${indicator.top}px)`,
            height: indicator.height,
            "--row-accent": selectedColor,
            transition: indicator.snap ? "none" : undefined,
          } as CSSProperties}
        />
      )}
      {shown.map((e, i) => (
        <div
          key={e.id}
          ref={i === selectedIndex ? selectedRef : undefined}
          className={`clip-row${i === selectedIndex ? " selected" : ""}${deleting.has(e.id) ? " deleting" : ""}`}
          style={{ "--row-i": Math.min(i, 12) } as CSSProperties}
          role="option"
          aria-selected={i === selectedIndex}
          onClick={() => onSelect(i)}
          onDoubleClick={() => onActivate(i)}
        >
          {rowIcon(e)}
          <div className="clip-row-snippet">{rowSnippet(e)}</div>
          <div className="clip-row-shortcut" style={i < 9 ? undefined : { visibility: "hidden" }}>
            {i < 9 ? i + 1 : ""}
          </div>
        </div>
      ))}
      {entries.length > RENDER_CAP && (
        <div className="clip-list-more">Showing first {RENDER_CAP} · type to search older entries</div>
      )}
    </div>
  );
}
