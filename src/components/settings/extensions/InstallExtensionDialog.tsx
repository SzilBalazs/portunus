import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import type { InstallPreview } from "../../../types";
import Modal from "../Modal";
import TextInput from "../TextInput";
import PermissionChips from "./PermissionChips";
import SpawnDangerNotice from "./SpawnDangerNotice";
import NetworkDangerNotice from "./NetworkDangerNotice";
import { useTauriListener } from "../../../hooks/useTauriListener";

interface InstallProgress {
  fetched: number;
  total: number | null;
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

interface Props {
  onClose: () => void;
  /** Called after a successful install (list refresh rides extensions-reloaded too). */
  onInstalled: () => void;
}

type Phase =
  | { step: "input" }
  | { step: "probing" }
  | { step: "consent"; preview: InstallPreview }
  | { step: "installing"; preview: InstallPreview }
  | { step: "error"; message: string };

/**
 * Two-phase sideload flow mirroring the backend: probe a local .portext file
 * (`preview_extension_install` stages + validates the exact bytes), show the
 * consent card (permissions, hash, replaces), then confirm. Cancelling at any
 * point after probing discards the staged bytes. URL installs go through the
 * marketplace instead.
 */
export default function InstallExtensionDialog({ onClose, onInstalled }: Props) {
  const [phase, setPhase] = useState<Phase>({ step: "input" });
  const [expectedSha, setExpectedSha] = useState("");
  // Last source probed, so the error phase can offer a Retry.
  const [lastSource, setLastSource] = useState("");
  const [progress, setProgress] = useState<InstallProgress | null>(null);
  // Sandbox-relaxing grants (spawn allowlist, any-host network) each need an
  // explicit acknowledgement before the confirm button unlocks.
  const [spawnAck, setSpawnAck] = useState(false);
  const [networkAck, setNetworkAck] = useState(false);

  const spawnCmds = phase.step === "consent" ? phase.preview.permissions.spawn : [];
  const networkAny = phase.step === "consent" && phase.preview.permissions.network.includes("*");
  const blockedOnAck = (spawnCmds.length > 0 && !spawnAck) || (networkAny && !networkAck);

  useTauriListener<InstallProgress>("ext-install-progress", p => {
    if (phase.step === "probing") setProgress(p);
  }, [phase.step]);

  const probe = (src: string) => {
    if (!src.trim()) return;
    setLastSource(src.trim());
    setProgress(null);
    setSpawnAck(false);
    setNetworkAck(false);
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
      title="Install extension"
      onClose={close}
      width={470}
      footer={
        phase.step === "input" ? (
          <>
            <button className="settings-btn-secondary" onClick={close}>Cancel</button>
            <button className="settings-btn-primary" onClick={chooseFile}>Choose file…</button>
          </>
        ) : phase.step === "consent" ? (
          <>
            <button className="settings-btn-secondary" onClick={close}>Cancel</button>
            <button
              className="settings-btn-primary"
              onClick={() => confirm(phase.preview)}
              disabled={blockedOnAck}
              title={blockedOnAck ? "Acknowledge the warning above to continue" : undefined}
            >
              {phase.preview.replaces ? `Update to v${phase.preview.version}` : "Install & enable"}
            </button>
          </>
        ) : phase.step === "error" ? (
          <>
            <button className="settings-btn-secondary" onClick={onClose}>Close</button>
            {lastSource && (
              <button className="settings-btn-secondary" onClick={() => probe(lastSource)}>Retry</button>
            )}
            <button className="settings-btn-primary" onClick={() => setPhase({ step: "input" })}>Back</button>
          </>
        ) : undefined
      }
    >
      {phase.step === "input" && (
        <div className="settings-ext-install-input">
          <div className="settings-field-desc">
            Pick a downloaded <code>.portext</code> file. The archive is verified and you review
            its permissions before anything runs.
          </div>
          <TextInput
            value={expectedSha}
            onChange={setExpectedSha}
            placeholder="sha256 (optional - verified against the file)"
            mono
            label="Expected sha256"
          />
        </div>
      )}

      {(phase.step === "probing" || phase.step === "installing") && (
        <div className="settings-ext-install-busy">
          {phase.step === "installing" ? "Installing…" : progress ? (
            <div className="settings-ext-install-progress">
              <div className="settings-ext-install-progress-label">
                {progress.total
                  ? `Downloading… ${fmtBytes(progress.fetched)} / ${fmtBytes(progress.total)}`
                  : `Downloading… ${fmtBytes(progress.fetched)}`}
              </div>
              {progress.total && (
                <div className="settings-ext-install-progress-bar">
                  <div
                    className="settings-ext-install-progress-fill"
                    style={{ width: `${Math.min(100, (progress.fetched / progress.total) * 100)}%` }}
                  />
                </div>
              )}
            </div>
          ) : "Fetching and verifying…"}
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
          {phase.preview.keywords.length > 0 && (
            <div className="settings-field-desc">
              Keywords: {phase.preview.keywords.map(k => <code key={k}>{k}&nbsp;</code>)}
            </div>
          )}
          <PermissionChips permissions={phase.preview.permissions} />
          <SpawnDangerNotice commands={spawnCmds} acked={spawnAck} onAckChange={setSpawnAck} />
          <NetworkDangerNotice any={networkAny} acked={networkAck} onAckChange={setNetworkAck} />
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
