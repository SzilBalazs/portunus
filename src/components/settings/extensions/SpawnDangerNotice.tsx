import { interpretersIn } from "../../../spawn";
import DangerNotice from "./DangerNotice";

interface Props {
  /** The allowlisted commands the extension may launch (permissions.spawn). */
  commands: string[];
  acked: boolean;
  onAckChange: (v: boolean) => void;
}

function CommandList({ commands }: { commands: string[] }) {
  return (
    <>
      {commands.map((c, i) => (
        <span key={c}>{i > 0 ? ", " : ""}<code>{c}</code></span>
      ))}
    </>
  );
}

/**
 * THE spawn-permission consent surface: the red danger box naming the exact
 * binaries, an escalated warning when any of them is a shell/interpreter, and
 * the mandatory "I understand" checkbox. Shared by every path that can enable a
 * spawn extension (install dialog, enable toggle, reconsent) so the hard gate
 * can't be bypassed by one path forgetting it.
 */
export default function SpawnDangerNotice({ commands, acked, onAckChange }: Props) {
  if (commands.length === 0) return null;
  const interpreters = interpretersIn(commands);
  return (
    <DangerNotice
      title="Runs programs outside the sandbox"
      ackLabel="I understand this extension can run programs outside the sandbox."
      acked={acked}
      onAckChange={onAckChange}
    >
      This extension can launch programs on your computer: <CommandList commands={commands} />.
      Extensions are normally sandboxed and cannot touch your system — this one asks to break out
      of that sandbox, so it runs with your full account access. Only continue if you trust the
      source.
      {interpreters.length > 0 && (
        <div className="settings-ext-danger-escalate">
          <CommandList commands={interpreters} />{" "}
          {interpreters.length === 1 ? "is a command interpreter — it can" : "are command interpreters — they can"} run
          {" "}<strong>any</strong> program, not just itself, so this grants effectively unrestricted access.
        </div>
      )}
    </DangerNotice>
  );
}
