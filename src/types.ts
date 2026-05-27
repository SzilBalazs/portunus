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
}

export interface ExpiredTimer {
  id: number;
  label: string;
}
