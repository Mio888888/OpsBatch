import { useEffect, useMemo, useState, memo } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView } from '@codemirror/view';
import type { Extension } from '@codemirror/state';

type LanguageLoader = () => Promise<Extension>;

const loadShell: LanguageLoader = async () => {
  const [{ StreamLanguage }, { shell }] = await Promise.all([
    import('@codemirror/language'),
    import('@codemirror/legacy-modes/mode/shell'),
  ]);
  return StreamLanguage.define(shell);
};

const loadPowerShell: LanguageLoader = async () => {
  const [{ StreamLanguage }, { powerShell }] = await Promise.all([
    import('@codemirror/language'),
    import('@codemirror/legacy-modes/mode/powershell'),
  ]);
  return StreamLanguage.define(powerShell);
};

const loadToml: LanguageLoader = async () => {
  const [{ StreamLanguage }, { toml }] = await Promise.all([
    import('@codemirror/language'),
    import('@codemirror/legacy-modes/mode/toml'),
  ]);
  return StreamLanguage.define(toml);
};

const loadGo: LanguageLoader = async () => {
  const [{ StreamLanguage }, { go }] = await Promise.all([
    import('@codemirror/language'),
    import('@codemirror/legacy-modes/mode/go'),
  ]);
  return StreamLanguage.define(go);
};

const LANGUAGE_LOADERS: Record<string, LanguageLoader> = {
  shell: loadShell,
  bash: loadShell,
  powershell: loadPowerShell,
  ps1: loadPowerShell,
  toml: loadToml,
  go: loadGo,
  json: async () => (await import('@codemirror/lang-json')).json(),
  html: async () => (await import('@codemirror/lang-html')).html(),
  css: async () => (await import('@codemirror/lang-css')).css(),
  scss: async () => (await import('@codemirror/lang-css')).css(),
  sql: async () => (await import('@codemirror/lang-sql')).sql(),
  markdown: async () => (await import('@codemirror/lang-markdown')).markdown(),
  md: async () => (await import('@codemirror/lang-markdown')).markdown(),
  yaml: async () => (await import('@codemirror/lang-yaml')).yaml(),
  yml: async () => (await import('@codemirror/lang-yaml')).yaml(),
  xml: async () => (await import('@codemirror/lang-xml')).xml(),
  rust: async () => (await import('@codemirror/lang-rust')).rust(),
  java: async () => (await import('@codemirror/lang-java')).java(),
  javascript: async () => (await import('@codemirror/lang-javascript')).javascript(),
  js: async () => (await import('@codemirror/lang-javascript')).javascript(),
  ts: async () => (await import('@codemirror/lang-javascript')).javascript({ typescript: true }),
  tsx: async () => (await import('@codemirror/lang-javascript')).javascript({ jsx: true, typescript: true }),
  jsx: async () => (await import('@codemirror/lang-javascript')).javascript({ jsx: true }),
  python: async () => (await import('@codemirror/lang-python')).python(),
  py: async () => (await import('@codemirror/lang-python')).python(),
  cpp: async () => (await import('@codemirror/lang-cpp')).cpp(),
  c: async () => (await import('@codemirror/lang-cpp')).cpp(),
};

const extensionCache = new Map<string, Promise<Extension>>();

function loadExtension(language: string) {
  const normalizedLanguage = language.toLowerCase();
  const loader = LANGUAGE_LOADERS[normalizedLanguage];
  if (!loader) return null;

  let extension = extensionCache.get(normalizedLanguage);
  if (!extension) {
    extension = loader();
    extensionCache.set(normalizedLanguage, extension);
  }
  return extension;
}

interface CodeEditorProps {
  value: string;
  onChange?: (value: string) => void;
  language?: string;
  readOnly?: boolean;
  height?: string;
  placeholder?: string;
}

const EDITOR_STYLE = {
  fontSize: 13,
  fontFamily: "'Menlo', 'Monaco', 'Courier New', monospace",
  borderRadius: 6,
  overflow: 'auto',
  border: '1px solid #333',
};

const EDITOR_STYLE_FULL_HEIGHT = {
  ...EDITOR_STYLE,
  height: '100%',
  borderRadius: 0,
  border: 'none',
};

const BASIC_SETUP = {
  lineNumbers: true,
  highlightActiveLine: true,
  bracketMatching: true,
  foldGutter: true,
  autocompletion: true,
};

export default memo(function CodeEditor({ value, onChange, language = 'shell', readOnly = false, height = '300px', placeholder }: CodeEditorProps) {
  const [languageExtension, setLanguageExtension] = useState<Extension | null>(null);

  useEffect(() => {
    let ignore = false;
    const extension = loadExtension(language);
    setLanguageExtension(null);

    if (!extension) return undefined;

    extension
      .then((loadedExtension) => {
        if (!ignore) setLanguageExtension(loadedExtension);
      })
      .catch(() => {
        if (!ignore) setLanguageExtension(null);
      });

    return () => {
      ignore = true;
    };
  }, [language]);

  const extensions = useMemo(() => {
    return languageExtension ? [languageExtension, EditorView.lineWrapping] : [EditorView.lineWrapping];
  }, [languageExtension]);

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      extensions={extensions}
      theme={oneDark}
      height={height}
      readOnly={readOnly}
      placeholder={placeholder}
      basicSetup={BASIC_SETUP}
      style={height === '100%' ? EDITOR_STYLE_FULL_HEIGHT : EDITOR_STYLE}
    />
  );
});
