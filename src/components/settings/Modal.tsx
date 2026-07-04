import { ReactNode, useEffect, useRef } from "react";

interface Props {
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
  /** Action row rendered bottom-right (buttons). */
  footer?: ReactNode;
  /** Optional fixed width in px (default 440). */
  width?: number;
}

/**
 * Centered modal dialog over a dimmed backdrop. Esc and backdrop-click close
 * it. The Esc listener runs in the capture phase and stops propagation so the
 * window-level Esc handler in Settings (which hides the whole window) never
 * fires while a dialog is open.
 */
export default function Modal({ title, onClose, children, footer, width = 440 }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopImmediatePropagation();
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <div className="settings-modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="settings-modal" ref={cardRef} style={{ width }} role="dialog" aria-modal="true">
        <div className="settings-modal-title">{title}</div>
        <div className="settings-modal-body">{children}</div>
        {footer && <div className="settings-modal-footer">{footer}</div>}
      </div>
    </div>
  );
}
