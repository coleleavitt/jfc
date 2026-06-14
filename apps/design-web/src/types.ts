export type Asset = {
  name: string;
  path: string;
};

export type ProjectMeta = {
  id: string;
  title: string;
  is_design_system: boolean;
  assets: Asset[];
};

export type ProjectResponse = {
  meta: ProjectMeta;
  root: string;
};

export type FileReadResponse = {
  path: string;
  encoding: 'utf-8' | 'base64';
  content: string;
};

export type Capability = {
  feature: string;
  surface: string;
  status: 'done' | 'via_existing' | 'partial' | 'planned';
};

export type CapabilitiesResponse = {
  matrix: Capability[];
  markdown: string;
};

export type DesignEvent = {
  id: string;
  ts_ms: number;
  project_id: string | null;
  kind: string;
  path: string | null;
  meta: ProjectMeta | null;
  data: unknown | null;
};

export type BundleResponse = {
  output: string;
  bytes: number;
  misses: string[];
  summary: string;
};

export type HandoffResponse = {
  dir: string;
  readme: string;
  copied: string[];
};

export type BrowserLog = {
  type: string;
  text: string;
  location?: unknown;
};

export type EvalJsResponse = {
  ok: boolean;
  path: string;
  url: string;
  title: string;
  result: unknown;
  logs: BrowserLog[];
  errors: string[];
  duration_ms: number;
};

export type ScreenshotResponse = {
  ok: boolean;
  path: string;
  output: string;
  bytes: number;
  data_base64: string;
  logs: BrowserLog[];
  errors: string[];
  duration_ms: number;
};

export type MultiScreenshotResponse = {
  ok: boolean;
  path: string;
  output: string;
  output_dir: string;
  selector: string;
  screenshots: Array<{
    index: number;
    output: string | null;
    bytes: number;
    hash: string;
    data_base64: string | null;
  }>;
  logs: BrowserLog[];
  errors: string[];
  duration_ms: number;
};

export type PptxResponse = {
  ok: boolean;
  path: string;
  output: string;
  bytes: number;
  slides: number;
  mode: string;
  warnings?: string[];
  validation?: {
    selector?: string;
    editable_elements?: number;
    notes?: number;
    duplicate_hashes?: number;
  };
  logs: BrowserLog[];
  errors: string[];
  duration_ms: number;
};

export type PdfResponse = {
  ok: boolean;
  path: string;
  output: string;
  bytes: number;
  logs: BrowserLog[];
  errors: string[];
  duration_ms: number;
};

export type PublicTokenResponse = {
  path: string;
  token: string;
  url: string;
  public_url: string;
  embed_url: string;
  expires_at_ms: number;
};

export type PublicShareRecord = {
  path: string;
  token: string;
  public_url: string;
  embed_url: string;
  issued_at_ms: number;
  expires_at_ms: number;
  scope_dir: string;
  title: string | null;
  allow_download: boolean;
  revoked_at_ms: number | null;
};

export type DirectEditResponse = {
  path: string;
  overrides: unknown;
};

export type DirectEditInspectResponse = {
  ok: boolean;
  path: string;
  selector: string | null;
  selectors: string[];
  tag?: string;
  text?: string;
  rect?: { x: number; y: number; width: number; height: number };
  styles?: Record<string, string>;
  attributes?: Record<string, string>;
  source?: unknown;
  logs: BrowserLog[];
  errors: string[];
  duration_ms: number;
};

export type DirectEditApplyResponse = {
  path: string;
  selector: string;
  bytes: number;
  runtime: string;
  overrides_path: string;
  source_rewritten: boolean;
  source_path: string | null;
  fallback_reason: string | null;
  overrides: unknown;
};

export type TweaksResponse = {
  path: string;
  values: unknown;
};

export type DcWriteResponse = {
  path: string;
  bytes: number;
  appended: boolean;
  streaming: boolean;
  name: string | null;
  kind: string | null;
};

export type VerifyResponse = {
  ok: boolean;
  path: string;
  output: string;
  screenshot: {
    bytes: number;
    hash: string;
    data_base64: string | null;
  };
  stats: Record<string, unknown>;
  checks: Array<{ name: string; status: 'pass' | 'warn' | 'fail'; detail: string }>;
  warnings: string[];
  logs: BrowserLog[];
  errors: string[];
  duration_ms: number;
};

export type VerifyOrchestrateResponse = {
  ok: boolean;
  path: string;
  output_dir: string;
  runs: Array<{
    name: string;
    viewport: { width: number; height: number };
    result: VerifyResponse;
  }>;
  warnings: string[];
  verdict?: string;
};

export type ChatMessage = {
  role: string;
  content: string;
  ts_ms: number;
  path: string | null;
};

export type ChatResponse = {
  path: string;
  reply: string;
  actions: string[];
  messages: ChatMessage[];
};

export type GeneratedMediaResponse = {
  output: string;
  bytes: number;
  provider: string;
  mime: string;
  prompt: string;
};

export type DesignSystemManifest = {
  namespace: string;
  entry_css: string | null;
  tokens: string[];
  fonts: string[];
  components: Array<{ name: string; jsx: string; dts: string | null }>;
  cards: Array<{ file: string; group: string | null; name: string | null }>;
  starting_points: Array<{ file: string; kind: string; section: string | null }>;
  issues: string[];
};
