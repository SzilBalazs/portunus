import { useState } from "react";
import Modal from "../Modal";
import SpawnDangerNotice from "./SpawnDangerNotice";
import NetworkDangerNotice from "./NetworkDangerNotice";
import BusDangerNotice from "./BusDangerNotice";

interface Props {
  title: string;
  /** Spawn allowlist to warn about (empty = no spawn gate). */
  spawnCommands?: string[];
  /** Whether to warn about newly-granted any-host network access. */
  networkAny?: boolean;
  /** Whether to warn about the companion message-bus grant. */
  bus?: boolean;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Blocking consent gate for sandbox-relaxing grants on the paths that don't go
 * through the install dialog: enabling a hand-dropped/dev extension, and
 * re-approving grown permissions. Shows every applicable danger notice (spawn,
 * any-host network); Confirm stays disabled until each shown acknowledgement is
 * ticked, matching the install flow so the warnings are enforced uniformly.
 * Acks are local state, fresh on every mount.
 */
export default function DangerConsentModal({
  title,
  spawnCommands = [],
  networkAny = false,
  bus = false,
  confirmLabel,
  onConfirm,
  onCancel,
}: Props) {
  const [spawnAck, setSpawnAck] = useState(false);
  const [networkAck, setNetworkAck] = useState(false);
  const [busAck, setBusAck] = useState(false);
  const blocked =
    (spawnCommands.length > 0 && !spawnAck) || (networkAny && !networkAck) || (bus && !busAck);
  return (
    <Modal
      title={title}
      onClose={onCancel}
      width={470}
      footer={
        <>
          <button className="settings-btn-secondary" onClick={onCancel}>Cancel</button>
          <button
            className="settings-btn-danger"
            onClick={onConfirm}
            disabled={blocked}
            title={blocked ? "Acknowledge the warning above to continue" : undefined}
          >
            {confirmLabel}
          </button>
        </>
      }
    >
      <SpawnDangerNotice commands={spawnCommands} acked={spawnAck} onAckChange={setSpawnAck} />
      <NetworkDangerNotice any={networkAny} acked={networkAck} onAckChange={setNetworkAck} />
      <BusDangerNotice enabled={bus} acked={busAck} onAckChange={setBusAck} />
    </Modal>
  );
}
