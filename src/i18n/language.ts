export type AppLanguage = 'zh-CN' | 'en-US';
export type LanguageMode = 'system' | AppLanguage;

export const supportedLanguages: AppLanguage[] = ['zh-CN', 'en-US'];

export function resolveSystemLanguage(): AppLanguage {
  const candidates = typeof navigator === 'undefined'
    ? []
    : [
      ...(navigator.languages ?? []),
      navigator.language,
    ].filter(Boolean);

  const preferred = candidates.find((lang) => lang.toLowerCase().startsWith('zh'));
  return preferred ? 'zh-CN' : 'en-US';
}

export function resolveLanguage(mode: LanguageMode): AppLanguage {
  return mode === 'system' ? resolveSystemLanguage() : mode;
}
