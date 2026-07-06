import { useState, useContext, type ReactElement } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { BookIcon, CategoryGlyph, ClipboardIcon, FileGlyphIcon, SearchIcon } from "../icons";
import { fileCategory } from "../utils";
import { ColoredIconsContext } from "../coloredIcons";

interface Props {
  icon_path?: string;
  iconDataUri?: string;
  /** Named glyph for a built-in command entry (theme-aware inline SVG). */
  glyph?: string;
  title: string;
  kind: string;
}

const COMMAND_GLYPHS: Record<string, () => ReactElement> = {
  book: BookIcon,
  clipboard: ClipboardIcon,
  search: SearchIcon,
};

export default function ResultIcon({ icon_path, iconDataUri, glyph, title, kind }: Props) {
  const [failed, setFailed] = useState(false);
  const coloredIcons = useContext(ColoredIconsContext);

  // Extension-supplied icon (host-validated data: URI); on error falls
  // through to the kind glyph below.
  if (iconDataUri && !failed) {
    return (
      <img
        className="result-icon-img"
        src={iconDataUri}
        alt=""
        onError={() => setFailed(true)}
      />
    );
  }

  if (icon_path && !failed) {
    return (
      <img
        className="result-icon-img"
        src={convertFileSrc(icon_path)}
        alt=""
        onError={() => setFailed(true)}
      />
    );
  }

  if (kind === "ext-error") {
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      </div>
    );
  }

  // Command entries: a named glyph when the descriptor supplies one (built-in
  // commands), else the generic chevron-prompt glyph.
  if (kind === "command") {
    const Glyph = glyph ? COMMAND_GLYPHS[glyph] : undefined;
    if (Glyph) {
      return <div className="result-icon"><Glyph /></div>;
    }
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <polyline points="4 17 10 11 4 5" />
          <line x1="12" y1="19" x2="20" y2="19" />
        </svg>
      </div>
    );
  }

  if (kind === "calc") {
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <rect x="4" y="2" width="16" height="20" rx="2" />
          <rect x="7" y="5" width="10" height="4" rx="1" />
          <circle cx="8" cy="14" r="1" fill="currentColor" stroke="none" />
          <circle cx="12" cy="14" r="1" fill="currentColor" stroke="none" />
          <circle cx="16" cy="14" r="1" fill="currentColor" stroke="none" />
          <circle cx="8" cy="18" r="1" fill="currentColor" stroke="none" />
          <circle cx="12" cy="18" r="1" fill="currentColor" stroke="none" />
          <circle cx="16" cy="18" r="1" fill="currentColor" stroke="none" />
        </svg>
      </div>
    );
  }

  if (kind === "dict" || kind === "dict-hint") {
    return <div className="result-icon"><BookIcon /></div>;
  }

  if (kind === "folder") {
    return (
      <div className="result-icon" data-cat={coloredIcons ? "folder" : undefined}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
        </svg>
      </div>
    );
  }

  // WASM extension results: generic puzzle-piece glyph.
  if (kind.startsWith("ext-")) {
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <path d="M20.5 11H19V7a2 2 0 0 0-2-2h-4V3.5a2.5 2.5 0 0 0-5 0V5H4a2 2 0 0 0-2 2v3.8h1.5a2.7 2.7 0 0 1 0 5.4H2V20a2 2 0 0 0 2 2h3.8v-1.5a2.7 2.7 0 0 1 5.4 0V22H17a2 2 0 0 0 2-2v-4h1.5a2.5 2.5 0 0 0 0-5z" />
        </svg>
      </div>
    );
  }

  if (kind === "file") {
    if (!coloredIcons) {
      return <div className="result-icon"><FileGlyphIcon size={16} /></div>;
    }
    const cat = fileCategory(title);
    // "other" carries no data-cat so it falls back to the generic accent on
    // select (like apps/calc), instead of the muted currentColor-mix branch.
    return (
      <div className="result-icon" data-cat={cat === "other" ? undefined : cat}>
        <CategoryGlyph cat={cat} size={16} />
      </div>
    );
  }

  return <div className="result-icon">{title[0]}</div>;
}
