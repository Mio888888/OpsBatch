export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

const ANSI_COLORS: Record<string, string> = {
  '30': '#4e4e4e', '31': '#ff6b6b', '32': '#51cf66', '33': '#fcc419',
  '34': '#339af0', '35': '#cc5de8', '36': '#22b8cf', '37': '#dee2e6',
  '90': '#636363', '91': '#ff8787', '92': '#69db7c', '93': '#ffe066',
  '94': '#74c0fc', '95': '#e599f7', '96': '#66d9e8', '97': '#ffffff',
  '1;30': '#4e4e4e', '1;31': '#ff6b6b', '1;32': '#51cf66', '1;33': '#fcc419',
  '1;34': '#339af0', '1;35': '#cc5de8', '1;36': '#22b8cf', '1;37': '#ffffff',
};

export function ansiToHtml(text: string): string {
  return text.replace(/\x1b\[([0-9;]*)m/g, (_match, code: string) => {
    if (code === '' || code === '0') return '</span>';
    const color = ANSI_COLORS[code];
    if (color) return `</span><span style="color:${color}">`;
    return '';
  });
}

export function renderAnsiOutput(text: string) {
  if (!text) return null;
  const wrapped = `<span>${ansiToHtml(escapeHtml(text))}</span>`;
  return <span dangerouslySetInnerHTML={{ __html: wrapped }} />;
}

export const RISK_COLORS: Record<string, string> = {
  low: 'green',
  medium: 'orange',
  high: 'red',
  critical: 'magenta',
};
