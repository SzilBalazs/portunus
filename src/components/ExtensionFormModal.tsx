import { useEffect, useRef, useState } from "react";
import type { FormDto, FormFieldDto } from "../types";

interface Props {
  form: FormDto;
  /** Submit is in flight - inputs lock, the button shows a spinner label. */
  busy: boolean;
  onSubmit: (values: Record<string, unknown>) => void;
  onClose: () => void;
}

/** Initial form values from the fields' declared defaults. */
function initialValues(fields: FormFieldDto[]): Record<string, unknown> {
  const values: Record<string, unknown> = {};
  for (const f of fields) {
    if (f.type === "checkbox") values[f.key] = f.default === true;
    else if (f.type === "number") values[f.key] = typeof f.default === "number" ? f.default : null;
    else if (f.type === "select") values[f.key] = typeof f.default === "string" ? f.default : (f.options?.[0]?.value ?? "");
    else values[f.key] = typeof f.default === "string" ? f.default : "";
  }
  return values;
}

function isEmpty(field: FormFieldDto, value: unknown): boolean {
  if (field.type === "checkbox") return false; // a bool is never "missing"
  if (field.type === "number") return value == null || value === "";
  return String(value ?? "").trim() === "";
}

/**
 * Custom dropdown for `select` fields. A native `<select>` popup cannot open
 * inside the decorationless always-on-top layer-shell launcher window
 * (WebKitGTK's menu is a Wayland popup the compositor won't grant a grab
 * for), so the menu is plain in-DOM markup: click or Enter opens it, typing
 * filters, ↑↓ + Enter or a click picks.
 */
/** Option count above which the dropdown grows a type-to-filter input. Small
 *  lists stay arrow-pickable without the extra focus hop into the filter. */
const SELECT_FILTER_MIN_OPTIONS = 6;

function FieldSelect({ field, value, disabled, invalid, onChange, buttonRef }: {
  field: FormFieldDto;
  value: string;
  disabled: boolean;
  invalid: boolean;
  onChange: (v: string) => void;
  buttonRef?: (el: HTMLElement | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const [index, setIndex] = useState(0);
  const wrapRef = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const rowRef = useRef<HTMLDivElement>(null);

  const options = field.options ?? [];
  const hasFilter = options.length >= SELECT_FILTER_MIN_OPTIONS;
  const f = filter.trim().toLowerCase();
  const shown = f
    ? options.filter(o => o.value.toLowerCase().includes(f) || o.label.toLowerCase().includes(f))
    : options;
  const current = options.find(o => o.value === value);

  // Closing must hand focus back to the trigger button - the filter input
  // unmounts with the menu and focus would otherwise fall to <body>, where
  // the next Enter hits the modal's submit handler.
  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) btnRef.current?.focus();
  };

  useEffect(() => setIndex(0), [filter]);
  useEffect(() => {
    if (open) {
      setFilter("");
      setIndex(Math.max(0, options.findIndex(o => o.value === value)));
      // Focus must land inside the wrap either way - WebKit doesn't focus a
      // button on click, and the wrap's key handlers (↑↓/Enter/Esc) only see
      // keystrokes when focus is within it.
      if (hasFilter) filterRef.current?.focus();
      else btnRef.current?.focus();
    }
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { rowRef.current?.scrollIntoView({ block: "nearest" }); }, [index]);

  // Clicking anywhere outside the widget folds the menu (without stealing
  // focus from whatever was clicked). Esc folds it from anywhere too - focus
  // can stray outside the wrap, where the element-level handler can't hear
  // it (the modal's capture listener stays inert while a menu is open).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) close(false);
    };
    const onEsc = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || wrapRef.current?.contains(e.target as Node)) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      close();
    };
    document.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onEsc, true);
    return () => {
      document.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onEsc, true);
    };
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  const pick = (v: string) => {
    onChange(v);
    close();
  };

  // Keys are handled on the element (not window): the modal's own capture
  // listener lets events inside `.ext-form-select-wrap` propagate here.
  const onKey = (e: React.KeyboardEvent) => {
    if (!open) {
      if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
        e.preventDefault();
        setOpen(true);
      }
      return;
    }
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === "Tab") { close(false); } // the modal's focus trap moves focus; just fold the menu
    else if (e.key === "ArrowDown") { e.preventDefault(); setIndex(i => Math.min(i + 1, shown.length - 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setIndex(i => Math.max(i - 1, 0)); }
    else if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      if (shown[index]) pick(shown[index].value);
    }
  };

  return (
    <div ref={wrapRef} className="ext-form-select-wrap" onKeyDown={onKey}>
      <button
        ref={el => { btnRef.current = el; buttonRef?.(el); }}
        type="button"
        className={`ext-form-input ext-form-select${invalid ? " invalid" : ""}`}
        disabled={disabled}
        onClick={() => setOpen(o => !o)}
      >
        <span className={current ? "ext-form-select-value" : "ext-form-select-placeholder"}>
          {current ? (current.label || current.value) : (field.placeholder ?? "Choose…")}
        </span>
        <span className="ext-form-select-arrow">▾</span>
      </button>
      {open && (
        <div className="ext-form-select-menu">
          {hasFilter && (
            <input
              ref={filterRef}
              className="ext-form-select-filter"
              type="text"
              placeholder="Filter…"
              value={filter}
              onChange={e => setFilter(e.target.value)}
              spellCheck={false}
            />
          )}
          <div className="ext-form-select-list">
            {shown.length === 0 && <div className="ext-form-select-empty">No matches</div>}
            {shown.map((o, i) => (
              <div
                key={o.value}
                ref={i === index ? rowRef : undefined}
                className={`ext-form-select-row${i === index ? " selected" : ""}${o.value === value ? " current" : ""}`}
                onMouseEnter={() => setIndex(i)}
                onMouseDown={e => { e.preventDefault(); pick(o.value); }}
              >
                {o.label || o.value}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * Modal form an extension requested via the ShowForm activate effect. Modal
 * like the action picker: a capture-phase key listener keeps the launcher's
 * own handler inert while it's open. Esc cancels (no call reaches the
 * extension), clicking the backdrop cancels too; Enter (Ctrl+Enter inside a
 * textarea) submits.
 */
export default function ExtensionFormModal({ form, busy, onSubmit, onClose }: Props) {
  const [values, setValues] = useState(() => initialValues(form.fields));
  const [missing, setMissing] = useState<Set<string>>(new Set());
  // Focused field is a textarea - the footer hint swaps ↵ for Ctrl+↵.
  const [inTextarea, setInTextarea] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const firstFieldRef = useRef<HTMLElement | null>(null);

  // A multi-step flow swaps the form in place - reset to the new defaults.
  useEffect(() => {
    setValues(initialValues(form.fields));
    setMissing(new Set());
    firstFieldRef.current?.focus();
  }, [form]);

  useEffect(() => { firstFieldRef.current?.focus(); }, []);

  // Focus can fall to <body> around the modal: the window may have been
  // hidden-then-reshown around a slow activation, or a submit error just
  // re-enabled the disabled fields. Pull it back into the form.
  useEffect(() => {
    const refocus = () => {
      if (document.activeElement === document.body) firstFieldRef.current?.focus();
    };
    window.addEventListener("focus", refocus);
    return () => window.removeEventListener("focus", refocus);
  }, []);
  useEffect(() => {
    if (!busy && document.activeElement === document.body) firstFieldRef.current?.focus();
  }, [busy]);

  // A rejected submit focuses the first missing field instead of leaving the
  // user to hunt for the red label. Flag-gated to the submit itself -
  // `missing` also shrinks as the user types, and that must not yank focus.
  const focusInvalidRef = useRef(false);
  useEffect(() => {
    if (!focusInvalidRef.current || missing.size === 0) return;
    focusInvalidRef.current = false;
    const el = rootRef.current?.querySelector<HTMLElement>(".ext-form-input.invalid");
    el?.focus();
    el?.scrollIntoView({ block: "nearest" });
  }, [missing]);

  const set = (key: string, value: unknown) => {
    setValues(v => ({ ...v, [key]: value }));
    setMissing(m => {
      if (!m.has(key)) return m;
      const next = new Set(m);
      next.delete(key);
      return next;
    });
  };

  const submit = () => {
    if (busy) return;
    const bad = new Set(
      form.fields.filter(f => f.required && isEmpty(f, values[f.key])).map(f => f.key),
    );
    if (bad.size) {
      focusInvalidRef.current = true;
      setMissing(bad);
      return;
    }
    // Number fields travel as numbers; empty optional numbers are omitted.
    const out: Record<string, unknown> = {};
    for (const f of form.fields) {
      const v = values[f.key];
      if (f.type === "number") {
        const n = typeof v === "number" ? v : parseFloat(String(v));
        if (!Number.isNaN(n)) out[f.key] = n;
      } else {
        out[f.key] = v;
      }
    }
    onSubmit(out);
  };

  // No dependency array on purpose: `submit` closes over the live field
  // values, so the listener re-binds each render instead of tracking a
  // dependency list that would have to name every piece of form state.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      // Focus trap: the launcher input behind the overlay is still tabbable,
      // so Tab must cycle within the modal instead of escaping to it. Runs
      // before the select-wrap early return (Tab inside a select must trap
      // too; the widget folds its own menu on Tab).
      if (e.key === "Tab") {
        e.preventDefault();
        const nodes = Array.from(
          rootRef.current?.querySelectorAll<HTMLElement>("input, textarea, button") ?? [],
        ).filter(el => !el.matches(":disabled") && el.tabIndex !== -1);
        if (nodes.length) {
          const i = nodes.indexOf(document.activeElement as HTMLElement);
          const next = e.shiftKey
            ? (i <= 0 ? nodes.length - 1 : i - 1)
            : (i === -1 || i === nodes.length - 1 ? 0 : i + 1);
          nodes[next].focus();
        }
        return;
      }
      const menuOpen = rootRef.current?.querySelector(".ext-form-select-menu") != null;
      // Keys inside a select widget must propagate: its React handlers live
      // further down the capture path (open/drive the menu). One exception -
      // Esc on a closed select isn't the widget's; it cancels the form like
      // Esc anywhere else.
      if (target?.closest(".ext-form-select-wrap")) {
        if (menuOpen || e.key !== "Escape") return;
        e.stopImmediatePropagation();
        e.preventDefault();
        if (!busy) onClose();
        return;
      }
      // A select menu can be open while focus sits outside its wrap (stray
      // blur). Esc must close that menu - not the form - and Enter must not
      // submit underneath it; the widget's outside-click handler and its own
      // keys take over once focus returns.
      if (menuOpen) return;
      // Everything else: capture phase + stopImmediatePropagation, same
      // contract as the action picker.
      e.stopImmediatePropagation();
      if (e.key === "Escape") {
        e.preventDefault();
        if (!busy) onClose();
      } else if (e.key === "Enter" && !e.shiftKey) {
        // Plain Enter submits, except inside a textarea (newline) where the
        // chord is Ctrl+Enter.
        const targetIsTextarea = target?.tagName === "TEXTAREA";
        if (!targetIsTextarea || e.ctrlKey) {
          e.preventDefault();
          submit();
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  const fieldControl = (f: FormFieldDto, i: number) => {
    const ref = i === 0 ? (el: HTMLElement | null) => { firstFieldRef.current = el; } : undefined;
    const invalid = missing.has(f.key);
    const cls = `ext-form-input${invalid ? " invalid" : ""}`;
    switch (f.type) {
      case "textarea":
        return (
          <textarea
            ref={ref as React.Ref<HTMLTextAreaElement>}
            className={cls}
            value={String(values[f.key] ?? "")}
            placeholder={f.placeholder}
            onChange={e => set(f.key, e.target.value)}
            onFocus={() => setInTextarea(true)}
            onBlur={() => setInTextarea(false)}
            disabled={busy}
            rows={4}
            spellCheck={false}
          />
        );
      case "select":
        return (
          <FieldSelect
            field={f}
            value={String(values[f.key] ?? "")}
            disabled={busy}
            invalid={invalid}
            onChange={v => set(f.key, v)}
            buttonRef={ref}
          />
        );
      case "checkbox":
        return (
          <input
            ref={ref as React.Ref<HTMLInputElement>}
            className="ext-form-checkbox"
            type="checkbox"
            checked={values[f.key] === true}
            onChange={e => set(f.key, e.target.checked)}
            disabled={busy}
          />
        );
      case "number":
        return (
          <input
            ref={ref as React.Ref<HTMLInputElement>}
            className={cls}
            type="number"
            value={values[f.key] == null ? "" : String(values[f.key])}
            placeholder={f.placeholder}
            onChange={e => set(f.key, e.target.value === "" ? null : e.target.valueAsNumber)}
            disabled={busy}
          />
        );
      case "password":
        return (
          <input
            ref={ref as React.Ref<HTMLInputElement>}
            className={cls}
            type="password"
            value={String(values[f.key] ?? "")}
            placeholder={f.placeholder}
            onChange={e => set(f.key, e.target.value)}
            disabled={busy}
            spellCheck={false}
          />
        );
      default: // "text" and unknown kinds
        return (
          <input
            ref={ref as React.Ref<HTMLInputElement>}
            className={cls}
            type="text"
            value={String(values[f.key] ?? "")}
            placeholder={f.placeholder}
            onChange={e => set(f.key, e.target.value)}
            disabled={busy}
            spellCheck={false}
          />
        );
    }
  };

  return (
    <div className="ext-form-overlay" onMouseDown={() => { if (!busy) onClose(); }}>
      <div ref={rootRef} className="ext-form" onMouseDown={e => e.stopPropagation()}>
        <div className="ext-form-header">
          <span className="ext-form-title">{form.title}</span>
        </div>
        <div className="ext-form-fields">
          {form.fields.map((f, i) => (
            <label key={f.key} className={`ext-form-field${f.type === "checkbox" ? " inline" : ""}`}>
              <span className="ext-form-label">
                {f.label}
                {f.required && <span className="ext-form-required">*</span>}
              </span>
              {fieldControl(f, i)}
              {missing.has(f.key) && <span className="ext-form-error">Required</span>}
            </label>
          ))}
        </div>
        <div className="ext-form-footer">
          <button className="ext-form-cancel" onClick={onClose} disabled={busy} tabIndex={-1}>
            Cancel <kbd>Esc</kbd>
          </button>
          <button className="ext-form-submit" onClick={submit} disabled={busy}>
            {busy ? "Working…" : (form.submitLabel ?? "Submit")} {!busy && <kbd>{inTextarea ? "Ctrl ↵" : "↵"}</kbd>}
          </button>
        </div>
      </div>
    </div>
  );
}
