const DONE_MARKER_RE = /__OPSBATCH_AI_DONE_[a-f0-9]+__:-?\d+__/g;
const TRACKER_ARTIFACT_LINE_RE = /^.*(?:__opsbatch_ai_status=\$\?|printf ['"][^\r\n]*__OPSBATCH_AI_DONE_[a-f0-9]+__|__OPSBATCH_AI_DONE_[a-f0-9]+__:-?\d+__).*(?:\r?\n)?/gm;
const TRACKER_STATUS_TOKEN = '__opsbatch_ai_status=$?';
const TRACKER_DONE_PREFIX = '__OPSBATCH_AI_DONE_';
const PARTIAL_TRACKER_PREFIXES = [TRACKER_STATUS_TOKEN, TRACKER_DONE_PREFIX];
const PARTIAL_DONE_MARKER_RE = /__OPSBATCH_AI_DONE_[a-f0-9]{4,}/g;
const ANSI_CONTROL_PREFIX_RE = /^(?:[\r\s]|\x1b\[[0-?]*[ -/]*[@-~])*$/;

export interface TrackedCommand {
  visibleCommand: string;
  hiddenTracker: string;
}

export interface TrackedCommandOutputCleaner {
  clean: (data: string) => string;
}

export function createTrackedCommand(command: string, donePrefix: string): TrackedCommand {
  const visibleCommand = command.trimEnd();
  return {
    visibleCommand,
    hiddenTracker: `\n__opsbatch_ai_status=$?; printf '\\r\\033[K'; printf '${donePrefix}%s__\\n' "$__opsbatch_ai_status"`,
  };
}

export function stripTrackedCommandOutputArtifacts(output: string): string {
  return output.replace(TRACKER_ARTIFACT_LINE_RE, '').replace(DONE_MARKER_RE, '');
}

function findTrackerTokenIndex(text: string, fromIndex: number): number {
  const statusIndex = text.indexOf(TRACKER_STATUS_TOKEN, fromIndex);
  PARTIAL_DONE_MARKER_RE.lastIndex = fromIndex;
  const doneMatch = PARTIAL_DONE_MARKER_RE.exec(text);
  if (statusIndex === -1) {
    return doneMatch?.index ?? -1;
  }
  if (!doneMatch) {
    return statusIndex;
  }
  return Math.min(statusIndex, doneMatch.index);
}

function hasPotentialPartialTracker(text: string, index: number): boolean {
  return PARTIAL_TRACKER_PREFIXES.some((prefix) => prefix.startsWith(text.slice(index)));
}

function getTrackerPrefixSuffixLength(text: string): number {
  let longest = 0;
  for (const prefix of PARTIAL_TRACKER_PREFIXES) {
    const maxLength = Math.min(prefix.length - 1, text.length);
    for (let length = maxLength; length > longest; length -= 1) {
      if (prefix.startsWith(text.slice(-length))) {
        longest = length;
        break;
      }
    }
  }
  return longest;
}

function getLineStart(text: string, index: number): number {
  return text.lastIndexOf('\n', Math.max(0, index - 1)) + 1;
}

function getPendingStartForPartialTracker(text: string, suffixLength: number): number {
  const suffixStart = text.length - suffixLength;
  const lineStart = getLineStart(text, suffixStart);
  const prefixBeforeSuffix = text.slice(lineStart, suffixStart);
  return ANSI_CONTROL_PREFIX_RE.test(prefixBeforeSuffix) ? lineStart : suffixStart;
}

export function createTrackedCommandOutputCleaner(): TrackedCommandOutputCleaner {
  let pending = '';
  let discardingArtifactLine = false;

  return {
    clean(data: string): string {
      if (!data) {
        return '';
      }

      let source = pending + data;
      pending = '';
      let cursor = 0;
      let output = '';

      while (cursor < source.length) {
        if (discardingArtifactLine) {
          const lineEnd = source.indexOf('\n', cursor);
          if (lineEnd === -1) {
            return output;
          }
          cursor = lineEnd + 1;
          discardingArtifactLine = false;
          continue;
        }

        const trackerIndex = findTrackerTokenIndex(source, cursor);
        if (trackerIndex === -1) {
          const donePrefixIndex = source.indexOf(TRACKER_DONE_PREFIX, cursor);
          if (donePrefixIndex !== -1 && hasPotentialPartialTracker(source, donePrefixIndex)) {
            const pendingStart = getLineStart(source, donePrefixIndex);
            output += source.slice(cursor, pendingStart);
            pending = source.slice(pendingStart);
            break;
          }

          const remainder = source.slice(cursor);
          const suffixLength = getTrackerPrefixSuffixLength(remainder);
          if (suffixLength === 0) {
            output += remainder;
            break;
          }

          const pendingStartInRemainder = getPendingStartForPartialTracker(remainder, suffixLength);
          output += remainder.slice(0, pendingStartInRemainder);
          pending = remainder.slice(pendingStartInRemainder);
          break;
        }

        const lineStart = getLineStart(source, trackerIndex);
        output += source.slice(cursor, lineStart);

        const lineEnd = source.indexOf('\n', trackerIndex);
        if (lineEnd === -1) {
          discardingArtifactLine = true;
          break;
        }
        cursor = lineEnd + 1;
      }

      return stripTrackedCommandOutputArtifacts(output);
    },
  };
}
