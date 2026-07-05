import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Badge from "../Badge";

interface Props {
  extension: string;
  settingKey: string;
  placeholder?: string;
  /** A value is already stored in the keyring for this key. */
  isSet: boolean;
  /** A Secret Service daemon is reachable; false disables the control. */
  available: boolean;
  /** Called after a successful set/clear so the card can refresh `secrets_set`. */
  onChanged: () => void;
}

/**
 * Masked input for a `type = "secret"` setting. Values are write-only from the
 * UI's side - the stored secret is never fetched back, so the reveal toggle
 * only unmasks the draft being typed. Set/Replace and Clear route through the
 * dedicated secret commands (keyring, never config.toml).
 */
export default function SecretSettingInput({ extension, settingKey, placeholder, isSet, available, onChanged }: Props) {
  const [draft, setDraft] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!available) {
    return (
      <div className="settings-ext-secret">
        <input className="settings-text-input" type="password" disabled placeholder="unavailable" />
        <div className="settings-ext-secret-hint settings-ext-secret-warn">
          No Secret Service found — install gnome-keyring or KWallet to store secrets.
        </div>
      </div>
    );
  }

  const save = () => {
    if (!draft) return;
    setBusy(true);
    setError(null);
    invoke("extension_secret_set", { name: extension, key: settingKey, value: draft })
      .then(() => { setDraft(""); setRevealed(false); onChanged(); })
      .catch(e => setError(String(e)))
      .finally(() => setBusy(false));
  };

  const clear = () => {
    setBusy(true);
    setError(null);
    invoke("extension_secret_clear", { name: extension, key: settingKey })
      .then(() => onChanged())
      .catch(e => setError(String(e)))
      .finally(() => setBusy(false));
  };

  return (
    <div className="settings-ext-secret">
      <div className="settings-ext-secret-row">
        {isSet && !draft && <Badge tone="neutral">stored</Badge>}
        <input
          className="settings-text-input mono"
          type={revealed ? "text" : "password"}
          value={draft}
          autoComplete="off"
          spellCheck={false}
          placeholder={isSet ? "New value to replace" : (placeholder || "Not set")}
          onChange={e => setDraft(e.target.value)}
          onKeyDown={e => { if (e.key === "Enter") save(); }}
        />
        {draft && (
          <button
            className="settings-btn-secondary"
            type="button"
            onClick={() => setRevealed(r => !r)}
            title={revealed ? "Hide" : "Reveal what you're typing"}
          >
            {revealed ? "Hide" : "Show"}
          </button>
        )}
        <button className="settings-btn-secondary" type="button" onClick={save} disabled={!draft || busy}>
          {isSet ? "Replace" : "Set"}
        </button>
        {isSet && (
          <button className="settings-btn-secondary" type="button" onClick={clear} disabled={busy}>
            Clear
          </button>
        )}
      </div>
      {error && <div className="settings-ext-secret-hint settings-ext-secret-warn">{error}</div>}
      <div className="settings-ext-secret-hint">Stored in your system keyring.</div>
    </div>
  );
}
