import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import type { InstallPreview } from "../../../types";
import Modal from "../Modal";
import TextInput from "../TextInput";
import PermissionChips from "./PermissionChips";

interface Props {
  onClose: () => void;
  /** Called after a successful install (list refresh rides extensions-reloaded too). */
  onInstalled: () => void;
  /** Pre-staged preview (update flow) - skips the source input step. */
  initialPreview?: InstallPreview;
}

type Phase =
  | { step: "input" }
  | { step: "probing" }
  | { step: "consent"; preview: InstallPreview }
  | { step: "installing"; preview: InstallPreview }
  | { step: "error"; message: string };

/**
 * Two-phase install flow mirroring the backend: probe a URL or local file
 * (`preview_extension_install` stages + validates the exact bytes), show the
 * consent card (permissions, hash, replaces), then confirm. Cancelling at any
 * point after probing discards the staged bytes.
 */
export default function InstallExtensionDialog({ onClose, onInstalled, initialPreview }: Props) {
  const [phase, setPhase] = useState<Phase>(
    initialPreview ? { step: "consent", preview: initialPreview } : { step: "input" },
  );
  const [source, setSource] = useState("");
  const [expectedSha, setExpectedSha] = useState("");

  const probe = (src: string) => {
    if (!src.trim()) return;
    setPhase({ step: "probing" });
    invoke<InstallPreview>("preview_extension_install", {
      source: src.trim(),
      expectedSha256: expectedSha.trim() || null,
    })
      .then(preview => setPhase({ step: "consent", preview }))
      .catch(e => setPhase({ step: "error", message: String(e) }));
  };

  const chooseFile = async () => {
    const path = await pickFile({
      multiple: false,
      filters: [{ name: "Portunus extension", extensions: ["portext", "zip"] }],
    });
    if (typeof path === "string") {
      setSource(path);
      probe(path);
    }
  };

  const cancelStaged = (preview: InstallPreview) => {
    invoke("cancel_extension_install", { stagingToken: preview.staging_token }).catch(() => {});
  };

  const close = () => {
    if (phase.step === "consent") cancelStaged(phase.preview);
    onClose();
  };

  const confirm = (preview: InstallPreview) => {
    setPhase({ step: "installing", preview });
    invoke<string>("confirm_extension_install", { stagingToken: preview.staging_token })
      .then(() => { onInstalled(); onClose(); })
      .catch(e => setPhase({ step: "error", message: String(e) }));
  };

  return (
    <Modal
      title={initialPreview ? `Update ${initialPreview.name}` : "Install extension"}
      onClose={close}
      width={470}
      footer={
        phase.step === "input" ? (
          <>
            <button className="settings-btn-secondary" onClick={close}>Cancel</button>
            <button className="settings-btn-primary" onClick={() => probe(source)} disabled={!source.trim()}>Fetch</button>
          </>
        ) : phase.step === "consent" ? (
          <>
            <button className="settings-btn-secondary" onClick={close}>Cancel</button>
            <button className="settings-btn-primary" onClick={() => confirm(phase.preview)}>
              {phase.preview.replaces ? `Update to v${phase.preview.version}` : "Install & enable"}
            </button>
          </>
        ) : phase.step === "error" ? (
          <>
            <button className="settings-btn-secondary" onClick={onClose}>Close</button>
            {!initialPreview && (
              <button className="settings-btn-primary" onClick={() => setPhase({ step: "input" })}>Back</button>
            )}
          </>
        ) : undefined
      }
    >
      {phase.step === "input" && (
        <div className="settings-ext-install-input">
          <div className="settings-field-desc">
            Paste a <code>.portext</code> URL, or pick a downloaded file. The archive is verified
            and you review its permissions before anything runs.
          </div>
          <TextInput
            value={source}
            onChange={setSource}
            placeholder="https://example.com/my-extension.portext"
            mono
            autoFocus
            onEnter={() => probe(source)}
            label="Extension URL or path"
          />
          <TextInput
            value={expectedSha}
            onChange={setExpectedSha}
            placeholder="sha256 (optional - verified against the download)"
            mono
            label="Expected sha256"
          />
          <button className="settings-btn-secondary" onClick={chooseFile}>Choose file…</button>
        </div>
      )}

      {(phase.step === "probing" || phase.step === "installing") && (
        <div className="settings-ext-install-busy">
          {phase.step === "probing" ? "Fetching and verifying…" : "Installing…"}
        </div>
      )}

      {phase.step === "consent" && (
        <div className="settings-ext-consent">
          <div className="settings-ext-consent-head">
            <span className="settings-ext-consent-name">{phase.preview.name}</span>
            <span className="settings-ext-version">v{phase.preview.version}</span>
            {phase.preview.author && <span className="settings-ext-consent-author">by {phase.preview.author}</span>}
          </div>
          {phase.preview.description && <div className="settings-field-desc">{phase.preview.description}</div>}
          {phase.preview.triggers.length > 0 && (
            <div className="settings-field-desc">
              Trigger: {phase.preview.triggers.map(t => <code key={t}>{t}&nbsp;</code>)}
            </div>
          )}
          <PermissionChips permissions={phase.preview.permissions} />
          {phase.preview.replaces && (
            <div className="settings-field-desc">
              Replaces installed v{phase.preview.replaces.old_version}
              {phase.preview.replaces.permissions_grew && (
                <strong> - requests new permissions, review above</strong>
              )}
            </div>
          )}
          <div className="settings-ext-hash" title={phase.preview.sha256}>
            sha256 <code>{phase.preview.sha256}</code>
          </div>
        </div>
      )}

      {phase.step === "error" && <div className="settings-ext-install-error">{phase.message}</div>}
    </Modal>
  );
}
