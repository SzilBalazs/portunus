export function shortenPath(path: string): string {
  return path.replace(/^\/home\/[^/]+/, "~").replace(/^\/root/, "~");
}

export function groupLabel(kind: string): string | null {
  if (kind === "calc") return "CALCULATOR";
  if (kind === "dict" || kind === "dict-hint") return "DICTIONARY";
  if (kind === "app") return "APPS";
  if (kind === "file" || kind === "folder") return "FILES";
  if (kind === "timer-item" || kind === "timer-create" || kind === "timer-new" || kind === "timer-expired" || kind === "timer-hint") return "TIMERS";
  if (kind === "clipboard" || kind === "clipboard-image") return "CLIPBOARD";
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

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "tif"]);

export function isImagePreviewable(filename: string): boolean {
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  return IMAGE_EXTS.has(ext);
}

export function isSvg(filename: string): boolean {
  return filename.split(".").pop()?.toLowerCase() === "svg";
}

export function isCsv(filename: string): boolean {
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  return ext === "csv" || ext === "tsv";
}

const OFFICE_TEXT_EXTS = new Set(["docx", "pptx", "odt", "odp"]);
const SPREADSHEET_EXTS = new Set(["xlsx", "ods"]);

export function isOfficeText(filename: string): boolean {
  return OFFICE_TEXT_EXTS.has(filename.split(".").pop()?.toLowerCase() ?? "");
}

export function isSpreadsheet(filename: string): boolean {
  return SPREADSHEET_EXTS.has(filename.split(".").pop()?.toLowerCase() ?? "");
}

const TEXT_PREVIEW_LANGS: Record<string, string> = {
  rs: "rust",
  ts: "typescript", tsx: "typescript",
  js: "javascript", jsx: "javascript",
  py: "python",
  go: "go",
  sh: "bash", bash: "bash", zsh: "bash",
  json: "json",
  toml: "ini",
  ini: "ini", conf: "ini", cfg: "ini", env: "ini",
  yaml: "yaml", yml: "yaml",
  md: "markdown",
  css: "css", scss: "scss", less: "less",
  html: "xml", htm: "xml", xml: "xml", vue: "xml",
  c: "c", h: "c",
  cpp: "cpp", cc: "cpp", cxx: "cpp", hh: "cpp", hpp: "cpp",
  java: "java",
  rb: "ruby",
  kt: "kotlin", kts: "kotlin",
  sql: "sql",
  php: "php",
  lua: "lua",
  swift: "swift",
  dockerfile: "dockerfile",
  makefile: "makefile",
  rst: "plaintext", log: "plaintext",
  txt: "plaintext",
};

export function textPreviewLang(filename: string): string | null {
  // `split(".").pop()` returns the whole name for extensionless files,
  // so "Dockerfile"/"Makefile" map correctly via their lowercased name.
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  return TEXT_PREVIEW_LANGS[ext] ?? null;
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
    docx: "Word Document", xlsx: "Excel Spreadsheet",
    pptx: "PowerPoint Presentation",
    odt: "OpenDocument Text", ods: "OpenDocument Spreadsheet",
    odp: "OpenDocument Presentation",
    java: "Java Source", rb: "Ruby Source",
    kt: "Kotlin Source", kts: "Kotlin Script",
    swift: "Swift Source", php: "PHP Source", lua: "Lua Source",
    c: "C Source", h: "C Header",
    cpp: "C++ Source", cc: "C++ Source", cxx: "C++ Source",
    hh: "C++ Header", hpp: "C++ Header",
    sql: "SQL Script", vue: "Vue Component",
    md: "Markdown", txt: "Text File",
    rst: "reStructuredText", log: "Log File",
    csv: "CSV Data", tsv: "TSV Data",
    zip: "Archive", tar: "Archive", gz: "Archive",
    bz2: "Archive", xz: "Archive", "7z": "Archive", rar: "Archive",
    mp4: "Video", mkv: "Video", mov: "Video", avi: "Video",
    mp3: "Audio", flac: "Audio", wav: "Audio", ogg: "Audio",
    json: "JSON Data", xml: "XML Document",
    html: "HTML Document", css: "CSS Stylesheet",
    scss: "Sass Stylesheet", less: "Less Stylesheet",
    sh: "Shell Script", bash: "Shell Script", zsh: "Shell Script",
    toml: "TOML Config",
    ini: "INI Config", conf: "Config File", cfg: "Config File", env: "Env File",
    yaml: "YAML Config", yml: "YAML Config",
  };
  return map[ext] ?? "File";
}
