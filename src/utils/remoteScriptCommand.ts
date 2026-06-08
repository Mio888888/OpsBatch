interface RemoteScriptParameter {
  name: string;
  defaultValue: string;
}

function quoteShellArg(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

function buildRemoteScriptArgs(
  params: RemoteScriptParameter[],
  values: Record<string, string>,
): string[] {
  const args: string[] = [];

  for (const param of params) {
    const value = values[param.name] ?? param.defaultValue ?? '';
    const trimmedValue = value.trim();
    if (!trimmedValue) continue;

    if (param.name === 'args') {
      args.push(trimmedValue);
      continue;
    }

    if (param.name.startsWith('-')) {
      args.push(param.name);
      if (trimmedValue !== 'true') {
        args.push(quoteShellArg(trimmedValue));
      }
      continue;
    }

    args.push(`${param.name}=${quoteShellArg(trimmedValue)}`);
  }

  return args;
}

export function buildRemoteScriptCommand(
  url: string,
  params: RemoteScriptParameter[],
  values: Record<string, string>,
): string {
  const args = buildRemoteScriptArgs(params, values).join(' ');

  if (args) {
    return `curl -sSL ${quoteShellArg(url)} | bash -s -- ${args}`;
  }
  return `curl -sSL ${quoteShellArg(url)} | bash`;
}
