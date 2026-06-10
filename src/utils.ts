export function shortenPath(path: string): string {
  return path.replace(/^\/home\/[^/]+/, "~").replace(/^\/root/, "~");
}

export function groupLabel(kind: string): string | null {
  if (kind === "calc") return "CALCULATOR";
  if (kind === "dict" || kind === "dict-hint") return "DICTIONARY";
  if (kind === "app") return "APPS";
  if (kind === "file" || kind === "folder") return "FILES";
  if (kind === "clipboard-mode") return "CLIPBOARD";
  if (kind.startsWith("ext-")) return "EXTENSIONS";
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

// Turn a raw .desktop Exec string into a bare binary name for display.
// Drops env-prefix tokens (VAR=value) and field codes (%U %F %i ...), then
// strips any leading path and surrounding quotes. Cosmetic only — never throws.
export function cleanExec(exec: string): string {
  const trimmed = exec.trim();
  const tokens = trimmed.split(/\s+/);
  for (const raw of tokens) {
    if (/^%[a-zA-Z]$/.test(raw)) continue; // field code
    if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(raw)) continue; // env assignment
    if (raw === "env") continue;
    const bin = raw.replace(/^["']|["']$/g, "").split("/").pop();
    if (bin) return bin;
  }
  return trimmed;
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
