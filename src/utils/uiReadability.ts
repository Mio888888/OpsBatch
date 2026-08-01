export const UI_TEXT_PALETTES = {
  light: {
    primary: '#303446',
    secondary: '#4d5366',
    muted: '#535a6d',
  },
  dark: {
    primary: '#eef1f8',
    secondary: '#bec4d3',
    muted: '#9aa1b5',
  },
} as const;

export function getUiTextPalette(isDark: boolean) {
  return isDark ? UI_TEXT_PALETTES.dark : UI_TEXT_PALETTES.light;
}
