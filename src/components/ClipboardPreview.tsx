import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SearchResult } from "../types";

interface Props {
  result: SearchResult;
  onPaste: () => void;
}

export default function ClipboardPreview({ result, onPaste }: Props) {
  const id = result.exec?.replace("clipboard:copy:", "") ?? "";
  const isImage = result.kind === "clipboard-image";

  const [text, setText] = useState<string | null>(null);
  const [imgSrc, setImgSrc] = useState<string | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setText(null);
    setImgSrc(null);
    setError(false);

    invoke<number[]>("decode_clipboard_entry", { id })
      .then(bytes => {
        if (cancelled) return;
        const arr = new Uint8Array(bytes);
        const isPng  = arr[0] === 0x89 && arr[1] === 0x50;
        const isJpeg = arr[0] === 0xff && arr[1] === 0xd8;
        if (isPng || isJpeg) {
          const mime = isPng ? "image/png" : "image/jpeg";
          const url = URL.createObjectURL(new Blob([arr], { type: mime }));
          setImgSrc(url);
        } else {
          setText(new TextDecoder().decode(arr));
        }
      })
      .catch(() => { if (!cancelled) setError(true); });

    return () => { cancelled = true; };
  }, [id]);

  return (
    <div className="clipboard-preview">
      <div className="clipboard-preview-header">
        <span className="clipboard-preview-label">
          {isImage ? result.title : result.subtitle ?? "Text"}
        </span>
        <button className="btn-primary" onClick={onPaste}>
          Paste <span className="btn-kbd">↵</span>
        </button>
      </div>

      {error && <div className="clipboard-preview-empty">Preview unavailable</div>}

      {imgSrc && (
        <div className="clipboard-img-wrap">
          <img src={imgSrc} alt="clipboard image" />
        </div>
      )}

      {text !== null && (
        <div className="clipboard-text-wrap">
          <pre className="clipboard-text">{text}</pre>
        </div>
      )}

      {!error && text === null && !imgSrc && (
        <div className="clipboard-preview-empty">Loading…</div>
      )}
    </div>
  );
}
