export interface ThemePreviewMeta {
  id: string;
  name: string;
  tone: 'dark' | 'light';
  preview: {
    background: string;
    lineStrong: string;
    lineMedium: string;
    lineWeak: string;
    accent: string;
  };
  ui: {
    pageBg: string;
    sidebarBg: string;
    mainBg: string;
    cardBg: string;
    headerBg: string;
    border: string;
    borderStrong: string;
    inputBg: string;
    inputBorder: string;
    navHover: string;
  };
  xterm: Record<string, string>;
}

export interface AccentColorMeta {
  id: string;
  name: string;
  base: string;
  strong: string;
}

export const darkThemes: ThemePreviewMeta[] = [
  {
    id: 'tokyo-night-storm',
    name: 'Tokyo Night Storm',
    tone: 'dark',
    preview: { background: '#24283b', lineStrong: '#c0caf5', lineMedium: '#9aa5ce', lineWeak: '#565f89', accent: '#7aa2f7' },
    ui: { pageBg: '#1a1b2e', sidebarBg: '#1f2035', mainBg: '#222338', cardBg: '#282a40', headerBg: '#1e1f33', border: 'rgba(122, 162, 247, 0.1)', borderStrong: 'rgba(122, 162, 247, 0.18)', inputBg: 'rgba(192, 202, 245, 0.05)', inputBorder: 'rgba(192, 202, 245, 0.12)', navHover: 'rgba(192, 202, 245, 0.04)' },
    xterm: { background: '#24283b', foreground: '#c0caf5', cursor: '#c0caf5', selectionBackground: '#364a82', black: '#15161e', red: '#f7768e', green: '#9ece6a', yellow: '#e0af68', blue: '#7aa2f7', magenta: '#bb9af7', cyan: '#7dcfff', white: '#a9b1d6', brightBlack: '#414868', brightRed: '#f7768e', brightGreen: '#9ece6a', brightYellow: '#e0af68', brightBlue: '#7aa2f7', brightMagenta: '#bb9af7', brightCyan: '#7dcfff', brightWhite: '#c0caf5' },
  },
  {
    id: 'dracula',
    name: 'Dracula',
    tone: 'dark',
    preview: { background: '#282a36', lineStrong: '#f8f8f2', lineMedium: '#bd93f9', lineWeak: '#6272a4', accent: '#bd93f9' },
    ui: { pageBg: '#1e1f29', sidebarBg: '#21222c', mainBg: '#252636', cardBg: '#2a2c3e', headerBg: '#232535', border: 'rgba(189, 147, 249, 0.1)', borderStrong: 'rgba(189, 147, 249, 0.18)', inputBg: 'rgba(248, 248, 242, 0.04)', inputBorder: 'rgba(248, 248, 242, 0.1)', navHover: 'rgba(189, 147, 249, 0.06)' },
    xterm: { background: '#282a36', foreground: '#f8f8f2', cursor: '#f8f8f2', selectionBackground: '#44475a', black: '#21222c', red: '#ff5555', green: '#50fa7b', yellow: '#f1fa8c', blue: '#bd93f9', magenta: '#ff79c6', cyan: '#8be9fd', white: '#f8f8f2', brightBlack: '#6272a4', brightRed: '#ff6e6e', brightGreen: '#69ff94', brightYellow: '#ffffa5', brightBlue: '#d6acff', brightMagenta: '#ff92df', brightCyan: '#a4ffff', brightWhite: '#ffffff' },
  },
  {
    id: 'one-dark',
    name: 'One Dark',
    tone: 'dark',
    preview: { background: '#282c34', lineStrong: '#abb2bf', lineMedium: '#7f848e', lineWeak: '#4b5263', accent: '#61afef' },
    ui: { pageBg: '#1e2128', sidebarBg: '#22252c', mainBg: '#262930', cardBg: '#2c3038', headerBg: '#24272e', border: 'rgba(97, 175, 239, 0.08)', borderStrong: 'rgba(97, 175, 239, 0.14)', inputBg: 'rgba(171, 178, 191, 0.04)', inputBorder: 'rgba(171, 178, 191, 0.1)', navHover: 'rgba(97, 175, 239, 0.04)' },
    xterm: { background: '#282c34', foreground: '#abb2bf', cursor: '#528bff', selectionBackground: '#3e4451', black: '#282c34', red: '#e06c75', green: '#98c379', yellow: '#e5c07b', blue: '#61afef', magenta: '#c678dd', cyan: '#56b6c2', white: '#abb2bf', brightBlack: '#5c6370', brightRed: '#e06c75', brightGreen: '#98c379', brightYellow: '#e5c07b', brightBlue: '#61afef', brightMagenta: '#c678dd', brightCyan: '#56b6c2', brightWhite: '#ffffff' },
  },
  {
    id: 'nord',
    name: 'Nord',
    tone: 'dark',
    preview: { background: '#2e3440', lineStrong: '#d8dee9', lineMedium: '#81a1c1', lineWeak: '#4c566a', accent: '#88c0d0' },
    ui: { pageBg: '#242933', sidebarBg: '#272c36', mainBg: '#2b313b', cardBg: '#313844', headerBg: '#292e38', border: 'rgba(136, 192, 208, 0.08)', borderStrong: 'rgba(136, 192, 208, 0.14)', inputBg: 'rgba(216, 222, 233, 0.04)', inputBorder: 'rgba(216, 222, 233, 0.1)', navHover: 'rgba(136, 192, 208, 0.04)' },
    xterm: { background: '#2e3440', foreground: '#d8dee9', cursor: '#d8dee9', selectionBackground: '#434c5e', black: '#3b4252', red: '#bf616a', green: '#a3be8c', yellow: '#ebcb8b', blue: '#81a1c1', magenta: '#b48ead', cyan: '#88c0d0', white: '#e5e9f0', brightBlack: '#4c566a', brightRed: '#bf616a', brightGreen: '#a3be8c', brightYellow: '#ebcb8b', brightBlue: '#81a1c1', brightMagenta: '#b48ead', brightCyan: '#8fbcbb', brightWhite: '#eceff4' },
  },
  {
    id: 'gruvbox-dark',
    name: 'Gruvbox Dark',
    tone: 'dark',
    preview: { background: '#282828', lineStrong: '#ebdbb2', lineMedium: '#bdae93', lineWeak: '#665c54', accent: '#fabd2f' },
    ui: { pageBg: '#1d2021', sidebarBg: '#212020', mainBg: '#262424', cardBg: '#2c2a2a', headerBg: '#242222', border: 'rgba(250, 189, 47, 0.08)', borderStrong: 'rgba(250, 189, 47, 0.14)', inputBg: 'rgba(235, 219, 178, 0.04)', inputBorder: 'rgba(235, 219, 178, 0.1)', navHover: 'rgba(250, 189, 47, 0.04)' },
    xterm: { background: '#282828', foreground: '#ebdbb2', cursor: '#ebdbb2', selectionBackground: '#504945', black: '#282828', red: '#cc241d', green: '#98971a', yellow: '#d79921', blue: '#458588', magenta: '#b16286', cyan: '#689d6a', white: '#a89984', brightBlack: '#928374', brightRed: '#fb4934', brightGreen: '#b8bb26', brightYellow: '#fabd2f', brightBlue: '#83a598', brightMagenta: '#d3869b', brightCyan: '#8ec07c', brightWhite: '#ebdbb2' },
  },
  {
    id: 'catppuccin-mocha',
    name: 'Catppuccin Mocha',
    tone: 'dark',
    preview: { background: '#1e1e2e', lineStrong: '#cdd6f4', lineMedium: '#a6adc8', lineWeak: '#585b70', accent: '#cba6f7' },
    ui: { pageBg: '#16161e', sidebarBg: '#191924', mainBg: '#1c1c2a', cardBg: '#222234', headerBg: '#1a1a28', border: 'rgba(203, 166, 247, 0.08)', borderStrong: 'rgba(203, 166, 247, 0.14)', inputBg: 'rgba(205, 214, 244, 0.04)', inputBorder: 'rgba(205, 214, 244, 0.1)', navHover: 'rgba(203, 166, 247, 0.04)' },
    xterm: { background: '#1e1e2e', foreground: '#cdd6f4', cursor: '#f5e0dc', selectionBackground: '#45475a', black: '#45475a', red: '#f38ba8', green: '#a6e3a1', yellow: '#f9e2af', blue: '#89b4fa', magenta: '#f5c2e7', cyan: '#94e2d5', white: '#bac2de', brightBlack: '#585b70', brightRed: '#f38ba8', brightGreen: '#a6e3a1', brightYellow: '#f9e2af', brightBlue: '#89b4fa', brightMagenta: '#f5c2e7', brightCyan: '#94e2d5', brightWhite: '#a6adc8' },
  },
  {
    id: 'github-dark',
    name: 'GitHub Dark',
    tone: 'dark',
    preview: { background: '#0d1117', lineStrong: '#e6edf3', lineMedium: '#7d8590', lineWeak: '#484f58', accent: '#58a6ff' },
    ui: { pageBg: '#090c10', sidebarBg: '#0b0e14', mainBg: '#0d1117', cardBg: '#131921', headerBg: '#0c1018', border: 'rgba(88, 166, 255, 0.08)', borderStrong: 'rgba(88, 166, 255, 0.14)', inputBg: 'rgba(230, 237, 243, 0.04)', inputBorder: 'rgba(230, 237, 243, 0.1)', navHover: 'rgba(88, 166, 255, 0.04)' },
    xterm: { background: '#0d1117', foreground: '#e6edf3', cursor: '#e6edf3', selectionBackground: '#264f78', black: '#484f58', red: '#ff7b72', green: '#3fb950', yellow: '#d29922', blue: '#58a6ff', magenta: '#bc8cff', cyan: '#39c5cf', white: '#b1bac4', brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364', brightYellow: '#e3b341', brightBlue: '#79c0ff', brightMagenta: '#d2a8ff', brightCyan: '#56d4dd', brightWhite: '#ffffff' },
  },
  {
    id: 'material-darker',
    name: 'Material Darker',
    tone: 'dark',
    preview: { background: '#212121', lineStrong: '#eeffff', lineMedium: '#89ddff', lineWeak: '#546e7a', accent: '#82aaff' },
    ui: { pageBg: '#181818', sidebarBg: '#1b1b1b', mainBg: '#1f1f1f', cardBg: '#262626', headerBg: '#1d1d1d', border: 'rgba(130, 170, 255, 0.08)', borderStrong: 'rgba(130, 170, 255, 0.14)', inputBg: 'rgba(238, 255, 255, 0.04)', inputBorder: 'rgba(238, 255, 255, 0.1)', navHover: 'rgba(130, 170, 255, 0.04)' },
    xterm: { background: '#212121', foreground: '#eeffff', cursor: '#eeffff', selectionBackground: '#44475a', black: '#000000', red: '#ff5370', green: '#c3e88d', yellow: '#ffcb6b', blue: '#82aaff', magenta: '#c792ea', cyan: '#89ddff', white: '#ffffff', brightBlack: '#546e7a', brightRed: '#ff5370', brightGreen: '#c3e88d', brightYellow: '#ffcb6b', brightBlue: '#82aaff', brightMagenta: '#c792ea', brightCyan: '#89ddff', brightWhite: '#ffffff' },
  },
];

export const lightThemes: ThemePreviewMeta[] = [
  {
    id: 'tokyo-night-day',
    name: 'Tokyo Night Day',
    tone: 'light',
    preview: { background: '#e1e2e7', lineStrong: '#3760bf', lineMedium: '#6172b0', lineWeak: '#a8aecb', accent: '#2e7de9' },
    ui: { pageBg: '#d4d5db', sidebarBg: '#d8d9e0', mainBg: '#dce0e8', cardBg: '#e1e2e7', headerBg: '#dde0e6', border: 'rgba(55, 96, 191, 0.1)', borderStrong: 'rgba(55, 96, 191, 0.18)', inputBg: 'rgba(255, 255, 255, 0.7)', inputBorder: 'rgba(55, 96, 191, 0.14)', navHover: 'rgba(55, 96, 191, 0.04)' },
    xterm: { background: '#e1e2e7', foreground: '#3760bf', cursor: '#3760bf', selectionBackground: '#b4b8d8', black: '#e9e9ed', red: '#f52a65', green: '#587539', yellow: '#8c6c3e', blue: '#2e7de9', magenta: '#9854f1', cyan: '#007197', white: '#6172b0', brightBlack: '#a1a6c5', brightRed: '#f52a65', brightGreen: '#587539', brightYellow: '#8c6c3e', brightBlue: '#2e7de9', brightMagenta: '#9854f1', brightCyan: '#007197', brightWhite: '#3760bf' },
  },
  {
    id: 'catppuccin-latte',
    name: 'Catppuccin Latte',
    tone: 'light',
    preview: { background: '#eff1f5', lineStrong: '#4c4f69', lineMedium: '#7c7f93', lineWeak: '#bcc0cc', accent: '#7287fd' },
    ui: { pageBg: '#e4e6ec', sidebarBg: '#e8eaf0', mainBg: '#eceef4', cardBg: '#eff1f5', headerBg: '#eef0f6', border: 'rgba(76, 79, 105, 0.1)', borderStrong: 'rgba(76, 79, 105, 0.16)', inputBg: '#fff', inputBorder: 'rgba(76, 79, 105, 0.14)', navHover: 'rgba(114, 135, 253, 0.06)' },
    xterm: { background: '#eff1f5', foreground: '#4c4f69', cursor: '#dc8a78', selectionBackground: '#ccd0da', black: '#5c5f77', red: '#d20f39', green: '#40a02b', yellow: '#df8e1d', blue: '#1e66f5', magenta: '#ea76cb', cyan: '#179299', white: '#4c4f69', brightBlack: '#6c6f85', brightRed: '#d20f39', brightGreen: '#40a02b', brightYellow: '#df8e1d', brightBlue: '#1e66f5', brightMagenta: '#ea76cb', brightCyan: '#179299', brightWhite: '#4c4f69' },
  },
  {
    id: 'github-light',
    name: 'GitHub Light',
    tone: 'light',
    preview: { background: '#ffffff', lineStrong: '#1f2328', lineMedium: '#656d76', lineWeak: '#b1bac4', accent: '#0969da' },
    ui: { pageBg: '#f6f8fa', sidebarBg: '#f0f2f5', mainBg: '#f6f8fa', cardBg: '#fff', headerBg: '#f6f8fa', border: 'rgba(31, 35, 40, 0.08)', borderStrong: 'rgba(31, 35, 40, 0.14)', inputBg: '#fff', inputBorder: 'rgba(31, 35, 40, 0.12)', navHover: 'rgba(9, 105, 218, 0.04)' },
    xterm: { background: '#ffffff', foreground: '#1f2328', cursor: '#1f2328', selectionBackground: '#b6e3ff', black: '#24292f', red: '#cf222e', green: '#116329', yellow: '#4d2d00', blue: '#0969da', magenta: '#8250df', cyan: '#1b7c83', white: '#6e7781', brightBlack: '#57606a', brightRed: '#a40e26', brightGreen: '#1a7f37', brightYellow: '#633c01', brightBlue: '#218bff', brightMagenta: '#a475f4', brightCyan: '#3192aa', brightWhite: '#8c959f' },
  },
  {
    id: 'solarized-light',
    name: 'Solarized Light',
    tone: 'light',
    preview: { background: '#fdf6e3', lineStrong: '#586e75', lineMedium: '#839496', lineWeak: '#d3cbb7', accent: '#268bd2' },
    ui: { pageBg: '#f0e8d4', sidebarBg: '#f4ecda', mainBg: '#f7f0e0', cardBg: '#fdf6e3', headerBg: '#faf2e2', border: 'rgba(88, 110, 117, 0.1)', borderStrong: 'rgba(88, 110, 117, 0.16)', inputBg: '#fffdf6', inputBorder: 'rgba(88, 110, 117, 0.16)', navHover: 'rgba(38, 139, 210, 0.04)' },
    xterm: { background: '#fdf6e3', foreground: '#586e75', cursor: '#586e75', selectionBackground: '#eee8d5', black: '#073642', red: '#dc322f', green: '#859900', yellow: '#b58900', blue: '#268bd2', magenta: '#d33682', cyan: '#2aa198', white: '#eee8d5', brightBlack: '#002b36', brightRed: '#cb4b16', brightGreen: '#586e75', brightYellow: '#657b83', brightBlue: '#839496', brightMagenta: '#6c71c4', brightCyan: '#93a1a1', brightWhite: '#fdf6e3' },
  },
  {
    id: 'gruvbox-light',
    name: 'Gruvbox Light',
    tone: 'light',
    preview: { background: '#fbf1c7', lineStrong: '#3c3836', lineMedium: '#504945', lineWeak: '#bdae93', accent: '#076678' },
    ui: { pageBg: '#f0e6b2', sidebarBg: '#f4ecc0', mainBg: '#f7f0cc', cardBg: '#fbf1c7', headerBg: '#f8f2c8', border: 'rgba(60, 56, 54, 0.1)', borderStrong: 'rgba(60, 56, 54, 0.16)', inputBg: '#fffdf2', inputBorder: 'rgba(60, 56, 54, 0.14)', navHover: 'rgba(7, 102, 120, 0.04)' },
    xterm: { background: '#fbf1c7', foreground: '#3c3836', cursor: '#3c3836', selectionBackground: '#ebdbb2', black: '#fbf1c7', red: '#9d0006', green: '#79740e', yellow: '#b57614', blue: '#076678', magenta: '#8f3f71', cyan: '#427b58', white: '#504945', brightBlack: '#928374', brightRed: '#9d0006', brightGreen: '#79740e', brightYellow: '#b57614', brightBlue: '#076678', brightMagenta: '#8f3f71', brightCyan: '#427b58', brightWhite: '#3c3836' },
  },
  {
    id: 'one-light',
    name: 'One Light',
    tone: 'light',
    preview: { background: '#fafafa', lineStrong: '#383a42', lineMedium: '#696c77', lineWeak: '#d4d4d6', accent: '#4078f2' },
    ui: { pageBg: '#f0f0f2', sidebarBg: '#f3f3f5', mainBg: '#f5f5f7', cardBg: '#fafafa', headerBg: '#f7f7f9', border: 'rgba(56, 58, 66, 0.08)', borderStrong: 'rgba(56, 58, 66, 0.14)', inputBg: '#fff', inputBorder: 'rgba(56, 58, 66, 0.12)', navHover: 'rgba(64, 120, 242, 0.04)' },
    xterm: { background: '#fafafa', foreground: '#383a42', cursor: '#526fff', selectionBackground: '#e5e5e6', black: '#383a42', red: '#e45649', green: '#50a14f', yellow: '#c18401', blue: '#4078f2', magenta: '#a626a4', cyan: '#0184bc', white: '#a0a1a7', brightBlack: '#4f525e', brightRed: '#e06c75', brightGreen: '#98c379', brightYellow: '#e5c07b', brightBlue: '#61afef', brightMagenta: '#c678dd', brightCyan: '#56b6c2', brightWhite: '#ffffff' },
  },
];

export const allThemes: ThemePreviewMeta[] = [...darkThemes, ...lightThemes];

export const accentColors: AccentColorMeta[] = [
  { id: 'blue', name: 'Blue', base: '#1677ff', strong: '#0958d9' },
  { id: 'sky', name: 'Sky', base: '#0ea5e9', strong: '#0284c7' },
  { id: 'cyan', name: 'Cyan', base: '#06b6d4', strong: '#0891b2' },
  { id: 'teal', name: 'Teal', base: '#14b8a6', strong: '#0d9488' },
  { id: 'emerald', name: 'Emerald', base: '#10b981', strong: '#059669' },
  { id: 'green', name: 'Green', base: '#22c55e', strong: '#16a34a' },
  { id: 'amber', name: 'Amber', base: '#f59e0b', strong: '#d97706' },
  { id: 'orange', name: 'Orange', base: '#f97316', strong: '#ea580c' },
  { id: 'red', name: 'Red', base: '#ef4444', strong: '#dc2626' },
  { id: 'rose', name: 'Rose', base: '#f43f5e', strong: '#e11d48' },
  { id: 'pink', name: 'Pink', base: '#ec4899', strong: '#db2777' },
  { id: 'violet', name: 'Violet', base: '#8b5cf6', strong: '#7c3aed' },
  { id: 'indigo', name: 'Indigo', base: '#6366f1', strong: '#4f46e5' },
];

export function getThemeById(id: string): ThemePreviewMeta | undefined {
  return allThemes.find((t) => t.id === id);
}

export function getAccentById(id: string): AccentColorMeta | undefined {
  return accentColors.find((c) => c.id === id);
}
