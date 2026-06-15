export interface DirEntry {
  path: string;
  depth: number;
}

export interface ContentDirEntry {
  path: string;
  depth: number;
  extensions: string[] | null;
}

/** Pre-index estimate for one directory (from the `estimate_dir_index` command). */
export interface DirEstimate {
  total_files: number;
  pdf_files: number;
  image_files: number;
  est_secs_min: number;
  est_secs_max: number;
}

export interface Config {
  general: {
    max_results: number;
    onboarding_completed: boolean;
    layer_shell: boolean;
  };
  providers: {
    apps: boolean;
    files: boolean;
    calc: boolean;
  };
  dict: {
    enabled: boolean;
    fill_sparse: boolean;
    correct_misspellings: boolean;
    copy_definition: boolean;
    fill_threshold: number;
    fill_max: number;
  };
  clipboard: {
    paste_mode: "auto" | "copy";
    max_entries: number;
    ocr_images: boolean;
  };
  files: {
    dirs: DirEntry[];
    show_dotfiles: boolean;
    colored_icons: boolean;
  };
  search: {
    min_quality: number;
    history_weight: number;
  };
  debug: {
    log_scores: boolean;
    log_watcher: boolean;
    log_pdf: boolean;
  };
  frecency: {
    enabled: boolean;
    half_life_days: number;
  };
  content: {
    enabled: boolean;
    dirs: ContentDirEntry[];
    extensions: string[];
    max_file_bytes: number;
    ocr_images: boolean;
    ocr_pdf_fallback: boolean;
    ocr_language: string;
    threads: number;
    ocr_highlight: boolean;
    ocr_highlight_cache: boolean;
  };
  appearance: {
    theme: string;
    font_size: number;
    animate_results: "off" | "slide" | "flip";
    show_metadata: boolean;
    accent_bleed: boolean;
    slide_selection: boolean;
    grain: number;
  };
  extensions: {
    /** Per-extension enable map keyed by name. Absent = disabled. */
    enabled: Record<string, boolean>;
  };
}

/** Wire DTO an extension returned for a result; round-tripped on activate/preview. */
export interface ExtensionResult {
  id: string;
  title: string;
  subtitle?: string;
  relevance: number;
  actions?: string[];
  /** Unvalidated - render `SearchResult.icon_data_uri` instead. */
  icon?: { mime: string; data_base64: string };
}

/** Declarative preview content returned by an extension's `preview` export. */
export type PreviewContent =
  | { type: "markdown"; content: string }
  | { type: "metadata"; items: { label: string; value: string }[] }
  | { type: "image"; mime: string; data_base64: string }
  | { type: "list"; items: { title: string; subtitle?: string; tag?: string; mono?: boolean }[] }
  | { type: "sections"; items: { heading?: string; rows: string[][] }[] }
  | { type: "code"; lang: string; content: string }
  | { type: "html"; content: string };

/** One installed extension, as reported by the `list_extensions` command. */
export interface ExtensionInfo {
  name: string;
  version: string;
  description: string;
  author: string;
  permissions: { network: string[]; kv: boolean; clipboard: boolean; open_url: boolean } | null;
  enabled: boolean;
  loaded: boolean;
  error: string | null;
  benched: boolean;
  /** Set when the manifest declares a [background] refresh interval. */
  background_interval_secs: number | null;
}

export interface SearchResult {
  id: string;
  title: string;
  subtitle?: string;
  snippet?: string;
  kind: string;
  score: number;
  exec?: string;
  icon_path?: string;
  /** Pre-built `data:` URI for a validated extension-supplied icon. */
  icon_data_uri?: string;
  file_size?: number;
  created?: number;
  modified?: number;
  /** 0-based PDF page where the content query mainly matched (content provider only). */
  match_page?: number;
  /** Original extension DTO for `ext:` results; passed back on activate/preview. */
  ext?: ExtensionResult;
}

/** One clipboard history entry, as returned by the `clipboard_list` command. */
export interface ClipboardEntry {
  id: string;
  /** "text" | "image" */
  kind: string;
  /** First-line snippet (text) or the cliphist binary-data label (image). */
  preview: string;
  /** "text" | "url" | "color" | "json" | "image" */
  content_type: string;
  /** Normalized CSS color when content_type === "color". */
  color: string | null;
  byte_size: number | null;
  dimensions: [number, number] | null;
  format: string | null;
  /** Image entries: cached OCR'd text, searchable. Null until OCR'd. */
  ocr_text?: string | null;
}

export interface ClipboardCapabilities {
  /** Enter pastes into the focused window (wtype on Wayland) vs copy-only. */
  smart_paste: boolean;
}

export interface DepStatus {
  id: string;
  label: string;
  feature: string;
  available: boolean;
  install_hint: string;
}
