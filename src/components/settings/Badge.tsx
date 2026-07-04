interface Props {
  /** Visual tone; maps to a `settings-badge--<tone>` class. */
  tone?: "new" | "dev" | "update" | "error" | "neutral";
  children: React.ReactNode;
}

/** Tiny inline label chip (new / dev / update available / error). */
export default function Badge({ tone = "neutral", children }: Props) {
  return <span className={`settings-badge settings-badge--${tone}`}>{children}</span>;
}
