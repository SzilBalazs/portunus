import { useState, useContext } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { BookIcon, CategoryGlyph, FileGlyphIcon } from "../icons";
import { fileCategory } from "../utils";
import { ColoredIconsContext } from "../coloredIcons";

interface Props {
  icon_path?: string;
  iconDataUri?: string;
  title: string;
  kind: string;
}

export default function ResultIcon({ icon_path, iconDataUri, title, kind }: Props) {
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

  if (kind === "clipboard-mode") {
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <rect x="8" y="2" width="8" height="4" rx="1" />
          <path d="M8 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2h-2" />
          <line x1="8" y1="12" x2="16" y2="12" />
          <line x1="8" y1="16" x2="13" y2="16" />
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
