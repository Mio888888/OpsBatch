import { lazy, Suspense, useCallback, useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { ask } from '@tauri-apps/plugin-dialog';
import { useEditorStore } from '../../stores/editor';
import WindowControls from '../../components/WindowControls';
import FileTree from './FileTree';
import '../../styles/windows/editor.css';

const LazyCodeEditor = lazy(() => import('../../components/CodeEditor'));

function guessLanguage(filename: string | null): string {
  if (!filename) return 'shell';
  const ext = filename.split('.').pop()?.toLowerCase() || '';
  const map: Record<string, string> = {
    sh: 'shell', bash: 'shell', zsh: 'shell', fish: 'shell',
    py: 'python',
    js: 'javascript', ts: 'typescript', jsx: 'javascript', tsx: 'typescript',
    json: 'json', html: 'html', css: 'css', scss: 'scss',
    yaml: 'yaml', yml: 'yaml', toml: 'toml',
    md: 'markdown', sql: 'sql', xml: 'xml', csv: 'csv',
    rs: 'rust', go: 'go', java: 'java', c: 'c', cpp: 'cpp',
  };
  return map[ext] || 'shell';
}

function getBaseName(path: string): string {
  return path.replace(/\/$/, '').split('/').pop() || path;
}

function EditorContentSkeleton({ label }: { label: string }) {
  return (
    <div className="editor-content-skeleton" role="status" aria-label={label}>
      <div className="editor-content-skeleton-header">
        <span className="editor-skeleton-dot" />
        <span className="editor-skeleton-label">{label}</span>
      </div>
      <div className="editor-content-skeleton-code">
        <span className="editor-skeleton-line editor-skeleton-line-wide" />
        <span className="editor-skeleton-line editor-skeleton-line-medium" />
        <span className="editor-skeleton-line editor-skeleton-line-full" />
        <span className="editor-skeleton-line editor-skeleton-line-narrow" />
        <span className="editor-skeleton-line editor-skeleton-line-wide" />
        <span className="editor-skeleton-line editor-skeleton-line-medium" />
        <span className="editor-skeleton-line editor-skeleton-line-full" />
      </div>
    </div>
  );
}

export default function EditorWindow() {
  const [searchParams] = useSearchParams();

  const hostId = searchParams.get('hostId') || '';
  const requestedMode = searchParams.get('mode') === 'dir' ? 'dir' : 'file';
  const path = searchParams.get('path') || '';

  const fileName = useEditorStore((state) => state.fileName);
  const filePath = useEditorStore((state) => state.filePath);
  const dirPath = useEditorStore((state) => state.dirPath);
  const content = useEditorStore((state) => state.content);
  const loading = useEditorStore((state) => state.loading);
  const saving = useEditorStore((state) => state.saving);
  const dirty = useEditorStore((state) => state.dirty);
  const tree = useEditorStore((state) => state.tree);
  const treeLoading = useEditorStore((state) => state.treeLoading);
  const currentFilePath = useEditorStore((state) => state.currentFilePath);
  const currentFileName = useEditorStore((state) => state.currentFileName);
  const error = useEditorStore((state) => state.error);
  const initDir = useEditorStore((state) => state.initDir);
  const initFile = useEditorStore((state) => state.initFile);
  const setContent = useEditorStore((state) => state.setContent);
  const save = useEditorStore((state) => state.save);

  useEffect(() => {
    if (!hostId || !path) return;
    if (requestedMode === 'dir') {
      void initDir(hostId, path);
    } else {
      void initFile(hostId, path);
    }
  }, [hostId, path, requestedMode, initDir, initFile]);

  const handleSave = useCallback(async () => {
    await save();
  }, [save]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        e.preventDefault();
        void handleSave();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [handleSave]);

  useEffect(() => {
    const handleBeforeUnload = async () => {
      if (!dirty) return;
      const answer = await ask('文件有未保存的修改，是否保存？', {
        title: '确认关闭',
        kind: 'warning',
        okLabel: '保存',
        cancelLabel: '不保存',
      });
      if (answer) {
        await save();
      }
      const win = await WebviewWindow.getByLabel('editor');
      if (win) win.destroy();
    };
    window.addEventListener('beforeunload', handleBeforeUnload);
    return () => window.removeEventListener('beforeunload', handleBeforeUnload);
  }, [dirty, save]);

  const shellMode = requestedMode;
  const fallbackName = path ? getBaseName(path) : '远程文件';
  const displayFileName = shellMode === 'dir'
    ? (currentFileName || (loading ? '正在加载…' : '未选择文件'))
    : (fileName || fallbackName);
  const displayPath = shellMode === 'dir'
    ? (currentFilePath || dirPath || path)
    : (filePath || path);
  const sidebarTitle = fileName || (shellMode === 'dir' && path ? getBaseName(path) : '远程目录');

  useEffect(() => {
    WebviewWindow.getByLabel('editor').then((win) => {
      if (!win) return;
      const titleFileName = shellMode === 'dir'
        ? (currentFileName || '未选择文件')
        : (fileName || fallbackName);
      const prefix = dirty ? '● ' : '';
      void win.setTitle(`${prefix}${titleFileName} - 远程编辑`);
    });
  }, [dirty, fileName, currentFileName, shellMode, fallbackName]);

  const handleChange = useCallback((value: string) => {
    setContent(value);
  }, [setContent]);

  return (
    <div className="editor-window">
      <div
        className="editor-toolbar"
        onMouseDown={(e) => {
          const target = e.target as HTMLElement;
          if (target.closest('button')) return;
          void WebviewWindow.getCurrent().startDragging();
        }}
      >
        <WindowControls className="editor-window-controls" />
        <span className="editor-toolbar-file">
          {dirty && <span className="editor-dirty-dot" />}
          {displayFileName}
        </span>
        <span className="editor-toolbar-path">
          {displayPath}
        </span>
        <div className="editor-toolbar-actions">
          {saving && <span className="editor-saving">保存中…</span>}
          <button
            className="editor-save-btn"
            onClick={handleSave}
            disabled={!dirty || saving}
          >
            保存 ⌘S
          </button>
        </div>
      </div>
      <div className="editor-body">
        {shellMode === 'dir' && (
          <div className="editor-sidebar">
            <div className="editor-sidebar-header">
              <span className="editor-sidebar-title">{sidebarTitle}</span>
            </div>
            <FileTree tree={tree} loading={treeLoading || (loading && !tree)} />
          </div>
        )}
        <div className="editor-main">
          {loading ? (
            <EditorContentSkeleton label={shellMode === 'dir' ? '正在加载文件内容…' : '正在加载远程文件…'} />
          ) : shellMode === 'dir' && !currentFilePath ? (
            <div className="editor-empty">从左侧选择文件开始编辑</div>
          ) : (
            <Suspense fallback={<EditorContentSkeleton label="编辑器加载中…" />}>
              <LazyCodeEditor
                value={content}
                onChange={handleChange}
                language={guessLanguage(
                  shellMode === 'dir' ? currentFileName : fileName
                )}
                height="100%"
                placeholder="文件内容"
              />
            </Suspense>
          )}
        </div>
      </div>
      {error && !loading && (
        <div className="editor-error-bar">{error}</div>
      )}
    </div>
  );
}
