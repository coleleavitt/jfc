import {
  CheckCircle2,
  Copy,
  Download,
  FileCode2,
  FileJson2,
  FilePlus2,
  FolderTree,
  Globe2,
  Image,
  LayoutDashboard,
  MessageSquare,
  Music2,
  PackageCheck,
  Play,
  Plus,
  Printer,
  RefreshCcw,
  Save,
  SearchCheck,
  Send,
  Settings2,
  SlidersHorizontal,
  Trash2,
  XCircle
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';

import { designApi, downloadUrl, previewUrl, printUrl } from './api';
import type {
  Asset,
  BundleResponse,
  Capability,
  ChatMessage,
  ChatResponse,
  DcWriteResponse,
  DesignEvent,
  DesignSystemManifest,
  DirectEditApplyResponse,
  DirectEditInspectResponse,
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
  PublicTokenResponse,
  ScreenshotResponse,
  TweaksResponse,
  VerifyOrchestrateResponse,
  VerifyResponse
} from './types';

type ServerState = 'checking' | 'online' | 'offline';

type Notice = {
  kind: 'info' | 'error';
  text: string;
};

type ToolResult =
  | { kind: 'bundle'; value: BundleResponse }
  | { kind: 'handoff'; value: HandoffResponse }
  | { kind: 'ds'; value: DesignSystemManifest }
  | { kind: 'capabilities'; value: Capability[] }
  | { kind: 'eval'; value: EvalJsResponse }
  | { kind: 'screenshot'; value: ScreenshotResponse }
  | { kind: 'multi-shot'; value: MultiScreenshotResponse }
  | { kind: 'pptx'; value: PptxResponse }
  | { kind: 'pdf'; value: PdfResponse }
  | { kind: 'public'; value: PublicTokenResponse }
  | { kind: 'direct'; value: DirectEditResponse }
  | { kind: 'direct-inspect'; value: DirectEditInspectResponse }
  | { kind: 'direct-apply'; value: DirectEditApplyResponse }
  | { kind: 'tweaks'; value: TweaksResponse }
  | { kind: 'dc'; value: DcWriteResponse }
  | { kind: 'media'; value: GeneratedMediaResponse }
  | { kind: 'verify'; value: VerifyResponse }
  | { kind: 'verify-suite'; value: VerifyOrchestrateResponse }
  | { kind: 'chat'; value: ChatResponse };

type DirectSourceHint = {
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
};

const DEFAULT_FILE = `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <template id="__bundler_thumbnail">
      <div style="font: 600 22px system-ui; padding: 32px;">JFC Design</div>
    </template>
    <title>JFC Design</title>
    <style>
      :root { color-scheme: light; }
      body {
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        font-family: Inter, ui-sans-serif, system-ui, sans-serif;
        background: #f6f2ea;
        color: #182321;
      }
      main {
        width: min(840px, calc(100vw - 48px));
        padding: 40px;
        border: 1px solid #c9c2b7;
        background: #ffffff;
      }
      h1 { margin: 0 0 12px; font-size: 34px; letter-spacing: 0; }
      p { margin: 0; color: #47524f; line-height: 1.55; }
    </style>
  </head>
  <body>
    <main>
      <h1>JFC Design</h1>
      <p>Start editing the artifact source and refresh the preview.</p>
    </main>
  </body>
</html>
`;

function isPreviewable(path: string | null): boolean {
  if (!path) return false;
  return /\.(html?|svg|png|jpe?g|gif|webp|css|txt|json)$/i.test(path);
}

function isTextFile(file: FileReadResponse): boolean {
  return file.encoding === 'utf-8';
}

function extractDirectSourceHint(source: unknown): DirectSourceHint | null {
  if (!source || typeof source !== 'object') return null;
  const sourceRecord = source as {
    attributes?: unknown;
    react?: unknown;
    generated?: unknown;
  };
  const attrs =
    sourceRecord.attributes && typeof sourceRecord.attributes === 'object'
      ? (sourceRecord.attributes as Record<string, unknown>)
      : {};
  const react =
    sourceRecord.react && typeof sourceRecord.react === 'object'
      ? (sourceRecord.react as Record<string, unknown>)
      : {};
  const generated =
    'generated' in sourceRecord && sourceRecord.generated && typeof sourceRecord.generated === 'object'
      ? (sourceRecord.generated as Record<string, unknown>)
      : {};

  const pickString = (...keys: string[]) => {
    for (const key of keys) {
      const value = attrs[key] ?? react[key] ?? generated[key];
      if (typeof value === 'string' && value.trim()) return value.trim();
      if (typeof value === 'number' && Number.isFinite(value)) return String(value);
    }
    return undefined;
  };
  const pickNumber = (...keys: string[]) => {
    const raw = pickString(...keys);
    if (!raw) return undefined;
    const value = Number(raw);
    return Number.isSafeInteger(value) && value >= 0 ? value : undefined;
  };

  const parseLoc = (value: string | undefined) => {
    if (!value) return null;
    const match = value.match(/^(.*?)(?::(\d+))(?::(\d+))?$/);
    if (!match) return null;
    return {
      path: match[1],
      line: Number(match[2]),
      column: Number(match[3] ?? 0)
    };
  };
  const generatedLoc = parseLoc(pickString('data-src-loc', 'data-loc', 'generated'));

  const hint: DirectSourceHint = {
    source_path: pickString('data-source', 'data-src', 'data-src-file', 'data-om-source', 'data-om-path', 'fileName'),
    source_start: pickNumber('data-om-start', 'data-source-start', 'start'),
    source_end: pickNumber('data-om-end', 'data-source-end', 'end'),
    source_line: pickNumber('data-src-line', 'data-om-line', 'lineNumber', 'line'),
    source_column: pickNumber('data-src-column', 'data-om-column', 'columnNumber', 'column'),
    source_kind: pickString('data-om-kind', 'data-source-kind', 'kind'),
    generated_path: pickString('data-generated-path', 'generated_path', 'generatedPath', 'generatedFile', 'generatedFileName') ?? generatedLoc?.path,
    generated_line: pickNumber('data-generated-line', 'generated_line', 'generatedLine', 'generatedLineNumber') ?? generatedLoc?.line,
    generated_column: pickNumber('data-generated-column', 'generated_column', 'generatedColumn', 'generatedColumnNumber') ?? generatedLoc?.column,
    source_map_path: pickString('data-source-map', 'data-sourcemap', 'source_map_path', 'sourceMapPath')
  };
  if (
    !hint.source_path &&
    hint.source_start === undefined &&
    hint.source_end === undefined &&
    hint.source_line === undefined &&
    hint.generated_path === undefined
  ) {
    return null;
  }
  return hint;
}

function App() {
  const [serverState, setServerState] = useState<ServerState>('checking');
  const [projects, setProjects] = useState<ProjectMeta[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [selectedProject, setSelectedProject] = useState<ProjectResponse | null>(null);
  const [files, setFiles] = useState<string[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [openFile, setOpenFile] = useState<FileReadResponse | null>(null);
  const [draft, setDraft] = useState('');
  const [dirty, setDirty] = useState(false);
  const [newProjectTitle, setNewProjectTitle] = useState('Untitled design');
  const [newFilePath, setNewFilePath] = useState('index.html');
  const [assetName, setAssetName] = useState('');
  const [handoffName, setHandoffName] = useState('design-handoff');
  const [bundleOutput, setBundleOutput] = useState('');
  const [evalScript, setEvalScript] = useState('document.title');
  const [screenshotOutput, setScreenshotOutput] = useState('');
  const [multiShotOutput, setMultiShotOutput] = useState('');
  const [pptxOutput, setPptxOutput] = useState('');
  const [pptxSelector, setPptxSelector] = useState('[data-slide], .slide, section');
  const [pptxMode, setPptxMode] = useState<'editable' | 'screenshots'>('editable');
  const [directSelector, setDirectSelector] = useState('h1');
  const [directText, setDirectText] = useState('');
  const [directStyles, setDirectStyles] = useState('{"outline":"2px solid #2f6f62"}');
  const [directSourceHint, setDirectSourceHint] = useState<DirectSourceHint | null>(null);
  const [directOriginalText, setDirectOriginalText] = useState('');
  const [verifyOutput, setVerifyOutput] = useState('');
  const [pdfOutput, setPdfOutput] = useState('');
  const [verifySuiteOutput, setVerifySuiteOutput] = useState('');
  const [mediaPrompt, setMediaPrompt] = useState('Clean product UI texture with precise geometric accents');
  const [mediaProvider, setMediaProvider] = useState('auto');
  const [imageOutput, setImageOutput] = useState('');
  const [soundOutput, setSoundOutput] = useState('');
  const [chatInput, setChatInput] = useState('');
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([]);
  const [overridesDraft, setOverridesDraft] = useState('{}');
  const [tweaksDraft, setTweaksDraft] = useState('{}');
  const [dcPath, setDcPath] = useState('artifact.dc.html');
  const [dcContent, setDcContent] = useState('<!doctype html><div data-dc-root></div>');
  const [dcStreaming, setDcStreaming] = useState(false);
  const [events, setEvents] = useState<DesignEvent[]>([]);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [toolResult, setToolResult] = useState<ToolResult | null>(null);
  const [previewKey, setPreviewKey] = useState(0);

  const selectedMeta = selectedProject?.meta ?? null;
  const activePreviewUrl = useMemo(() => {
    if (!selectedProjectId || !activePath || !isPreviewable(activePath)) return null;
    return previewUrl(selectedProjectId, activePath);
  }, [activePath, selectedProjectId, previewKey]);

  const showNotice = useCallback((next: Notice) => {
    setNotice(next);
    window.setTimeout(() => {
      setNotice((current) => (current?.text === next.text ? null : current));
    }, 4500);
  }, []);

  const run = useCallback(
    async <T,>(action: () => Promise<T>, ok?: string): Promise<T | null> => {
      try {
        const value = await action();
        if (ok) showNotice({ kind: 'info', text: ok });
        return value;
      } catch (error) {
        showNotice({
          kind: 'error',
          text: error instanceof Error ? error.message : 'Request failed'
        });
        return null;
      }
    },
    [showNotice]
  );

  const refreshProjects = useCallback(async () => {
    const next = await run(() => designApi.listProjects());
    if (!next) return;
    setProjects(next);
    setServerState('online');
    if (!selectedProjectId && next[0]) setSelectedProjectId(next[0].id);
  }, [run, selectedProjectId]);

  const refreshSelected = useCallback(async () => {
    if (!selectedProjectId) {
      setSelectedProject(null);
      setFiles([]);
      return;
    }
    const [project, nextFiles] = await Promise.all([
      run(() => designApi.getProject(selectedProjectId)),
      run(() => designApi.listFiles(selectedProjectId))
    ]);
    if (project) setSelectedProject(project);
    if (nextFiles) {
      setFiles(nextFiles);
      setActivePath((current) => current ?? nextFiles.find((f) => f.endsWith('.html')) ?? nextFiles[0] ?? null);
    }
  }, [run, selectedProjectId]);

  useEffect(() => {
    designApi
      .health()
      .then(() => {
        setServerState('online');
        void refreshProjects();
      })
      .catch(() => setServerState('offline'));
  }, [refreshProjects]);

  useEffect(() => {
    void refreshSelected();
    setOpenFile(null);
    setDraft('');
    setDirty(false);
    setEvents([]);
    setDirectSourceHint(null);
    setDirectOriginalText('');
    setChatMessages([]);
    if (selectedProjectId) {
      void run(async () => {
        const history = await designApi.chatHistory(selectedProjectId);
        setChatMessages(history.messages);
        return history;
      });
    }
  }, [refreshSelected, run, selectedProjectId]);

  useEffect(() => {
    if (!selectedProjectId) return;
    const source = new EventSource(
      `/design/projects/${encodeURIComponent(selectedProjectId)}/events`
    );
    source.onmessage = (event) => {
      try {
        const parsed = JSON.parse(event.data) as DesignEvent;
        setEvents((current) => [parsed, ...current].slice(0, 24));
      } catch {
        // Ignore malformed server-sent data.
      }
    };
    ['project.created', 'project.updated', 'file.written', 'file.deleted', 'file.copied', 'asset.registered', 'asset.unregistered', 'public_share.created', 'public_share.revoked', 'tool.bundle_html', 'tool.handoff', 'tool.check_design_system', 'tool.eval_js', 'tool.screenshot', 'tool.multi_screenshot', 'tool.gen_pptx', 'tool.save_pdf', 'tool.direct_edit_inspect', 'tool.direct_edit_apply', 'tool.direct_edit_overrides', 'tool.verify', 'tool.verify_orchestrate', 'tool.chat', 'tool.tweaks', 'tool.dc_write', 'tool.generate_image', 'tool.generate_sound'].forEach((eventName) => {
      source.addEventListener(eventName, (event) => {
        try {
          const parsed = JSON.parse((event as MessageEvent).data) as DesignEvent;
          setEvents((current) => [parsed, ...current].slice(0, 24));
        } catch {
          // Ignore malformed server-sent data.
        }
      });
    });
    return () => source.close();
  }, [selectedProjectId]);

  useEffect(() => {
    if (!selectedProjectId || !activePath) return;
    void run(async () => {
      const file = await designApi.readFile(selectedProjectId, activePath);
      setOpenFile(file);
      setDraft(isTextFile(file) ? file.content : '');
      setDirty(false);
      setDirectSourceHint(null);
      setDirectOriginalText('');
      return file;
    });
  }, [activePath, run, selectedProjectId]);

  async function createProject() {
    const project = await run(
      () => designApi.createProject(newProjectTitle.trim() || 'Untitled design'),
      'Project created'
    );
    if (!project) return;
    await designApi.writeFile(project.meta.id, 'index.html', DEFAULT_FILE, 'Starter artifact');
    setSelectedProjectId(project.meta.id);
    setActivePath('index.html');
    await refreshProjects();
  }

  async function createFile() {
    if (!selectedProjectId || !newFilePath.trim()) return;
    const path = newFilePath.trim();
    await run(
      () => designApi.writeFile(selectedProjectId, path, path.endsWith('.html') ? DEFAULT_FILE : ''),
      'File created'
    );
    setActivePath(path);
    await refreshSelected();
  }

  async function saveFile() {
    if (!selectedProjectId || !activePath || !openFile || !isTextFile(openFile)) return;
    const updated = await run(
      () => designApi.writeFile(selectedProjectId, activePath, draft),
      'Saved'
    );
    if (!updated) return;
    setSelectedProject(updated);
    setDirty(false);
    await refreshSelected();
    setPreviewKey((value) => value + 1);
  }

  async function deleteActiveFile() {
    if (!selectedProjectId || !activePath) return;
    const deleted = await run(
      () => designApi.deleteFile(selectedProjectId, activePath),
      'Deleted'
    );
    if (!deleted) return;
    setSelectedProject(deleted);
    setActivePath(null);
    await refreshSelected();
  }

  async function registerCurrentAsset() {
    if (!selectedProjectId || !activePath) return;
    const name = assetName.trim() || activePath.split('/').pop() || activePath;
    const updated = await run(
      () => designApi.registerAsset(selectedProjectId, name, activePath),
      'Asset registered'
    );
    if (updated) setSelectedProject(updated);
  }

  async function unregisterAsset(asset: Asset) {
    if (!selectedProjectId) return;
    const updated = await run(
      () => designApi.unregisterAsset(selectedProjectId, asset.path),
      'Asset removed'
    );
    if (updated) setSelectedProject(updated);
  }

  async function bundleActive() {
    if (!selectedProjectId || !activePath) return;
    const result = await run(() =>
      designApi.bundleHtml(selectedProjectId, activePath, bundleOutput.trim() || undefined)
    );
    if (!result) return;
    setToolResult({ kind: 'bundle', value: result });
    await refreshSelected();
  }

  async function createHandoff() {
    if (!selectedProjectId) return;
    const result = await run(() =>
      designApi.handoff(selectedProjectId, handoffName.trim() || 'design-handoff', files)
    );
    if (result) setToolResult({ kind: 'handoff', value: result });
  }

  async function checkDesignSystem() {
    if (!selectedProjectId) return;
    const result = await run(() => designApi.checkDesignSystem(selectedProjectId));
    if (!result) return;
    setToolResult({ kind: 'ds', value: result });
    await refreshSelected();
  }

  async function loadCapabilities() {
    const result = await run(() => designApi.capabilities());
    if (result) setToolResult({ kind: 'capabilities', value: result.matrix });
  }

  async function evalActive() {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const result = await run(() => designApi.evalJs(selectedProjectId, activePath, evalScript));
    if (result) setToolResult({ kind: 'eval', value: result });
  }

  async function screenshotActive() {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const result = await run(() =>
      designApi.screenshot(selectedProjectId, activePath, screenshotOutput.trim() || undefined)
    );
    if (!result) return;
    setToolResult({ kind: 'screenshot', value: result });
    await refreshSelected();
  }

  async function multiScreenshotActive() {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const result = await run(() =>
      designApi.multiScreenshot(
        selectedProjectId,
        activePath,
        multiShotOutput.trim() || undefined,
        pptxSelector.trim() || undefined
      )
    );
    if (!result) return;
    setToolResult({ kind: 'multi-shot', value: result });
    await refreshSelected();
  }

  async function pptxActive() {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const result = await run(() =>
      designApi.genPptx(
        selectedProjectId,
        activePath,
        pptxOutput.trim() || undefined,
        pptxSelector.trim() || undefined,
        pptxMode
      )
    );
    if (!result) return;
    setToolResult({ kind: 'pptx', value: result });
    await refreshSelected();
  }

  async function pdfActive() {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const result = await run(() =>
      designApi.savePdf(selectedProjectId, activePath, pdfOutput.trim() || undefined)
    );
    if (!result) return;
    setToolResult({ kind: 'pdf', value: result });
    await refreshSelected();
  }

  async function printActive() {
    if (!selectedProjectId || !activePath) return;
    window.open(printUrl(selectedProjectId, activePath), '_blank');
  }

  async function inspectDirectEdit() {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const result = await run(() =>
      designApi.directEditInspect(selectedProjectId, activePath, directSelector.trim() || 'body')
    );
    if (!result) return;
    setToolResult({ kind: 'direct-inspect', value: result });
    setDirectSourceHint(extractDirectSourceHint(result.source));
    setDirectOriginalText(result.text ?? '');
    if (result.text && !directText) setDirectText(result.text.slice(0, 240));
    if (result.selector) setDirectSelector(result.selector);
  }

  async function applyDirectEdit() {
    if (!selectedProjectId || !activePath) return;
    const parsedStyles = parseJsonObject(directStyles, 'Direct-edit styles');
    if (!parsedStyles.ok) {
      showNotice({ kind: 'error', text: parsedStyles.error });
      return;
    }
    const result = await run(() =>
      designApi.directEditApply(selectedProjectId, activePath, directSelector.trim() || 'body', {
        text: directText || undefined,
        styles: parsedStyles.value,
        fallback_overlay: true,
        previous_text: directOriginalText || undefined,
        ...(directSourceHint ?? {})
      })
    );
    if (!result) return;
    setToolResult({ kind: 'direct-apply', value: result });
    await refreshSelected();
    setPreviewKey((value) => value + 1);
  }

  async function verifyActive() {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const result = await run(() =>
      designApi.verify(
        selectedProjectId,
        activePath,
        verifyOutput.trim() || undefined,
        pptxSelector.trim() || undefined
      )
    );
    if (!result) return;
    setToolResult({ kind: 'verify', value: result });
    await refreshSelected();
  }

  async function verifySuiteActive(done = false) {
    if (!selectedProjectId || !activePath) return;
    if (dirty) await saveFile();
    const runVerify = done ? designApi.done : designApi.verifyOrchestrate;
    const result = await run(() =>
      runVerify(
        selectedProjectId,
        activePath,
        verifySuiteOutput.trim() || undefined,
        pptxSelector.trim() || undefined
      )
    );
    if (!result) return;
    setToolResult({ kind: 'verify-suite', value: result });
    await refreshSelected();
  }

  async function makePublicToken() {
    if (!selectedProjectId || !activePath) return;
    const result = await run(() => designApi.publicToken(selectedProjectId, activePath));
    if (result) setToolResult({ kind: 'public', value: result });
  }

  async function generateImageAsset() {
    if (!selectedProjectId) return;
    const prompt = mediaPrompt.trim();
    if (!prompt) {
      showNotice({ kind: 'error', text: 'Media prompt is required' });
      return;
    }
    const result = await run(() =>
      designApi.generateImage(
        selectedProjectId,
        prompt,
        imageOutput.trim() || undefined,
        1280,
        720,
        'product',
        mediaProvider
      )
    );
    if (!result) return;
    setToolResult({ kind: 'media', value: result });
    setActivePath(result.output);
    await refreshSelected();
  }

  async function generateSoundAsset() {
    if (!selectedProjectId) return;
    const prompt = mediaPrompt.trim();
    if (!prompt) {
      showNotice({ kind: 'error', text: 'Media prompt is required' });
      return;
    }
    const result = await run(() =>
      designApi.generateSound(
        selectedProjectId,
        prompt,
        soundOutput.trim() || undefined,
        1800,
        mediaProvider
      )
    );
    if (!result) return;
    setToolResult({ kind: 'media', value: result });
    setActivePath(result.output);
    await refreshSelected();
  }

  async function downloadActive() {
    if (!selectedProjectId || !activePath) return;
    window.open(downloadUrl(selectedProjectId, activePath), '_blank');
  }

  async function saveDirectEditOverrides() {
    if (!selectedProjectId) return;
    const parsed = parseJsonObject(overridesDraft, 'Direct-edit overrides');
    if (!parsed.ok) {
      showNotice({ kind: 'error', text: parsed.error });
      return;
    }
    const result = await run(() => designApi.writeDirectEditOverrides(selectedProjectId, parsed.value));
    if (result) setToolResult({ kind: 'direct', value: result });
  }

  async function saveTweaks() {
    if (!selectedProjectId) return;
    const parsed = parseJsonObject(tweaksDraft, 'Tweaks');
    if (!parsed.ok) {
      showNotice({ kind: 'error', text: parsed.error });
      return;
    }
    const result = await run(() => designApi.writeTweaks(selectedProjectId, parsed.value));
    if (result) setToolResult({ kind: 'tweaks', value: result });
  }

  async function writeDc() {
    if (!selectedProjectId) return;
    const result = await run(() =>
      designApi.dcWrite(
        selectedProjectId,
        dcPath,
        dcContent,
        false,
        dcStreaming,
        dcPath.split('/').pop(),
        dcPath.endsWith('.dc.html') ? 'component' : 'html'
      )
    );
    if (!result) return;
    setToolResult({ kind: 'dc', value: result });
    setActivePath(result.path);
    await refreshSelected();
  }

  async function sendChat() {
    if (!selectedProjectId) return;
    const message = chatInput.trim();
    if (!message) return;
    setChatInput('');
    const result = await run(() => designApi.chat(selectedProjectId, message, activePath ?? undefined));
    if (!result) return;
    setChatMessages(result.messages);
    setToolResult({ kind: 'chat', value: result });
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand">
          <LayoutDashboard size={19} />
          <span>JFC Design</span>
        </div>
        <div className="status-line">
          <ServerBadge state={serverState} />
          {notice ? <NoticeView notice={notice} /> : null}
        </div>
        <div className="topbar-actions">
          <button className="icon-button" title="Refresh" onClick={() => void refreshProjects()}>
            <RefreshCcw size={17} />
          </button>
          <button className="icon-button" title="Capabilities" onClick={() => void loadCapabilities()}>
            <SearchCheck size={17} />
          </button>
        </div>
      </header>

      <section className="workspace">
        <aside className="sidebar">
          <section className="panel-block">
            <div className="panel-heading">
              <span>Projects</span>
              <FolderTree size={16} />
            </div>
            <div className="inline-create">
              <input
                value={newProjectTitle}
                onChange={(event) => setNewProjectTitle(event.target.value)}
                aria-label="Project title"
              />
              <button className="icon-button filled" title="Create project" onClick={() => void createProject()}>
                <Plus size={17} />
              </button>
            </div>
            <div className="nav-list">
              {projects.map((project) => (
                <button
                  key={project.id}
                  className={project.id === selectedProjectId ? 'nav-item active' : 'nav-item'}
                  onClick={() => setSelectedProjectId(project.id)}
                >
                  <span>{project.title}</span>
                  <small>{project.assets.length}</small>
                </button>
              ))}
            </div>
          </section>

          <section className="panel-block files-block">
            <div className="panel-heading">
              <span>Files</span>
              <FileCode2 size={16} />
            </div>
            <div className="inline-create">
              <input
                value={newFilePath}
                onChange={(event) => setNewFilePath(event.target.value)}
                aria-label="File path"
              />
              <button className="icon-button" title="Create file" onClick={() => void createFile()} disabled={!selectedProjectId}>
                <FilePlus2 size={17} />
              </button>
            </div>
            <div className="file-list">
              {files.map((file) => (
                <button
                  key={file}
                  className={file === activePath ? 'file-item active' : 'file-item'}
                  onClick={() => setActivePath(file)}
                >
                  <span>{file}</span>
                </button>
              ))}
            </div>
          </section>
        </aside>

        <section className="editor-pane">
          <div className="pane-toolbar">
            <div className="path-title">
              <span>{activePath ?? 'No file selected'}</span>
              {dirty ? <small>modified</small> : null}
            </div>
            <div className="toolbar-actions">
              <button className="icon-button" title="Save" onClick={() => void saveFile()} disabled={!dirty}>
                <Save size={17} />
              </button>
              <button className="icon-button" title="Register asset" onClick={() => void registerCurrentAsset()} disabled={!activePath}>
                <PackageCheck size={17} />
              </button>
              <button className="icon-button danger" title="Delete file" onClick={() => void deleteActiveFile()} disabled={!activePath}>
                <Trash2 size={17} />
              </button>
            </div>
          </div>
          <textarea
            value={draft}
            disabled={!openFile || !isTextFile(openFile)}
            onChange={(event) => {
              setDraft(event.target.value);
              setDirty(true);
            }}
            spellCheck={false}
            aria-label="Source"
          />
        </section>

        <section className="preview-pane">
          <div className="pane-toolbar">
            <div className="path-title">
              <span>Preview</span>
              {selectedMeta?.is_design_system ? <small>design system</small> : null}
            </div>
            <div className="toolbar-actions">
              <button className="icon-button" title="Refresh preview" onClick={() => setPreviewKey((value) => value + 1)}>
                <RefreshCcw size={17} />
              </button>
              <button className="icon-button" title="Open preview" disabled={!activePreviewUrl} onClick={() => activePreviewUrl && window.open(activePreviewUrl, '_blank')}>
                <Globe2 size={17} />
              </button>
            </div>
          </div>
          <div className="preview-frame">
            {activePreviewUrl ? (
              <iframe key={`${activePreviewUrl}:${previewKey}`} src={activePreviewUrl} title="Design preview" />
            ) : (
              <div className="empty-state">
                <Play size={24} />
                <span>{selectedProjectId ? 'Select a previewable file' : 'Start the design server'}</span>
              </div>
            )}
          </div>
        </section>

        <aside className="right-rail">
          <section className="panel-block">
            <div className="panel-heading">
              <span>Meta</span>
              <Settings2 size={16} />
            </div>
            <ProjectMetaPanel
              project={selectedProject}
              onUpdate={async (updates) => {
                if (!selectedProjectId) return;
                const updated = await run(() => designApi.updateProject(selectedProjectId, updates), 'Updated');
                if (updated) setSelectedProject(updated);
                await refreshProjects();
              }}
            />
          </section>

          <section className="panel-block chat-block">
            <div className="panel-heading">
              <span>Chat</span>
              <MessageSquare size={16} />
            </div>
            <div className="chat-list">
              {chatMessages.slice(-8).map((message) => (
                <div className={`chat-message ${message.role}`} key={`${message.ts_ms}:${message.role}:${message.content}`}>
                  <span>{message.content}</span>
                  {message.path ? <small>{message.path}</small> : null}
                </div>
              ))}
            </div>
            <div className="chat-input">
              <input
                value={chatInput}
                onChange={(event) => setChatInput(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') void sendChat();
                }}
                aria-label="Design chat"
              />
              <button className="icon-button filled" title="Send" onClick={() => void sendChat()} disabled={!selectedProjectId}>
                <Send size={16} />
              </button>
            </div>
          </section>

          <section className="panel-block">
            <div className="panel-heading">
              <span>Assets</span>
              <PackageCheck size={16} />
            </div>
            <div className="inline-create">
              <input value={assetName} onChange={(event) => setAssetName(event.target.value)} aria-label="Asset name" />
              <button className="icon-button" title="Register asset" onClick={() => void registerCurrentAsset()} disabled={!activePath}>
                <Plus size={17} />
              </button>
            </div>
            <div className="asset-list">
              {(selectedMeta?.assets ?? []).map((asset) => (
                <div className="asset-item" key={asset.path}>
                  <button className="asset-main" onClick={() => setActivePath(asset.path)}>
                    <span>{asset.name}</span>
                    <small>{asset.path}</small>
                  </button>
                  <button className="icon-button compact" title="Remove asset" onClick={() => void unregisterAsset(asset)}>
                    <XCircle size={16} />
                  </button>
                </div>
              ))}
            </div>
          </section>

          <section className="panel-block">
            <div className="panel-heading">
              <span>Tools</span>
              <Download size={16} />
            </div>
            <input
              value={bundleOutput}
              onChange={(event) => setBundleOutput(event.target.value)}
              aria-label="Bundle output"
              placeholder="standalone output"
            />
            <div className="tool-row">
              <button className="command-button" onClick={() => void bundleActive()} disabled={!activePath}>
                <Download size={16} />
                <span>Bundle</span>
              </button>
              <button className="command-button" onClick={() => void checkDesignSystem()} disabled={!selectedProjectId}>
                <CheckCircle2 size={16} />
                <span>DS</span>
              </button>
            </div>
            <input
              value={handoffName}
              onChange={(event) => setHandoffName(event.target.value)}
              aria-label="Handoff name"
            />
            <button className="command-button full" onClick={() => void createHandoff()} disabled={!selectedProjectId}>
              <Copy size={16} />
              <span>Handoff</span>
            </button>
            <div className="tool-divider" />
            <textarea
              className="mini-editor"
              value={evalScript}
              onChange={(event) => setEvalScript(event.target.value)}
              aria-label="Eval JavaScript"
            />
            <div className="tool-row">
              <button className="command-button" onClick={() => void evalActive()} disabled={!activePath}>
                <Play size={16} />
                <span>Eval</span>
              </button>
              <button className="command-button" onClick={() => void screenshotActive()} disabled={!activePath}>
                <FileCode2 size={16} />
                <span>Shot</span>
              </button>
            </div>
            <input
              value={screenshotOutput}
              onChange={(event) => setScreenshotOutput(event.target.value)}
              aria-label="Screenshot output"
              placeholder="screenshots/page.png"
            />
            <input
              value={multiShotOutput}
              onChange={(event) => setMultiShotOutput(event.target.value)}
              aria-label="Multi screenshot output directory"
              placeholder="screenshots/deck"
            />
            <input
              value={pptxOutput}
              onChange={(event) => setPptxOutput(event.target.value)}
              aria-label="PPTX output"
              placeholder="exports/deck.pptx"
            />
            <input
              value={pdfOutput}
              onChange={(event) => setPdfOutput(event.target.value)}
              aria-label="PDF output"
              placeholder="exports/page.pdf"
            />
            <select
              value={pptxMode}
              onChange={(event) => setPptxMode(event.target.value as 'editable' | 'screenshots')}
              aria-label="PPTX mode"
            >
              <option value="editable">Editable PPTX</option>
              <option value="screenshots">Screenshot PPTX</option>
            </select>
            <input
              value={pptxSelector}
              onChange={(event) => setPptxSelector(event.target.value)}
              aria-label="PPTX slide selector"
            />
            <div className="tool-row">
              <button className="command-button" onClick={() => void pptxActive()} disabled={!activePath}>
                <Download size={16} />
                <span>PPTX</span>
              </button>
              <button className="command-button" onClick={() => void multiScreenshotActive()} disabled={!activePath}>
                <FileCode2 size={16} />
                <span>Multi</span>
              </button>
            </div>
            <div className="tool-row">
              <button className="command-button" onClick={() => void pdfActive()} disabled={!activePath}>
                <Printer size={16} />
                <span>PDF</span>
              </button>
              <button className="command-button" onClick={() => void printActive()} disabled={!activePath}>
                <Printer size={16} />
                <span>Print</span>
              </button>
            </div>
            <input
              value={verifyOutput}
              onChange={(event) => setVerifyOutput(event.target.value)}
              aria-label="Verifier screenshot output"
              placeholder="verifier/page.png"
            />
            <input
              value={verifySuiteOutput}
              onChange={(event) => setVerifySuiteOutput(event.target.value)}
              aria-label="Verifier output directory"
              placeholder="verifier/page"
            />
            <div className="tool-row">
              <button className="command-button" onClick={() => void verifyActive()} disabled={!activePath}>
                <SearchCheck size={16} />
                <span>Verify</span>
              </button>
              <button className="command-button" onClick={() => void verifySuiteActive()} disabled={!activePath}>
                <SearchCheck size={16} />
                <span>Suite</span>
              </button>
            </div>
            <div className="tool-row">
              <button className="command-button" onClick={() => void verifySuiteActive(true)} disabled={!activePath}>
                <CheckCircle2 size={16} />
                <span>Done</span>
              </button>
              <button className="command-button" onClick={() => void makePublicToken()} disabled={!activePath}>
                <Globe2 size={16} />
                <span>Link</span>
              </button>
            </div>
            <button className="command-button full" onClick={() => void downloadActive()} disabled={!activePath}>
              <Download size={16} />
              <span>Download File</span>
            </button>
          </section>

          <section className="panel-block">
            <div className="panel-heading">
              <span>Media</span>
              <Image size={16} />
            </div>
            <textarea
              className="mini-editor"
              value={mediaPrompt}
              onChange={(event) => setMediaPrompt(event.target.value)}
              aria-label="Media prompt"
            />
            <select
              value={mediaProvider}
              onChange={(event) => setMediaProvider(event.target.value)}
              aria-label="Media provider"
            >
              <option value="auto">Auto</option>
              <option value="local">Local</option>
              <option value="external">External</option>
              <option value="gemini">Gemini</option>
              <option value="imagen">Imagen</option>
              <option value="elevenlabs">ElevenLabs</option>
            </select>
            <input
              value={imageOutput}
              onChange={(event) => setImageOutput(event.target.value)}
              aria-label="Generated image output"
              placeholder="generated/hero.svg"
            />
            <input
              value={soundOutput}
              onChange={(event) => setSoundOutput(event.target.value)}
              aria-label="Generated sound output"
              placeholder="generated/tone.wav"
            />
            <div className="tool-row">
              <button className="command-button" onClick={() => void generateImageAsset()} disabled={!selectedProjectId}>
                <Image size={16} />
                <span>Image</span>
              </button>
              <button className="command-button" onClick={() => void generateSoundAsset()} disabled={!selectedProjectId}>
                <Music2 size={16} />
                <span>Sound</span>
              </button>
            </div>
          </section>

          <section className="panel-block">
            <div className="panel-heading">
              <span>Host State</span>
              <SlidersHorizontal size={16} />
            </div>
            <input
              value={directSelector}
              onChange={(event) => setDirectSelector(event.target.value)}
              aria-label="Direct edit selector"
              placeholder="h1"
            />
            <textarea
              className="mini-editor"
              value={directText}
              onChange={(event) => setDirectText(event.target.value)}
              aria-label="Direct edit text"
              placeholder="replacement text"
            />
            <textarea
              className="mini-editor"
              value={directStyles}
              onChange={(event) => setDirectStyles(event.target.value)}
              aria-label="Direct edit styles JSON"
            />
            <div className="tool-row">
              <button className="command-button" onClick={() => void inspectDirectEdit()} disabled={!activePath}>
                <SearchCheck size={16} />
                <span>Inspect</span>
              </button>
              <button className="command-button" onClick={() => void applyDirectEdit()} disabled={!activePath}>
                <Save size={16} />
                <span>Apply</span>
              </button>
            </div>
            <div className="tool-divider" />
            <textarea
              className="mini-editor"
              value={overridesDraft}
              onChange={(event) => setOverridesDraft(event.target.value)}
              aria-label="Direct edit overrides JSON"
            />
            <button className="command-button full" onClick={() => void saveDirectEditOverrides()} disabled={!selectedProjectId}>
              <FileJson2 size={16} />
              <span>Save Overrides</span>
            </button>
            <textarea
              className="mini-editor"
              value={tweaksDraft}
              onChange={(event) => setTweaksDraft(event.target.value)}
              aria-label="Tweaks JSON"
            />
            <button className="command-button full" onClick={() => void saveTweaks()} disabled={!selectedProjectId}>
              <Settings2 size={16} />
              <span>Save Tweaks</span>
            </button>
            <input value={dcPath} onChange={(event) => setDcPath(event.target.value)} aria-label="Design component path" />
            <textarea
              className="mini-editor"
              value={dcContent}
              onChange={(event) => setDcContent(event.target.value)}
              aria-label="Design component content"
            />
            <label className="small-check">
              <input
                type="checkbox"
                checked={dcStreaming}
                onChange={(event) => setDcStreaming(event.target.checked)}
              />
              <span>Streaming write</span>
            </label>
            <button className="command-button full" onClick={() => void writeDc()} disabled={!selectedProjectId}>
              <FileCode2 size={16} />
              <span>DC Write</span>
            </button>
          </section>

          <section className="panel-block output-block">
            <div className="panel-heading">
              <span>Output</span>
              <FileCode2 size={16} />
            </div>
            <ToolOutput result={toolResult} />
          </section>

          <section className="panel-block events-block">
            <div className="panel-heading">
              <span>Events</span>
              <RefreshCcw size={16} />
            </div>
            <div className="event-list">
              {events.map((event) => (
                <div className="event-item" key={event.id}>
                  <span>{event.kind}</span>
                  <small>{event.path ?? event.project_id}</small>
                </div>
              ))}
            </div>
          </section>
        </aside>
      </section>
    </main>
  );
}

function ServerBadge({ state }: { state: ServerState }) {
  return (
    <div className={`server-badge ${state}`}>
      {state === 'online' ? <CheckCircle2 size={15} /> : <XCircle size={15} />}
      <span>{state}</span>
    </div>
  );
}

function NoticeView({ notice }: { notice: Notice }) {
  return <div className={`notice ${notice.kind}`}>{notice.text}</div>;
}

function ProjectMetaPanel({
  project,
  onUpdate
}: {
  project: ProjectResponse | null;
  onUpdate: (updates: { title?: string; is_design_system?: boolean }) => Promise<void>;
}) {
  const [title, setTitle] = useState('');

  useEffect(() => {
    setTitle(project?.meta.title ?? '');
  }, [project?.meta.title]);

  if (!project) return <div className="empty-line">No project</div>;

  return (
    <div className="meta-form">
      <input value={title} onChange={(event) => setTitle(event.target.value)} aria-label="Title" />
      <div className="toggle-row">
        <label>
          <input
            type="checkbox"
            checked={project.meta.is_design_system}
            onChange={(event) => void onUpdate({ is_design_system: event.target.checked })}
          />
          <span>Design system</span>
        </label>
        <button className="icon-button" title="Save title" onClick={() => void onUpdate({ title })}>
          <Save size={16} />
        </button>
      </div>
      <small className="mono-path">{project.meta.id}</small>
      <small className="mono-path">{project.root}</small>
    </div>
  );
}

function ToolOutput({ result }: { result: ToolResult | null }) {
  if (!result) return <div className="empty-line">No output</div>;

  if (result.kind === 'bundle') {
    return (
      <div className="tool-output">
        <strong>{result.value.output}</strong>
        <span>{Math.ceil(result.value.bytes / 1024)} KB</span>
        {result.value.misses.map((miss) => (
          <small key={miss}>{miss}</small>
        ))}
      </div>
    );
  }

  if (result.kind === 'handoff') {
    return (
      <div className="tool-output">
        <strong>{result.value.dir}</strong>
        <span>{result.value.readme}</span>
        <small>{result.value.copied.length} copied</small>
      </div>
    );
  }

  if (result.kind === 'ds') {
    return (
      <div className="tool-output">
        <strong>window.{result.value.namespace}</strong>
        <span>{result.value.components.length} components</span>
        <span>{result.value.tokens.length} tokens</span>
        {result.value.issues.map((issue) => (
          <small key={issue}>{issue}</small>
        ))}
      </div>
    );
  }

  if (result.kind === 'eval') {
    return (
      <div className="tool-output">
        <strong>{result.value.ok ? 'eval ok' : 'eval error'}</strong>
        <span>{result.value.duration_ms} ms</span>
        <small>{JSON.stringify(result.value.result)}</small>
        <small>{result.value.logs.length} logs · {result.value.errors.length} errors</small>
      </div>
    );
  }

  if (result.kind === 'screenshot') {
    return (
      <div className="tool-output">
        <strong>{result.value.output}</strong>
        <span>{Math.ceil(result.value.bytes / 1024)} KB</span>
        <img className="shot-preview" alt="Screenshot preview" src={`data:image/png;base64,${result.value.data_base64}`} />
        <small>{result.value.logs.length} logs · {result.value.errors.length} errors</small>
      </div>
    );
  }

  if (result.kind === 'multi-shot') {
    return (
      <div className="tool-output">
        <strong>{result.value.output}</strong>
        <span>{result.value.screenshots.length} captures</span>
        {result.value.screenshots.slice(0, 4).map((shot) => (
          <small key={shot.hash}>{shot.output ?? shot.hash}</small>
        ))}
        <small>{result.value.logs.length} logs · {result.value.errors.length} errors</small>
      </div>
    );
  }

  if (result.kind === 'pptx') {
    return (
      <div className="tool-output">
        <strong>{result.value.output}</strong>
        <span>{result.value.slides} slides · {result.value.mode}</span>
        <small>{Math.ceil(result.value.bytes / 1024)} KB</small>
        {result.value.validation?.editable_elements !== undefined ? (
          <small>{result.value.validation.editable_elements} editable elements</small>
        ) : null}
        {(result.value.warnings ?? []).map((warning) => (
          <small key={warning}>{warning}</small>
        ))}
      </div>
    );
  }

  if (result.kind === 'pdf') {
    return (
      <div className="tool-output">
        <strong>{result.value.output}</strong>
        <span>{Math.ceil(result.value.bytes / 1024)} KB</span>
        <small>{result.value.logs.length} logs · {result.value.errors.length} errors</small>
      </div>
    );
  }

  if (result.kind === 'public') {
    return (
      <div className="tool-output">
        <strong>{result.value.path}</strong>
        <span>{result.value.public_url}</span>
        <small>{result.value.embed_url}</small>
        <small>expires {new Date(result.value.expires_at_ms).toLocaleString()}</small>
        <small>{result.value.token}</small>
      </div>
    );
  }

  if (result.kind === 'direct') {
    return (
      <div className="tool-output">
        <strong>{result.value.path}</strong>
        <small>{JSON.stringify(result.value.overrides)}</small>
      </div>
    );
  }

  if (result.kind === 'direct-inspect') {
    return (
      <div className="tool-output">
        <strong>{result.value.selector ?? 'not found'}</strong>
        <span>{result.value.tag ?? 'element'}</span>
        <small>{result.value.text?.slice(0, 160) ?? ''}</small>
        {result.value.selectors?.slice(0, 4).map((selector) => (
          <small key={selector}>{selector}</small>
        ))}
      </div>
    );
  }

  if (result.kind === 'direct-apply') {
    return (
      <div className="tool-output">
        <strong>{result.value.selector}</strong>
        <span>{result.value.source_rewritten ? 'source rewrite' : result.value.runtime}</span>
        <small>{result.value.path} · {result.value.bytes} bytes</small>
        {result.value.source_path ? <small>{result.value.source_path}</small> : null}
        {result.value.fallback_reason ? <small>{result.value.fallback_reason}</small> : null}
        <small>{result.value.overrides_path}</small>
      </div>
    );
  }

  if (result.kind === 'tweaks') {
    return (
      <div className="tool-output">
        <strong>{result.value.path}</strong>
        <small>{JSON.stringify(result.value.values)}</small>
      </div>
    );
  }

  if (result.kind === 'dc') {
    return (
      <div className="tool-output">
        <strong>{result.value.path}</strong>
        <span>{result.value.bytes} bytes</span>
        <small>{result.value.streaming ? 'streaming' : result.value.appended ? 'appended' : 'written'}</small>
        {result.value.name ? <small>{result.value.name}</small> : null}
      </div>
    );
  }

  if (result.kind === 'verify') {
    return (
      <div className="tool-output">
        <strong>{result.value.ok ? 'verified' : 'needs attention'}</strong>
        <span>{result.value.output}</span>
        {result.value.screenshot.data_base64 ? (
          <img className="shot-preview" alt="Verifier screenshot" src={`data:image/png;base64,${result.value.screenshot.data_base64}`} />
        ) : null}
        {result.value.checks.map((check) => (
          <small key={check.name}>{check.status} · {check.name} · {check.detail}</small>
        ))}
      </div>
    );
  }

  if (result.kind === 'verify-suite') {
    return (
      <div className="tool-output">
        <strong>{result.value.verdict ?? (result.value.ok ? 'ready' : 'needs attention')}</strong>
        <span>{result.value.output_dir}</span>
        {result.value.runs.map((run) => (
          <small key={run.name}>{run.name} · {run.result.ok ? 'pass' : 'fail'} · {run.viewport.width}x{run.viewport.height}</small>
        ))}
        {result.value.warnings.slice(0, 6).map((warning) => (
          <small key={warning}>{warning}</small>
        ))}
      </div>
    );
  }

  if (result.kind === 'chat') {
    return (
      <div className="tool-output">
        <strong>{result.value.path}</strong>
        <span>{result.value.reply}</span>
        {result.value.actions.map((action) => (
          <small key={action}>{action}</small>
        ))}
      </div>
    );
  }

  if (result.kind === 'media') {
    return (
      <div className="tool-output">
        <strong>{result.value.output}</strong>
        <span>{result.value.provider} · {result.value.mime}</span>
        <small>{Math.ceil(result.value.bytes / 1024)} KB</small>
        <small>{result.value.prompt}</small>
      </div>
    );
  }

  return (
    <div className="tool-output">
      {result.value.map((capability) => (
        <small key={`${capability.feature}:${capability.surface}`}>
          {capability.status} · {capability.feature}
        </small>
      ))}
    </div>
  );
}

function parseJsonObject(text: string, label: string): { ok: true; value: unknown } | { ok: false; error: string } {
  try {
    return { ok: true, value: JSON.parse(text) as unknown };
  } catch (error) {
    return {
      ok: false,
      error: `${label} JSON is invalid: ${error instanceof Error ? error.message : 'parse failed'}`
    };
  }
}

export default App;
