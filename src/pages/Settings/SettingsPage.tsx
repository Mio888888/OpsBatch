import { useState, useEffect, lazy, Suspense, useMemo, useRef } from 'react';
import type { KeyboardEvent } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import {
  Form, Input, Select, Switch, Button, Space, Divider, message,
  InputNumber, Table, Tag, Popconfirm, Modal,
} from '../../components/ui';
import {
  SaveOutlined, DatabaseOutlined, PlusOutlined, DeleteOutlined,
  SyncOutlined,
} from '../../components/ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import type { LanguageMode, TranslationKey } from '../../i18n';
import { useTranslation } from '../../i18n';
import { useLanguageStore, patchLanguage } from '../../stores/language';
import {
  useThemeStore, loadThemeSettings, patchTheme,
  normalizeTerminalFontFamily,
} from '../../stores/theme';
import { requestKeychainNotice } from '../../utils/keychainNotice';
import { darkThemes, lightThemes, accentColors } from '../../data/themes';
import type { ThemePreviewMeta } from '../../data/themes';

const QuickActionsPage = lazy(() => import('../QuickActions/QuickActionsPage'));
const CommandLibPage = lazy(() => import('../CommandLib/CommandLibPage'));
const ScriptLibPage = lazy(() => import('../ScriptLib/ScriptLibPage'));

interface DangerRule {
  id: string;
  name: string;
  pattern: string;
  enabled: boolean;
  is_builtin: boolean;
}

interface AiConfig {
  provider: string;
  api_url: string;
  api_key: string;
  model: string;
  enabled: boolean;
}

interface GeneralSettingsValues {
  defaultConcurrency: number;
  defaultTimeout: number;
  minimizeToTray: boolean;
  logRetentionDays: number;
}

interface DangerRuleFormValues {
  name: string;
  pattern: string;
}

interface SettingsSection {
  key: string;
  labelKey: TranslationKey;
  descriptionKey: TranslationKey;
}

interface FontFamilyInfo {
  family: string;
  styles?: string[];
}

const settingsSections: SettingsSection[] = [
  { key: 'general', labelKey: 'settings.sections.general', descriptionKey: 'settings.sections.generalDesc' },
  { key: 'appearance', labelKey: 'settings.sections.appearance', descriptionKey: 'settings.sections.appearanceDesc' },
  { key: 'quickActions', labelKey: 'settings.sections.quickActions', descriptionKey: 'settings.sections.quickActionsDesc' },
  { key: 'commandLib', labelKey: 'settings.sections.commandLib', descriptionKey: 'settings.sections.commandLibDesc' },
  { key: 'scriptLib', labelKey: 'settings.sections.scriptLib', descriptionKey: 'settings.sections.scriptLibDesc' },
  { key: 'ai', labelKey: 'settings.sections.ai', descriptionKey: 'settings.sections.aiDesc' },
  { key: 'danger', labelKey: 'settings.sections.danger', descriptionKey: 'settings.sections.dangerDesc' },
  { key: 'data', labelKey: 'settings.sections.data', descriptionKey: 'settings.sections.dataDesc' },
];

const AI_PROVIDER_DEFAULTS: Record<string, { apiUrl: string; model: string; requiresKey: boolean; descriptionKey: TranslationKey }> = {
  openai: {
    apiUrl: 'https://api.openai.com/v1',
    model: 'gpt-4o-mini',
    requiresKey: true,
    descriptionKey: 'settings.provider.openaiDesc',
  },
  ollama: {
    apiUrl: 'http://localhost:11434/v1',
    model: 'qwen3:8b',
    requiresKey: false,
    descriptionKey: 'settings.provider.ollamaDesc',
  },
  custom: {
    apiUrl: '',
    model: 'custom',
    requiresKey: true,
    descriptionKey: 'settings.provider.customDesc',
  },
};

const TERMINAL_FONT_SUGGESTIONS = [
  'Cascadia Code',
  'JetBrains Mono',
  'Fira Code',
  'Maple Mono',
  'Sarasa Mono SC',
  'SF Mono',
  'Menlo',
  'Monaco',
  'Courier New',
];

const TERMINAL_FONT_ROW_HEIGHT = 34;
const TERMINAL_FONT_LIST_HEIGHT = 220;
const TERMINAL_FONT_STYLE_SUFFIXES = [
  'Bold Italic',
  'BoldItalic',
  'Bold Oblique',
  'BoldOblique',
  'Extra Light Italic',
  'ExtraLightItalic',
  'Extra Light',
  'ExtraLight',
  'Ultra Light Italic',
  'UltraLightItalic',
  'Ultra Light',
  'UltraLight',
  'Semi Bold Italic',
  'SemiBoldItalic',
  'Semi Bold',
  'SemiBold',
  'Semibold',
  'Semi Light Italic',
  'SemiLightItalic',
  'Semi Light',
  'SemiLight',
  'Light Italic',
  'LightItalic',
  'Demi Bold',
  'DemiBold',
  'Demi Bold Italic',
  'DemiBoldItalic',
  'Extra Bold',
  'ExtraBold',
  'Extra Bold Italic',
  'ExtraBoldItalic',
  'Regular',
  'Roman',
  'Medium',
  'Book',
  'Bold',
  'Italic',
  'Oblique',
  'Light',
  'Thin',
  'Black',
  'Heavy',
  'Condensed',
  'Expanded',
  'Narrow',
];

function normalizeApiUrl(value: string): string {
  const trimmed = value.trim().replace(/\/+$/, '');
  return trimmed.endsWith('/chat/completions')
    ? trimmed.slice(0, -'/chat/completions'.length)
    : trimmed;
}

function quoteFontFamilyName(name: string) {
  const escaped = name.trim().replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  return escaped ? `"${escaped}", monospace` : '';
}

function fontFamilyToLabel(value: string) {
  const first = normalizeTerminalFontFamily(value).split(',')[0]?.trim() ?? '';
  return first.replace(/^["']|["']$/g, '');
}

function terminalFontFamilyName(value: FontFamilyInfo | string) {
  return typeof value === 'string' ? value : value.family;
}

function stripTerminalFontStyleSuffix(family: string) {
  const original = family.trim().replace(/\s+/g, ' ');
  let normalized = original.replace(/[-_]+/g, ' ').replace(/\s+/g, ' ');

  let changed = true;
  while (changed) {
    changed = false;
    const lower = normalized.toLocaleLowerCase();
    for (const suffix of TERMINAL_FONT_STYLE_SUFFIXES) {
      const marker = ` ${suffix.toLocaleLowerCase()}`;
      if (lower.endsWith(marker) && lower.length > marker.length) {
        normalized = normalized.slice(0, -marker.length).trim();
        changed = true;
        break;
      }
    }
  }

  return normalized || original;
}

function mergeFontFamilies(values: Array<FontFamilyInfo | string>, currentFamily: string) {
  const byFamily = new Map<string, string>();
  const add = (value: FontFamilyInfo | string) => {
    const family = stripTerminalFontStyleSuffix(terminalFontFamilyName(value));
    if (!family) return;
    const key = family.toLocaleLowerCase();
    if (!byFamily.has(key)) byFamily.set(key, family);
  };

  values.forEach(add);
  if (currentFamily) add(currentFamily);
  return Array.from(byFamily.values()).sort((a, b) => a.localeCompare(b));
}

function FontFamilySelect({
  value,
  families,
  loading,
  placeholder,
  searchPlaceholder,
  noOptionsText,
  onChange,
}: {
  value: string;
  families: string[];
  loading: boolean;
  placeholder: string;
  searchPlaceholder: string;
  noOptionsText: string;
  onChange: (family: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const normalizedValue = value.toLocaleLowerCase();
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filteredFamilies = useMemo(
    () => normalizedQuery
      ? families.filter((family) => family.toLocaleLowerCase().includes(normalizedQuery))
      : families,
    [families, normalizedQuery],
  );
  const selectedIndex = filteredFamilies.findIndex((family) => family.toLocaleLowerCase() === normalizedValue);
  const virtualizer = useVirtualizer<HTMLDivElement, Element>({
    count: open ? filteredFamilies.length : 0,
    getScrollElement: () => listRef.current,
    estimateSize: () => TERMINAL_FONT_ROW_HEIGHT,
    overscan: 8,
  });
  const listHeight = Math.min(TERMINAL_FONT_LIST_HEIGHT, filteredFamilies.length * TERMINAL_FONT_ROW_HEIGHT);
  const displayValue = value || placeholder;

  useEffect(() => {
    if (!open) return undefined;

    const handlePointerDown = (event: globalThis.MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    return () => document.removeEventListener('mousedown', handlePointerDown);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    searchRef.current?.focus();
    setActiveIndex(selectedIndex >= 0 ? selectedIndex : 0);
  }, [open, selectedIndex]);

  useEffect(() => {
    if (!open) return;
    setActiveIndex((index) => {
      if (filteredFamilies.length === 0) return 0;
      return Math.min(index, filteredFamilies.length - 1);
    });
  }, [filteredFamilies.length, open]);

  useEffect(() => {
    if (open && activeIndex >= 0 && activeIndex < filteredFamilies.length) {
      virtualizer.scrollToIndex(activeIndex, { align: 'auto' });
    }
  }, [activeIndex, filteredFamilies.length, open, virtualizer]);

  const selectFamily = (family: string) => {
    onChange(family);
    setQuery('');
    setOpen(false);
    triggerRef.current?.focus();
  };

  const moveActiveIndex = (delta: number) => {
    setActiveIndex((index) => {
      if (filteredFamilies.length === 0) return 0;
      return Math.max(0, Math.min(index + delta, filteredFamilies.length - 1));
    });
  };

  const handleSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      moveActiveIndex(1);
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      moveActiveIndex(-1);
      return;
    }
    if (event.key === 'Home') {
      event.preventDefault();
      setActiveIndex(0);
      return;
    }
    if (event.key === 'End') {
      event.preventDefault();
      setActiveIndex(Math.max(filteredFamilies.length - 1, 0));
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      const activeFamily = filteredFamilies[activeIndex];
      if (activeFamily) selectFamily(activeFamily);
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      setOpen(false);
      triggerRef.current?.focus();
    }
  };

  const handleTriggerKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      setOpen(true);
    }
  };

  return (
    <div ref={rootRef} className="appearance-font-combobox">
      <button
        ref={triggerRef}
        type="button"
        className={`ui-select-trigger appearance-font-select${open ? ' appearance-font-select-open' : ''}`}
        style={{ fontFamily: value ? quoteFontFamilyName(value) : undefined }}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((next) => !next)}
        onKeyDown={handleTriggerKeyDown}
      >
        <span className={`appearance-font-select-value${value ? '' : ' appearance-font-select-placeholder'}`}>
          {displayValue}
        </span>
        <span className="appearance-font-select-arrow" aria-hidden="true">▾</span>
      </button>
      {open ? (
        <div className="appearance-font-dropdown">
          <div className="appearance-font-search">
            <input
              ref={searchRef}
              type="search"
              className="ui-input"
              value={query}
              placeholder={searchPlaceholder}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={handleSearchKeyDown}
            />
          </div>
          <div
            ref={listRef}
            className="appearance-font-virtual-list"
            role="listbox"
            style={{ height: listHeight || TERMINAL_FONT_ROW_HEIGHT }}
          >
            {filteredFamilies.length > 0 ? (
              <div className="appearance-font-virtual-spacer" style={{ height: virtualizer.getTotalSize() }}>
                {virtualizer.getVirtualItems().map((virtualItem) => {
                  const family = filteredFamilies[virtualItem.index];
                  if (!family) return null;
                  const selected = family.toLocaleLowerCase() === normalizedValue;
                  const active = virtualItem.index === activeIndex;
                  return (
                    <button
                      key={family}
                      type="button"
                      role="option"
                      aria-selected={selected}
                      className={`appearance-font-option-row${selected ? ' appearance-font-option-row-selected' : ''}${active ? ' appearance-font-option-row-active' : ''}`}
                      style={{
                        transform: `translateY(${virtualItem.start}px)`,
                        fontFamily: quoteFontFamilyName(family),
                      }}
                      onMouseEnter={() => setActiveIndex(virtualItem.index)}
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={() => selectFamily(family)}
                    >
                      <span>{family}</span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="appearance-font-empty">{loading ? placeholder : noOptionsText}</div>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function ThemePreviewCard({ theme, selected, onClick }: { theme: ThemePreviewMeta; selected: boolean; onClick: () => void }) {
  const p = theme.preview;
  return (
    <button
      type="button"
      className={`appearance-theme-card${selected ? ' appearance-theme-card-selected' : ''}`}
      onClick={onClick}
      title={theme.name}
    >
      <span className="appearance-theme-thumb" style={{ background: p.background }}>
        <span style={{ display: 'block', height: 3, width: '70%', borderRadius: 2, background: p.lineMedium, margin: '4px 4px 0' }} />
        <span style={{ display: 'block', height: 3, width: '50%', borderRadius: 2, background: p.lineWeak, margin: '3px 4px 0' }} />
        <span style={{ display: 'block', height: 3, width: '80%', borderRadius: 2, background: p.accent, margin: '3px 4px 0' }} />
        <span style={{ display: 'block', height: 3, width: '40%', borderRadius: 2, background: p.lineStrong, margin: '3px 4px 0' }} />
      </span>
      <span className="appearance-theme-name">{theme.name}</span>
    </button>
  );
}

function AppearanceSection() {
  const theme = useThemeStore();
  const { t, tText } = useTranslation();
  const [systemFontFamilies, setSystemFontFamilies] = useState<FontFamilyInfo[]>([]);
  const [loadingFonts, setLoadingFonts] = useState(false);
  const currentFontFamilyLabel = fontFamilyToLabel(theme.terminalFontFamily);
  const fontFamilies = mergeFontFamilies([...TERMINAL_FONT_SUGGESTIONS, ...systemFontFamilies], currentFontFamilyLabel);
  const fontSelectPlaceholder = loadingFonts ? tText('settings.loadingFonts') : tText('settings.selectTerminalFont');

  useEffect(() => {
    let mounted = true;
    setLoadingFonts(true);
    invoke<FontFamilyInfo[]>('list_system_font_families')
      .then((fonts) => {
        if (mounted) setSystemFontFamilies(fonts);
      })
      .catch(() => {
        if (mounted) setSystemFontFamilies([]);
      })
      .finally(() => {
        if (mounted) setLoadingFonts(false);
      });
    return () => { mounted = false; };
  }, []);

  return (
    <div className="appearance-content">
      {/* 终端主题 — System */}
      <div className="appearance-group">
        <div className="appearance-group-header">
          <h3>{t('settings.terminalTheme')}</h3>
          <p>{t('settings.terminalThemeDesc')}</p>
        </div>
        <div className="appearance-theme-row">
          <button
            type="button"
            className={`appearance-theme-card appearance-theme-card-system${theme.themeMode === 'system' ? ' appearance-theme-card-selected' : ''}`}
            onClick={() => patchTheme({ themeMode: 'system' })}
          >
            <span className="appearance-theme-thumb appearance-theme-thumb-system">
              <span className="appearance-system-icon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
                  <line x1="8" y1="21" x2="16" y2="21" />
                  <line x1="12" y1="17" x2="12" y2="21" />
                </svg>
              </span>
              <span className="appearance-theme-name">System</span>
            </span>
          </button>
        </div>
      </div>

      {/* 深色主题 */}
      <div className="appearance-group">
        <div className="appearance-group-label">
          <span className="appearance-group-label-dot" style={{ background: '#6b7a8d' }} />
          {t('settings.dark')}
        </div>
        <div className="appearance-theme-grid">
          {darkThemes.map((t) => (
            <ThemePreviewCard
              key={t.id}
              theme={t}
              selected={theme.themeMode === 'manual' && theme.terminalThemeId === t.id}
              onClick={() => patchTheme({ themeMode: 'manual', terminalThemeId: t.id, terminalThemeTone: 'dark' })}
            />
          ))}
        </div>
      </div>

      {/* 浅色主题 */}
      <div className="appearance-group">
        <div className="appearance-group-label">
          <span className="appearance-group-label-dot" style={{ background: '#d4d4d6' }} />
          {t('settings.light')}
        </div>
        <div className="appearance-theme-grid">
          {lightThemes.map((t) => (
            <ThemePreviewCard
              key={t.id}
              theme={t}
              selected={theme.themeMode === 'manual' && theme.terminalThemeId === t.id}
              onClick={() => patchTheme({ themeMode: 'manual', terminalThemeId: t.id, terminalThemeTone: 'light' })}
            />
          ))}
        </div>
      </div>

      {/* 强调色 */}
      <div className="appearance-group">
        <div className="appearance-group-header">
          <h3>{t('settings.accentColor')}</h3>
          <p>{t('settings.accentColorDesc')}</p>
        </div>
        <div className="appearance-accent-row">
          {accentColors.map((c) => (
            <button
              key={c.id}
              type="button"
              className={`appearance-accent-dot${theme.accentColorId === c.id ? ' appearance-accent-dot-selected' : ''}`}
              style={{ background: c.base, borderColor: c.base }}
              onClick={() => patchTheme({ accentColorId: c.id })}
              title={c.name}
            />
          ))}
        </div>
      </div>

      {/* 动效 */}
      <div className="appearance-group">
        <div className="appearance-motion-option">
          <div className="appearance-motion-copy">
            <h3>{t('settings.reduceMotion')}</h3>
            <p>{t('settings.reduceMotionDesc')}</p>
          </div>
          <Switch
            checked={theme.reduceMotion}
            onChange={(checked) => { void patchTheme({ reduceMotion: checked }); }}
          />
        </div>
      </div>

      {/* 终端字号 & 历史行数 */}
      <div className="appearance-group">
        <div className="appearance-group-header">
          <h3>{t('settings.terminalText')}</h3>
        </div>
        <div className="appearance-font-panel">
          <div className="appearance-font-field">
            <label className="appearance-number-label">{t('settings.terminalFontFamily')}</label>
            <FontFamilySelect
              value={currentFontFamilyLabel}
              families={fontFamilies}
              loading={loadingFonts}
              placeholder={fontSelectPlaceholder}
              searchPlaceholder={tText('settings.terminalFontSearchPlaceholder')}
              noOptionsText={tText('common.noOptions')}
              onChange={(family) => { void patchTheme({ terminalFontFamily: quoteFontFamilyName(family) }); }}
            />
          </div>
          <div className="appearance-font-stack-field">
            <label className="appearance-number-label" htmlFor="terminal-font-family">{t('settings.terminalFontFamilyStack')}</label>
            <Input
              id="terminal-font-family"
              value={theme.terminalFontFamily}
              placeholder={tText('settings.terminalFontFamilyPlaceholder')}
              onChange={(event) => { void patchTheme({ terminalFontFamily: event.target.value }); }}
              onBlur={(event) => { void patchTheme({ terminalFontFamily: normalizeTerminalFontFamily(event.target.value) }); }}
              style={{ fontFamily: theme.terminalFontFamily }}
            />
          </div>
          <p className="appearance-font-hint">{t('settings.terminalFontFamilyHint')}</p>
        </div>
        <div className="appearance-number-fields">
          <div className="appearance-number-field">
            <label className="appearance-number-label">{t('settings.terminalFontSize')}</label>
            <InputNumber
              min={10}
              max={24}
              value={theme.terminalFontSize}
              onChange={(v) => v != null && patchTheme({ terminalFontSize: v })}
              style={{ width: 120 }}
            />
          </div>
          <div className="appearance-number-field">
            <label className="appearance-number-label">{t('settings.scrollback')}</label>
            <InputNumber
              min={500}
              max={50000}
              step={100}
              value={theme.terminalScrollback}
              onChange={(v) => v != null && patchTheme({ terminalScrollback: v })}
              style={{ width: 120 }}
              addonAfter={t('settings.lines')}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

export default function SettingsPage() {
  const [activeSection, setActiveSection] = useState('general');
  const [dangerRules, setDangerRules] = useState<DangerRule[]>([]);
  const [ruleModalOpen, setRuleModalOpen] = useState(false);
  const [ruleForm] = Form.useForm<DangerRuleFormValues>();
  const [aiForm] = Form.useForm<AiConfig>();
  const [generalForm] = Form.useForm<GeneralSettingsValues>();
  const [, setAiConfig] = useState<AiConfig | null>(null);
  const { t, tText, languageOptions } = useTranslation();
  const languageMode = useLanguageStore((state) => state.languageMode);
  const [testing, setTesting] = useState(false);
  const [customModel, setCustomModel] = useState('');
  const [aiProvider, setAiProvider] = useState('openai');
  const [aiModel, setAiModel] = useState('gpt-4o-mini');
  const [fetchedModels, setFetchedModels] = useState<{ value: string; label: string }[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);

  const activeSectionMeta = settingsSections.find((section) => section.key === activeSection) ?? settingsSections[0];
  const providerDefaults = AI_PROVIDER_DEFAULTS[aiProvider] ?? AI_PROVIDER_DEFAULTS.custom;

  const getModelOptions = (provider: string) => {
    switch (provider) {
      case 'ollama':
        return [
          { value: 'qwen3:8b', label: 'Qwen3 8B' },
          { value: 'deepseek-r1:8b', label: 'DeepSeek R1 8B' },
          { value: 'llama3.1:8b', label: 'Llama 3.1 8B' },
          { value: 'mistral:7b', label: 'Mistral 7B' },
          { value: 'custom', label: tText('settings.customModel') },
        ];
      case 'openai':
        return [
          { value: 'gpt-4o', label: 'GPT-4o' },
          { value: 'gpt-4o-mini', label: 'GPT-4o Mini' },
          { value: 'gpt-4.1', label: 'GPT-4.1' },
          { value: 'gpt-4.1-mini', label: 'GPT-4.1 Mini' },
          { value: 'o3', label: 'o3' },
          { value: 'o4-mini', label: 'o4-mini' },
          { value: 'custom', label: tText('settings.customModel') },
        ];
      default:
        return [
          { value: 'deepseek-chat', label: 'DeepSeek Chat' },
          { value: 'deepseek-reasoner', label: 'DeepSeek Reasoner' },
          { value: 'qwen-turbo', label: '通义千问 Turbo' },
          { value: 'qwen-plus', label: '通义千问 Plus' },
          { value: 'claude-sonnet-4-20250514', label: 'Claude Sonnet 4' },
          { value: 'gpt-4o', label: 'GPT-4o' },
          { value: 'custom', label: tText('settings.customModel') },
        ];
    }
  };

  const selectedModelLabel = aiModel === 'custom'
    ? customModel.trim() || tText('settings.customModelMissing')
    : getModelOptions(aiProvider).find((option) => option.value === aiModel)?.label ?? aiModel;

  const modelOptions = fetchedModels.length > 0
    ? [...fetchedModels, { value: 'custom', label: tText('settings.customModel') }]
    : getModelOptions(aiProvider);

  useEffect(() => {
    loadDangerRules();
    loadAiConfig();
    loadGeneralSettings();
    loadThemeSettings();
  }, []);

  const loadDangerRules = async () => {
    try {
      const rules = await invoke<DangerRule[]>('list_danger_rules');
      setDangerRules(rules);
    } catch { setDangerRules([]); }
  };

  const loadAiConfig = async () => {
    try {
      const config = await invoke<AiConfig>('get_ai_config');
      setAiConfig(config);
      const isPresetModel = getModelOptions(config.provider).some(o => o.value === config.model);
      if (!isPresetModel && config.model) {
        setCustomModel(config.model);
        aiForm.setFieldsValue({ ...config, model: 'custom' });
        setAiModel('custom');
      } else {
        setCustomModel('');
        aiForm.setFieldsValue(config);
        setAiModel(config.model);
      }
      setAiProvider(config.provider);
    } catch {}
  };

  const loadGeneralSettings = async () => {
    try {
      const settings = await invoke<Record<string, string>>('get_general_settings');
      generalForm.setFieldsValue({
        defaultConcurrency: settings.defaultConcurrency ? parseInt(settings.defaultConcurrency) : 10,
        defaultTimeout: settings.defaultTimeout ? parseInt(settings.defaultTimeout) : 30,
        minimizeToTray: settings.minimizeToTray !== 'false',
        logRetentionDays: settings.logRetentionDays ? parseInt(settings.logRetentionDays) : 90,
      });
    } catch {}
  };

  const handleSaveGeneral = async () => {
    try {
      const values = await generalForm.validateFields();
      await invoke('save_general_settings', {
        settings: {
          defaultConcurrency: String(values.defaultConcurrency),
          defaultTimeout: String(values.defaultTimeout),
          minimizeToTray: String(values.minimizeToTray ?? true),
          logRetentionDays: String(values.logRetentionDays || 90),
        },
      });
      message.success(tText('settings.generalSaved'));
    } catch {}
  };


  const handleLanguageChange = (nextLanguageMode: LanguageMode) => {
    void patchLanguage(nextLanguageMode);
    message.success(tText('settings.languageSaved'));
  };

  const handleAiProviderChange = (provider: string) => {
    const defaults = AI_PROVIDER_DEFAULTS[provider] ?? AI_PROVIDER_DEFAULTS.custom;
    setAiProvider(provider);
    setAiModel(defaults.model);
    setFetchedModels([]);
    if (defaults.model !== 'custom') {
      setCustomModel('');
    }
    aiForm.setFieldsValue({
      provider,
      api_url: defaults.apiUrl,
      model: defaults.model,
      api_key: provider === 'ollama' ? '' : String(aiForm.getFieldValue('api_key') ?? ''),
    });
  };

  const handleAiModelChange = (model: string) => {
    setAiModel(model);
    if (model !== 'custom') {
      setCustomModel('');
    }
  };

  const handleFetchModels = async () => {
    setFetchingModels(true);
    try {
      const formValues = aiForm.getFieldsValue();
      const models = await invoke<{ id: string; owned_by: string | null }[]>('ai_list_models', {
        apiUrl: formValues.api_url || undefined,
        apiKey: formValues.api_key?.includes('****') ? undefined : (formValues.api_key || undefined),
        provider: aiProvider,
      });
      const sorted = models.sort((a, b) => a.id.localeCompare(b.id));
      setFetchedModels(sorted.map((m) => ({ value: m.id, label: m.id })));
      if (sorted.length > 0) {
        message.success(tText('settings.fetchModelsSuccess', { count: sorted.length }));
      } else {
        message.warning(tText('settings.fetchModelsEmpty'));
      }
    } catch (e: unknown) {
      message.error(tText('settings.fetchModelsFailed', { error: String(e) }));
    } finally {
      setFetchingModels(false);
    }
  };

  const handleSaveAi = async () => {
    try {
      const values = await aiForm.validateFields();
      const configToSave = {
        ...values,
        api_url: normalizeApiUrl(values.api_url),
        api_key: values.provider === 'ollama' ? '' : values.api_key,
        model: values.model === 'custom' ? customModel.trim() : values.model,
      };
      if (values.model === 'custom' && !customModel.trim()) {
        message.error(tText('settings.aiMissingCustomModel'));
        return;
      }
      if (!configToSave.api_url) {
        message.error(tText('settings.aiMissingApiUrl'));
        return;
      }
      const storesApiKey = Boolean(
        configToSave.api_key
        && values.provider !== 'ollama'
        && !String(configToSave.api_key).includes('****')
        && configToSave.api_key !== '***keychain***',
      );
      if (storesApiKey && !(await requestKeychainNotice())) return;
      await invoke('save_ai_config', { config: configToSave });
      message.success(tText('settings.aiSaved'));
      setAiConfig(configToSave);
      aiForm.setFieldsValue({ ...configToSave, model: values.model });
    } catch {}
  };

  const handleTestAi = async () => {
    setTesting(true);
    try {
      const values = await aiForm.validateFields();
      const model = values.model === 'custom' ? customModel.trim() : values.model;
      if (!model) {
        message.error(tText('settings.aiSelectModelFirst'));
        return;
      }
      if (values.provider !== 'ollama' && !(await requestKeychainNotice())) return;
      const result = await invoke<{ content: string }>('ai_chat', {
        request: { messages: [{ role: 'user', content: tText('settings.aiTestPrompt') }], model },
      });
      message.success(tText('settings.aiTestSuccess', { content: result.content }));
    } catch (e: unknown) {
      message.error(tText('settings.aiTestFailed', { error: String(e) }));
    } finally {
      setTesting(false);
    }
  };

  const handleAddRule = async () => {
    try {
      const values = await ruleForm.validateFields();
      await invoke('add_danger_rule', { name: values.name, pattern: values.pattern });
      message.success(tText('settings.ruleAdded'));
      setRuleModalOpen(false);
      ruleForm.resetFields();
      loadDangerRules();
    } catch {}
  };

  const handleDeleteRule = async (id: string) => {
    await invoke('delete_danger_rule', { id });
    loadDangerRules();
  };

  const handleToggleRule = async (id: string, enabled: boolean) => {
    await invoke('toggle_danger_rule', { id, enabled });
    loadDangerRules();
  };

  const handleExportDatabase = async () => {
    try {
      const destination = await save({
        title: tText('settings.exportBackupTitle'),
        defaultPath: `opsbatch-${new Date().toISOString().slice(0, 10)}.db`,
        filters: [{ name: tText('settings.sqliteDatabase'), extensions: ['db'] }],
      });
      if (!destination) return;
      await invoke('export_database_backup', { destination });
      message.success(tText('settings.backupExported'));
    } catch (e: unknown) {
      message.error(tText('common.exportFailed', { error: String(e) }));
    }
  };

  const renderSectionContent = () => {
    switch (activeSection) {
      case 'general':
        return (
          <Form form={generalForm} layout="vertical" className="settings-section-form">
            <Form.Item name="defaultConcurrency" label={t('settings.defaultConcurrency')} initialValue={10}>
              <InputNumber min={1} max={100} style={{ width: '100%' }} />
            </Form.Item>
            <Form.Item name="defaultTimeout" label={t('settings.defaultTimeout')} initialValue={30}>
              <InputNumber min={1} max={3600} style={{ width: '100%' }} />
            </Form.Item>
            <Form.Item name="minimizeToTray" label={t('settings.minimizeToTray')} valuePropName="checked" initialValue={true}>
              <Switch />
            </Form.Item>
            <Form.Item name="logRetentionDays" label={t('settings.logRetentionDays')} initialValue={90}>
              <InputNumber min={7} max={365} style={{ width: '100%' }} addonAfter={t('settings.days')} />
            </Form.Item>
            <Form.Item label={t('settings.language')} extra={t('settings.languageExtra')}>
              <Select<LanguageMode> value={languageMode} options={languageOptions} onChange={handleLanguageChange} style={{ width: '100%' }} />
            </Form.Item>
            <Button type="primary" icon={<SaveOutlined />} onClick={handleSaveGeneral}>{t('common.save')}</Button>
          </Form>
        );
      case 'appearance':
        return <AppearanceSection />;
      case 'ai':
        return (
          <Form form={aiForm} layout="vertical" className="settings-section-form" style={{ maxWidth: 600 }}>
            {/* Row 1: Enabled + Provider */}
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, alignItems: 'end' }}>
              <Form.Item name="enabled" label={t('settings.aiEnabled')} valuePropName="checked">
                <Switch />
              </Form.Item>
              <Form.Item name="provider" label={t('settings.aiProvider')}>
                <Select options={[
                  { value: 'openai', label: 'OpenAI' },
                  { value: 'ollama', label: t('settings.ollamaLocal') },
                  { value: 'custom', label: t('settings.customApi') },
                ]} onChange={handleAiProviderChange} />
              </Form.Item>
            </div>

            {/* Provider hint */}
            <div className="settings-ai-hint">{t(providerDefaults.descriptionKey)}</div>

            {/* Row 2: API URL + Key */}
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
              <Form.Item name="api_url" label={t('settings.apiUrl')} rules={[{ required: true, message: tText('settings.aiMissingApiUrl') }]}>
                <Input placeholder={providerDefaults.apiUrl || 'https://api.example.com/v1'} onBlur={(e) => aiForm.setFieldValue('api_url', normalizeApiUrl(e.target.value))} />
              </Form.Item>
              {providerDefaults.requiresKey ? (
                <Form.Item name="api_key" label="API Key" rules={[{ required: true, message: tText('settings.apiKeyRequired') }]}>
                  <Input.Password placeholder="sk-..." />
                </Form.Item>
              ) : (
                <Form.Item label="API Key">
                  <Input disabled placeholder={tText('settings.apiKeyNotRequired')} />
                </Form.Item>
              )}
            </div>

            {/* Row 3: Model + Fetch */}
            <Form.Item name="model" label={t('settings.model')}>
              <div style={{ display: 'flex', gap: 8 }}>
                <Select options={modelOptions} onChange={handleAiModelChange} style={{ flex: 1 }} />
                <Button icon={<SyncOutlined spin={fetchingModels} />} onClick={handleFetchModels} loading={fetchingModels} title={tText('settings.fetchModelsTitle')}>
                  {t('settings.fetchModels')}
                </Button>
              </div>
            </Form.Item>
            {aiModel === 'custom' && (
              <Form.Item label={t('settings.customModelName')}>
                <Input
                  value={customModel}
                  onChange={(e) => setCustomModel(e.target.value)}
                  placeholder={tText('settings.customModelPlaceholder')}
                />
              </Form.Item>
            )}

            {/* Summary */}
            <div style={{ display: 'flex', gap: 16, marginBottom: 16, fontSize: 12, color: 'var(--color-text-muted)' }}>
              <span>{t('settings.currentModel')}<strong style={{ color: 'var(--color-text)' }}>{selectedModelLabel}</strong></span>
              <span>{t('settings.auth')}{providerDefaults.requiresKey ? 'Bearer API Key' : t('settings.apiKeyNotRequired')}</span>
            </div>

            {/* Actions */}
            <Space>
              <Button type="primary" icon={<SaveOutlined />} onClick={handleSaveAi}>{t('common.saveSettings')}</Button>
              <Button loading={testing} onClick={handleTestAi}>{t('settings.testConnection')}</Button>
            </Space>
          </Form>
        );
      case 'danger':
        return (
          <div>
            <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 12 }}>
              <Button type="primary" size="small" icon={<PlusOutlined />} onClick={() => setRuleModalOpen(true)}>
                {t('settings.addRule')}
              </Button>
            </div>
            <Table
              rowKey="id"
              size="small"
              dataSource={dangerRules}
              pagination={false}
              columns={[
                { title: t('common.name'), dataIndex: 'name', width: 180 },
                {
                  title: t('settings.regex'), dataIndex: 'pattern', ellipsis: true,
                  render: (p: string) => <code style={{ fontSize: 12 }}>{p}</code>,
                },
                {
                  title: t('common.source'), dataIndex: 'is_builtin', width: 80,
                  render: (b: boolean) => b ? <Tag>{t('common.builtin')}</Tag> : <Tag color="blue">{t('common.custom')}</Tag>,
                },
                {
                  title: t('common.enabled'), dataIndex: 'enabled', width: 80,
                  render: (e: boolean, r: DangerRule) => (
                    <Switch size="small" checked={e} onChange={(v) => handleToggleRule(r.id, v)} />
                  ),
                },
                {
                  title: t('common.action'), width: 80,
                  render: (_: unknown, r: DangerRule) => !r.is_builtin ? (
                    <Popconfirm title={t('common.confirmDelete')} onConfirm={() => handleDeleteRule(r.id)}>
                      <Button type="text" size="small" danger icon={<DeleteOutlined />} />
                    </Popconfirm>
                  ) : '-',
                },
              ]}
            />
          </div>
        );
      case 'data':
        return (
          <Form layout="vertical" className="settings-section-form">
            <Form.Item label={t('settings.dataPath')}>
              <Input disabled value="~/.opsbatch/data.db" />
            </Form.Item>
            <Form.Item label={t('settings.dataBackup')}>
              <Space>
                <Button icon={<DatabaseOutlined />} onClick={handleExportDatabase}>{t('settings.exportData')}</Button>
                <Button disabled title={tText('settings.importDataTitle')}>{t('settings.importData')}</Button>
              </Space>
            </Form.Item>
            <Divider />
            <Form.Item label={t('settings.clearData')}>
              <Button danger disabled title={tText('settings.clearAllDataTitle')}>{t('settings.clearAllData')}</Button>
            </Form.Item>
          </Form>
        );
      case 'quickActions':
        return (
          <Suspense fallback={<div style={{ padding: 24, color: 'var(--color-text-muted)' }}>{t('common.loadingShort')}</div>}>
            <QuickActionsPage embedded />
          </Suspense>
        );
      case 'commandLib':
        return (
          <Suspense fallback={<div style={{ padding: 24, color: 'var(--color-text-muted)' }}>{t('common.loadingShort')}</div>}>
            <CommandLibPage />
          </Suspense>
        );
      case 'scriptLib':
        return (
          <Suspense fallback={<div style={{ padding: 24, color: 'var(--color-text-muted)' }}>{t('common.loadingShort')}</div>}>
            <ScriptLibPage />
          </Suspense>
        );
      default:
        return null;
    }
  };

  return (
    <div className="page-container settings-window-page">
      <div className="settings-window-content">
        <aside className="settings-sidebar-card" aria-label={tText('settings.sidebarAria')}>
          <nav className="settings-nav-list">
            {settingsSections.map((section) => (
              <button
                key={section.key}
                type="button"
                className={`settings-nav-item${activeSection === section.key ? ' settings-nav-item-active' : ''}`}
                onClick={() => setActiveSection(section.key)}
              >
                <span className="settings-nav-title">{t(section.labelKey)}</span>
                <span className="settings-nav-description">{t(section.descriptionKey)}</span>
              </button>
            ))}
          </nav>
        </aside>

        <section className="settings-main-card">
          <header className="settings-main-header">
            <div>
              <span className="settings-main-eyebrow">{t('settings.currentSection')}</span>
              <h2>{t(activeSectionMeta.labelKey)}</h2>
              <p>{t(activeSectionMeta.descriptionKey)}</p>
            </div>
          </header>
          <div className="settings-main-body">
            {renderSectionContent()}
          </div>
        </section>
      </div>

      <Modal
        title={t('settings.addDangerRule')}
        open={ruleModalOpen}
        onOk={handleAddRule}
        onCancel={() => setRuleModalOpen(false)}
        destroyOnHidden
      >
        <Form form={ruleForm} layout="vertical" style={{ marginTop: 16 }}>
          <Form.Item name="name" label={t('settings.ruleName')} rules={[{ required: true, message: tText('common.required') }]}>
            <Input placeholder={tText('settings.ruleNamePlaceholder')} />
          </Form.Item>
          <Form.Item name="pattern" label={t('settings.regex')} rules={[{ required: true, message: tText('common.required') }]}>
            <Input placeholder={tText('settings.regexPlaceholder')} style={{ fontFamily: 'monospace' }} />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
