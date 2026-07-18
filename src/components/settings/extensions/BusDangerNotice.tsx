import DangerNotice from "./DangerNotice";

interface Props {
  /** Whether the extension requests the companion message bus. */
  enabled: boolean;
  acked: boolean;
  onAckChange: (v: boolean) => void;
}

/**
 * THE bus-permission consent surface: the extension itself stays sandboxed,
 * but it exchanges messages with a separately-installed companion process
 * (browser extension shim, editor plugin, ...) that runs with full account
 * access. Shared by every enable path, like SpawnDangerNotice.
 */
export default function BusDangerNotice({ enabled, acked, onAckChange }: Props) {
  if (!enabled) return null;
  return (
    <DangerNotice
      title="Talks to a companion app outside the sandbox"
      ackLabel="I understand this extension exchanges messages with an unsandboxed companion app."
      acked={acked}
      onAckChange={onAckChange}
    >
      This extension exchanges messages with a companion app on your computer (for example a
      browser extension it works with). The extension stays sandboxed, but whatever it sends
      leaves the sandbox, and the companion acts with your full account access. Only continue
      if you trust both the extension and its companion.
    </DangerNotice>
  );
}
