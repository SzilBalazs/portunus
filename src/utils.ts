export function shortenPath(path: string): string {
  return path.replace(/^\/home\/[^/]+/, "~").replace(/^\/root/, "~");
}

export function groupLabel(kind: string): string | null {
  if (kind === "calc") return "CALCULATOR";
  if (kind === "app") return "APPS";
  if (kind === "file" || kind === "folder") return "FILES";
  if (kind === "timer-item" || kind === "timer-create" || kind === "timer-new" || kind === "timer-expired") return "TIMERS";
  return null;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

export function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString("en-US", {
    year: "numeric", month: "short", day: "numeric",
  });
}

export function fmtRemaining(secs: number): string {
  if (secs <= 0) return "Done";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

let _audioCtx: AudioContext | null = null;
function audioCtx(): AudioContext {
  if (!_audioCtx || _audioCtx.state === "closed") _audioCtx = new AudioContext();
  return _audioCtx;
}

export async function audioCtxWarmup() {
  const ctx = audioCtx();
  if (ctx.state === "suspended") await ctx.resume();
}

export async function playTimerChime() {
  const ctx = audioCtx();
  if (ctx.state === "suspended") await ctx.resume();

  const master = ctx.createGain();
  master.gain.value = 0.6;
  master.connect(ctx.destination);

  // Bell strike using inharmonic partials (ratios from physical bell acoustics)
  const strike = (freq: number, when: number) => {
    const partials: [ratio: number, amp: number, decay: number][] = [
      [1,    0.50, 2.2],
      [2.76, 0.30, 1.3],
      [5.40, 0.15, 0.7],
      [8.93, 0.06, 0.35],
    ];
    for (const [ratio, amp, decay] of partials) {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.connect(gain);
      gain.connect(master);
      osc.type = "sine";
      osc.frequency.value = freq * ratio;
      gain.gain.setValueAtTime(0, when);
      gain.gain.linearRampToValueAtTime(amp, when + 0.005);
      gain.gain.exponentialRampToValueAtTime(0.0001, when + decay);
      osc.start(when);
      osc.stop(when + decay + 0.05);
    }
  };

  const t = ctx.currentTime;
  strike(523.25, t);        // C5
  strike(659.25, t + 0.30); // E5
  strike(783.99, t + 0.60); // G5
}

export function fileKind(title: string, isFolder: boolean): string {
  if (isFolder) return "Folder";
  const ext = title.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = {
    pdf: "PDF Document",
    png: "PNG Image", jpg: "JPEG Image", jpeg: "JPEG Image",
    gif: "GIF Image", webp: "WebP Image", svg: "SVG Image",
    ts: "TypeScript Source", tsx: "TypeScript Source",
    js: "JavaScript Source", jsx: "JavaScript Source",
    rs: "Rust Source", py: "Python Source", go: "Go Source",
    md: "Markdown", txt: "Text File",
    zip: "Archive", tar: "Archive", gz: "Archive",
    bz2: "Archive", xz: "Archive", "7z": "Archive", rar: "Archive",
    mp4: "Video", mkv: "Video", mov: "Video", avi: "Video",
    mp3: "Audio", flac: "Audio", wav: "Audio", ogg: "Audio",
    json: "JSON Data", xml: "XML Document",
    html: "HTML Document", css: "CSS Stylesheet",
    sh: "Shell Script", toml: "TOML Config",
    yaml: "YAML Config", yml: "YAML Config",
  };
  return map[ext] ?? "File";
}
