import DangerNotice from "./DangerNotice";

interface Props {
  /** Whether the extension requests any-host network access (network = ["*"]). */
  any: boolean;
  acked: boolean;
  onAckChange: (v: boolean) => void;
}

/**
 * THE any-host network consent surface: shown when an extension requests
 * `network = ["*"]` (outbound HTTP to any host, for CDN/rotating pools that
 * can't be enumerated). Sandbox-relaxing, so it carries the same red box and
 * mandatory acknowledgement as the spawn gate, enforced on every enable path.
 */
export default function NetworkDangerNotice({ any, acked, onAckChange }: Props) {
  if (!any) return null;
  return (
    <DangerNotice
      title="Connects to any host"
      ackLabel="I understand this extension can reach any host on the network."
      acked={acked}
      onAckChange={onAckChange}
    >
      This extension can make outbound network requests to <strong>any</strong> host. Extensions
      normally declare the exact hosts they need; this one asks for unrestricted network access, so
      it can send data anywhere. Only continue if you trust the source.
    </DangerNotice>
  );
}
