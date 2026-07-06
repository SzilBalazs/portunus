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
  calc: {
    currency: boolean;
    rate_max_age_hours: number;
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
  /** Per-extension state keyed by name. Absent = disabled. */
  extensions: Record<string, ExtensionConfigEntry>;
}

export interface ExtensionConfigEntry {
  enabled: boolean;
  /** Values for the extension's declared settings schema. */
  settings: Record<string, unknown>;
}

/** One user-facing action on an extension result. */
export interface ExtAction {
  id: string;
  label: string;
  hint?: string;
}

/** Wire DTO an extension returned for a result; round-tripped on activate/preview. */
export interface ExtensionResult {
  id: string;
  title: string;
  subtitle?: string;
  relevance: number;
  /** First action is the default on Enter; the rest via the action picker. */
  actions?: ExtAction[];
  /** Unvalidated - render `SearchResult.icon_data_uri` instead. */
  icon?: { mime: string; data_base64: string };
  /** Small chip shown right-aligned on the result row. */
  badge?: string;
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

export interface ExtensionPermissions {
  network: string[];
  kv: boolean;
  clipboard: boolean;
  open_url: boolean;
  /** May inject synthetic paste keystrokes into other applications. */
  paste: boolean;
  /** Declares at least one `type = "secret"` setting (stored in the keyring). */
  has_secrets: boolean;
}

/** One input field of a form an extension's `activate` returned. */
export interface FormFieldDto {
  key: string;
  label: string;
  /** "text" | "textarea" | "password" | "select" | "checkbox" | "number"; unknown renders as text. */
  type: string;
  required?: boolean;
  placeholder?: string;
  default?: unknown;
  options?: { value: string; label: string }[];
}

/** Form an extension asked the launcher to render (ShowForm effect). */
export interface FormDto {
  title: string;
  fields: FormFieldDto[];
  submitAction: string;
  submitLabel: string | null;
}

export type ToastLevel = "info" | "success" | "error";

export interface ToastDto {
  message: string;
  level: ToastLevel;
}

/** Response of the `extension_activate` command; side effects already ran. */
export interface ActivateResponse {
  hide: boolean;
  form: FormDto | null;
  toasts: ToastDto[];
  refreshResults: boolean;
}

/** One `[[settings]]` entry from an extension's manifest. */
export interface ExtensionSettingSpec {
  key: string;
  /** "string" | "bool" | "number" | "select" | "secret" */
  type: string;
  label: string;
  description: string;
  default: unknown;
  options: string[];
  min: number | null;
  max: number | null;
  step: number | null;
  placeholder: string;
}

/** One command an extension declares (manifest [[commands]]). */
export interface ExtensionCommandInfo {
  name: string;
  title: string;
  mode: ModeKind;
  /** Search synonyms folded into the command's fuzzy match. */
  keywords: string[];
  /** Command that runs live in root search on every keystroke. */
  always: boolean;
}

/** One installed extension, as reported by the `list_extensions` command. */
export interface ExtensionInfo {
  name: string;
  version: string;
  description: string;
  author: string;
  homepage: string;
  permissions: ExtensionPermissions | null;
  enabled: boolean;
  loaded: boolean;
  error: string | null;
  benched: boolean;
  /** Set when the manifest declares a [background] refresh interval. */
  background_interval_secs: number | null;
  /** The extension's declared [[commands]]. */
  commands: ExtensionCommandInfo[];
  /** Result kind the extension emits (drives launcher group labels). */
  kind: string | null;
  settings_schema: ExtensionSettingSpec[];
  settings_values: Record<string, unknown>;
  /** Permissions grew past the consented snapshot - blocked until re-approved. */
  needs_reconsent: boolean;
  /** "url" | "file" | "dev" - install origin, when recorded. */
  origin: string | null;
  origin_url: string | null;
  /** Extensions dir entry is a symlink (`portunus ext dev`). */
  dev: boolean;
  /** Secret setting keys that currently have a stored keyring value. */
  secrets_set: string[];
}

/** Staged install description from `preview_extension_install`. */
export interface InstallPreview {
  name: string;
  version: string;
  description: string;
  author: string;
  homepage: string;
  permissions: ExtensionPermissions;
  keywords: string[];
  sha256: string;
  size_bytes: number;
  replaces: { old_version: string; permissions_grew: boolean } | null;
  staging_token: string;
}

export interface UpdateCheck {
  current_version: string;
  /** Null when already up to date. */
  preview: InstallPreview | null;
}

export interface ExtensionLogEntry {
  ts_ms: number;
  level: "info" | "error";
  message: string;
}

/** How a command behaves when invoked. */
export type ModeKind = "scope" | "action";

export type CommandSource = { type: "builtin" } | { type: "extension"; name: string };

export type CommandRoute =
  | { type: "builtin"; provider_id: string }
  | { type: "extension"; name: string; command: string }
  /** Frontend swaps in a dedicated component; the backend is not searched. */
  | { type: "ui_takeover" };

/** A searchable launcher command ("Define Word", "Search Issues"). */
export interface CommandDescriptor {
  /** `cmd:<name>` for built-ins, `ext:<name>:cmd:<command>` for extensions. */
  id: string;
  title: string;
  /** Short label for the active-mode chip ("Contents", "Define"). */
  chip: string;
  subtitle?: string;
  source: CommandSource;
  mode_kind: ModeKind;
  /** Search synonyms folded into the command's fuzzy match. */
  keywords: string[];
  /** Input placeholder while the scope is active. */
  placeholder?: string;
  min_query_len: number;
  /** `SearchResult.kind` the command's own results carry. */
  result_kind: string;
  /** Built-in command icon: a named glyph rendered inline (theme-aware). */
  glyph?: string;
  icon_data_uri?: string;
  route: CommandRoute;
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
  /** Extension command that produced this result; passed back on activate/preview. */
  ext_command?: string;
  /** Descriptor behind a `kind: "command"` entry row. */
  command?: CommandDescriptor;
}

/** One extension whose async `query` export is running for the current
 *  keystroke; its batches arrive later as `search-stream` events. */
export interface PendingExt {
  name: string;
  kind: string;
}

/** Response of the `search` command: sync-tier results plus the async tier's
 *  started set. */
export interface SearchResponse {
  query_id: number;
  results: SearchResult[];
  pending: PendingExt[];
}

/** `search-stream` event payload - one streamed batch from an async query. */
export interface StreamPayload {
  query_id: number;
  ext: string;
  results: SearchResult[];
  done: boolean;
  error?: string;
}

/** `extension-preview-chunk` event payload - a streamed preview update that
 *  replaces the rendered content wholesale. */
export interface PreviewChunk {
  request_id: number;
  content: PreviewContent;
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
