import type { ReactNode } from 'react';
import { useMemo } from 'react';
import { useLanguageStore } from '../stores/language';
import { dictionaries } from './dictionaries';
import type { AppLanguage, LanguageMode } from './language';

export type { AppLanguage, LanguageMode } from './language';
export { resolveLanguage, resolveSystemLanguage, supportedLanguages } from './language';
export type TranslationKey = keyof typeof dictionaries['zh-CN'];

export interface TranslationValues {
  [key: string]: ReactNode;
}

export interface TranslationOption {
  value: LanguageMode;
  label: string;
}

export function interpolate(template: string, values?: TranslationValues): ReactNode {
  if (!values) return template;

  const parts: ReactNode[] = [];
  const pattern = /\{([a-zA-Z0-9_]+)\}/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = pattern.exec(template)) !== null) {
    if (match.index > lastIndex) {
      parts.push(template.slice(lastIndex, match.index));
    }
    const key = match[1];
    parts.push(values[key] ?? match[0]);
    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < template.length) {
    parts.push(template.slice(lastIndex));
  }

  return parts.length === 1 ? parts[0] : parts;
}

export function translate(language: AppLanguage, key: TranslationKey, values?: TranslationValues): ReactNode {
  const dictionary = dictionaries[language];
  const fallback = dictionaries['zh-CN'];
  return interpolate(dictionary[key] ?? fallback[key] ?? key, values);
}

export function translateText(language: AppLanguage, key: TranslationKey, values?: Record<string, string | number>): string {
  const result = translate(language, key, values);
  return Array.isArray(result) ? result.join('') : String(result);
}

export function useTranslation() {
  const languageMode = useLanguageStore((state) => state.languageMode);
  const language = useLanguageStore((state) => state.language);

  return useMemo(() => {
    const t = (key: TranslationKey, values?: TranslationValues): ReactNode => translate(language, key, values);
    const tText = (key: TranslationKey, values?: Record<string, string | number>): string => translateText(language, key, values);
    const languageOptions: TranslationOption[] = [
      { value: 'system', label: tText('settings.language.followSystem') },
      { value: 'zh-CN', label: tText('settings.language.zhCN') },
      { value: 'en-US', label: tText('settings.language.enUS') },
    ];

    return {
      t,
      tText,
      language,
      languageMode,
      languageOptions,
    };
  }, [language, languageMode]);
}
