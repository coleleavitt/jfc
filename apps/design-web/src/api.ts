import type {
  BundleResponse,
  CapabilitiesResponse,
  ChatResponse,
  DcWriteResponse,
  DirectEditApplyResponse,
  DirectEditInspectResponse,
  DesignSystemManifest,
  DirectEditResponse,
  EvalJsResponse,
  FileReadResponse,
  GeneratedMediaResponse,
  HandoffResponse,
  MultiScreenshotResponse,
  PdfResponse,
  PptxResponse,
  ProjectMeta,
  ProjectResponse,
  PublicShareRecord,
  PublicTokenResponse,
  ScreenshotResponse,
  TweaksResponse,
  VerifyOrchestrateResponse,
  VerifyResponse
} from './types';

const API_BASE = import.meta.env.VITE_JFC_DESIGN_API ?? '';

class ApiError extends Error {
  status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: {
      'content-type': 'application/json',
      ...init?.headers
    }
  });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: string };
      message = body.error ?? message;
    } catch {
      // Keep the status text.
    }
    throw new ApiError(response.status, message);
  }
  return (await response.json()) as T;
}

function fileQuery(path: string): string {
  return new URLSearchParams({ path }).toString();
}

export const designApi = {
  health: () => request<{ ok: boolean }>('/health'),

  capabilities: () => request<CapabilitiesResponse>('/design/capabilities'),

  listProjects: () => request<ProjectMeta[]>('/design/projects'),

  createProject: (title: string) =>
    request<ProjectResponse>('/design/projects', {
      method: 'POST',
      body: JSON.stringify({ title })
    }),

  getProject: (projectId: string) =>
    request<ProjectResponse>(`/design/projects/${encodeURIComponent(projectId)}`),

  updateProject: (
    projectId: string,
    updates: { title?: string; is_design_system?: boolean }
  ) =>
    request<ProjectResponse>(`/design/projects/${encodeURIComponent(projectId)}`, {
      method: 'PATCH',
      body: JSON.stringify(updates)
    }),

  listFiles: (projectId: string) =>
    request<string[]>(`/design/projects/${encodeURIComponent(projectId)}/files`),

  readFile: (projectId: string, path: string) =>
    request<FileReadResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/file?${fileQuery(path)}`
    ),

  writeFile: (
    projectId: string,
    path: string,
    content: string,
    assetName?: string
  ) =>
    request<ProjectResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/file?${fileQuery(path)}`,
      {
        method: 'PUT',
        body: JSON.stringify({
          content,
          encoding: 'utf-8',
          asset_name: assetName || undefined
        })
      }
    ),

  deleteFile: (projectId: string, path: string) =>
    request<ProjectResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/file?${fileQuery(path)}`,
      { method: 'DELETE' }
    ),

  copyFile: (projectId: string, fromPath: string, toPath: string) =>
    request<ProjectResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/files/copy`,
      {
        method: 'POST',
        body: JSON.stringify({ from_path: fromPath, to_path: toPath })
      }
    ),

  registerAsset: (projectId: string, name: string, path: string) =>
    request<ProjectResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/assets`,
      {
        method: 'POST',
        body: JSON.stringify({ name, path })
      }
    ),

  unregisterAsset: (projectId: string, path: string) =>
    request<ProjectResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/assets/by-path?${fileQuery(path)}`,
      { method: 'DELETE' }
    ),

  bundleHtml: (projectId: string, input: string, output?: string) =>
    request<BundleResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/super-inline-html`,
      {
        method: 'POST',
        body: JSON.stringify({
          input,
          output: output || undefined,
          require_thumbnail: false
        })
      }
    ),

  handoff: (projectId: string, feature: string, files: string[]) =>
    request<HandoffResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/handoff`,
      {
        method: 'POST',
        body: JSON.stringify({ feature, files })
      }
    ),

  checkDesignSystem: (projectId: string) =>
    request<DesignSystemManifest>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/check-design-system`,
      { method: 'POST', body: JSON.stringify({}) }
    ),

  evalJs: (projectId: string, path: string, script: string) =>
    request<EvalJsResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/eval-js`,
      {
        method: 'POST',
        body: JSON.stringify({ path, script, wait_ms: 150 })
      }
    ),

  screenshot: (projectId: string, path: string, output?: string) =>
    request<ScreenshotResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/screenshot`,
      {
        method: 'POST',
        body: JSON.stringify({
          path,
          output: output || undefined,
          full_page: true,
          wait_ms: 250
        })
      }
    ),

  multiScreenshot: (projectId: string, path: string, outputDir?: string, selector?: string) =>
    request<MultiScreenshotResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/multi-screenshot`,
      {
        method: 'POST',
        body: JSON.stringify({
          path,
          output_dir: outputDir || undefined,
          selector: selector || undefined,
          max_items: 12,
          include_data: false,
          wait_ms: 250
        })
      }
    ),

  genPptx: (
    projectId: string,
    path: string,
    output?: string,
    selector?: string,
    mode?: 'editable' | 'screenshots'
  ) =>
    request<PptxResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/gen-pptx`,
      {
        method: 'POST',
        body: JSON.stringify({
          path,
          output: output || undefined,
          selector: selector || undefined,
          mode: mode || 'editable',
          fallback: true,
          wait_ms: 250
        })
      }
    ),

  publicToken: (projectId: string, path: string) =>
    request<PublicTokenResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/public-token`,
      { method: 'POST', body: JSON.stringify({ path }) }
    ),

  publicShares: (projectId: string) =>
    request<PublicShareRecord[]>(
      `/design/projects/${encodeURIComponent(projectId)}/public-shares`
    ),

  revokePublicShare: (projectId: string, token: string) =>
    request<PublicShareRecord[]>(
      `/design/projects/${encodeURIComponent(projectId)}/public-shares/${encodeURIComponent(token)}`,
      { method: 'DELETE' }
    ),

  writeDirectEditOverrides: (projectId: string, overrides: unknown, path?: string) =>
    request<DirectEditResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/direct-edit-overrides`,
      {
        method: 'PUT',
        body: JSON.stringify({ path: path || undefined, overrides })
      }
    ),

  directEditInspect: (projectId: string, path: string, selector: string) =>
    request<DirectEditInspectResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/direct-edit-inspect`,
      {
        method: 'POST',
        body: JSON.stringify({ path, selector, wait_ms: 150 })
      }
    ),

  directEditApply: (
    projectId: string,
    path: string,
    selector: string,
    edit: {
      text?: string;
      html?: string;
      attributes?: unknown;
      styles?: unknown;
      source_path?: string;
      source_start?: number;
      source_end?: number;
      source_line?: number;
      source_column?: number;
      source_kind?: string;
      generated_path?: string;
      generated_line?: number;
      generated_column?: number;
      source_map_path?: string;
      previous_text?: string;
      fallback_overlay?: boolean;
    }
  ) =>
    request<DirectEditApplyResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/direct-edit-apply`,
      {
        method: 'POST',
        body: JSON.stringify({ path, selector, ...edit })
      }
    ),

  writeTweaks: (projectId: string, values: unknown, path?: string) =>
    request<TweaksResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/tweaks`,
      {
        method: 'PUT',
        body: JSON.stringify({ path: path || undefined, values })
      }
    ),

  dcWrite: (
    projectId: string,
    path: string,
    content: string,
    append = false,
    streaming = false,
    name?: string,
    kind?: string
  ) =>
    request<DcWriteResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/dc-write`,
      {
        method: 'POST',
        body: JSON.stringify({ path, content, append, streaming, name, kind })
      }
    ),

  verify: (projectId: string, path: string, output?: string, selector?: string) =>
    request<VerifyResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/verify`,
      {
        method: 'POST',
        body: JSON.stringify({
          path,
          output: output || undefined,
          selector: selector || undefined,
          max_screenshots: 8,
          wait_ms: 250
        })
      }
    ),

  verifyOrchestrate: (projectId: string, path: string, outputDir?: string, selector?: string) =>
    request<VerifyOrchestrateResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/verify-orchestrate`,
      {
        method: 'POST',
        body: JSON.stringify({
          path,
          output_dir: outputDir || undefined,
          selector: selector || undefined,
          wait_ms: 250
        })
      }
    ),

  done: (projectId: string, path: string, outputDir?: string, selector?: string) =>
    request<VerifyOrchestrateResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/done`,
      {
        method: 'POST',
        body: JSON.stringify({
          path,
          output_dir: outputDir || undefined,
          selector: selector || undefined,
          wait_ms: 250
        })
      }
    ),

  savePdf: (projectId: string, path: string, output?: string) =>
    request<PdfResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/save-pdf`,
      {
        method: 'POST',
        body: JSON.stringify({
          path,
          output: output || undefined,
          print_background: true,
          wait_ms: 250
        })
      }
    ),

  chatHistory: (projectId: string) =>
    request<ChatResponse>(`/design/projects/${encodeURIComponent(projectId)}/tools/chat`),

  chat: (projectId: string, message: string, path?: string) =>
    request<ChatResponse>(`/design/projects/${encodeURIComponent(projectId)}/tools/chat`, {
      method: 'POST',
      body: JSON.stringify({ message, path: path || undefined })
    }),

  generateImage: (
    projectId: string,
    prompt: string,
    output?: string,
    width?: number,
    height?: number,
    style?: string,
    provider?: string
  ) =>
    request<GeneratedMediaResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/generate-image`,
      {
        method: 'POST',
        body: JSON.stringify({
          prompt,
          output: output || undefined,
          width: width || undefined,
          height: height || undefined,
          style: style || undefined,
          provider: provider || undefined
        })
      }
    ),

  generateSound: (
    projectId: string,
    prompt: string,
    output?: string,
    durationMs?: number,
    provider?: string
  ) =>
    request<GeneratedMediaResponse>(
      `/design/projects/${encodeURIComponent(projectId)}/tools/generate-sound`,
      {
        method: 'POST',
        body: JSON.stringify({
          prompt,
          output: output || undefined,
          duration_ms: durationMs || undefined,
          provider: provider || undefined
        })
      }
    )
};

export function previewUrl(projectId: string, path?: string): string {
  const cleanPath = path?.replace(/^\/+/, '') ?? '';
  const suffix = cleanPath ? `/${cleanPath.split('/').map(encodeURIComponent).join('/')}` : '';
  return `${API_BASE}/design/projects/${encodeURIComponent(projectId)}/serve${suffix}`;
}

export function downloadUrl(projectId: string, path: string): string {
  return `${API_BASE}/design/projects/${encodeURIComponent(projectId)}/download?${fileQuery(path)}`;
}

export function printUrl(projectId: string, path: string): string {
  return `${API_BASE}/design/projects/${encodeURIComponent(projectId)}/print?${fileQuery(path)}`;
}
