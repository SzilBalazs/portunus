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
  };
  providers: {
    apps: boolean;
    files: boolean;
    recent: boolean;
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
  files: {
    dirs: DirEntry[];
  };
  recent: {
    max_entries: number;
  };
  search: {
    min_score_file: number;
    min_score_app: number;
    recency_weight: number;
  };
  debug: {
    log_scores: boolean;
    log_watcher: boolean;
    log_pdf: boolean;
  };
  frecency: {
    enabled: boolean;
    half_life_days: number;
    weight: number;
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
  };
  appearance: {
    theme: string;
    font_size: number;
  };
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
  file_size?: number;
  created?: number;
  modified?: number;
  /** 0-based PDF page where the content query mainly matched (content provider only). */
  match_page?: number;
}

export interface ExpiredTimer {
  id: number;
  label: string;
}

export interface DepStatus {
  id: string;
  label: string;
  feature: string;
  available: boolean;
  install_hint: string;
}
