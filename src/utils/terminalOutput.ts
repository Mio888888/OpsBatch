export interface TerminalOutputPumpOptions {
  write: (data: string) => void;
  closeMessage: string;
  maxChunkChars?: number;
  active?: boolean;
}

export interface TerminalOutputPump {
  enqueue: (data: string) => void;
  setActive: (active: boolean) => void;
  close: () => void;
  dispose: () => void;
}

const DEFAULT_MAX_CHUNK_CHARS = 12 * 1024;
const QUEUE_COMPACT_THRESHOLD = 64;

const ACTIVE_CONFIG = {
  maxCharsPerFrame: 48 * 1024,
  maxFrameMs: 6,
  directWriteThreshold: 256,
};

const INACTIVE_CONFIG = {
  maxCharsPerFrame: 16 * 1024,
  maxFrameMs: 3,
  directWriteThreshold: 0,
};

export function createTerminalOutputPump(options: TerminalOutputPumpOptions): TerminalOutputPump {
  const maxChunkChars = Math.max(1, Math.min(options.maxChunkChars ?? DEFAULT_MAX_CHUNK_CHARS, ACTIVE_CONFIG.maxCharsPerFrame));

  const queue: string[] = [];
  let queueHead = 0;
  let headOffset = 0;
  let rafId: number | null = null;
  let fallbackTimer: number | null = null;
  let disposed = false;
  let closePending = false;
  let activeConfig = options.active !== false ? ACTIVE_CONFIG : INACTIVE_CONFIG;

  const hasQueuedOutput = () => queueHead < queue.length;

  const clearScheduledFlush = () => {
    if (rafId !== null) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
    if (fallbackTimer !== null) {
      window.clearTimeout(fallbackTimer);
      fallbackTimer = null;
    }
  };

  const compactQueue = () => {
    if (queueHead < QUEUE_COMPACT_THRESHOLD || queueHead * 2 < queue.length) {
      return;
    }
    queue.splice(0, queueHead);
    queueHead = 0;
  };

  const writeNextChunk = (remainingChars: number) => {
    const current = queue[queueHead];
    if (current === undefined) {
      return 0;
    }

    const availableChars = current.length - headOffset;
    if (availableChars <= 0) {
      queueHead += 1;
      headOffset = 0;
      compactQueue();
      return 0;
    }

    const charsToWrite = Math.min(remainingChars, maxChunkChars, availableChars);
    options.write(current.slice(headOffset, headOffset + charsToWrite));
    headOffset += charsToWrite;

    if (headOffset >= current.length) {
      queueHead += 1;
      headOffset = 0;
      compactQueue();
    }

    return charsToWrite;
  };

  const flushFrame = () => {
    rafId = null;
    if (fallbackTimer !== null) {
      window.clearTimeout(fallbackTimer);
      fallbackTimer = null;
    }
    if (disposed) {
      return;
    }

    const startedAt = performance.now();
    let writtenChars = 0;

    while (hasQueuedOutput() && writtenChars < activeConfig.maxCharsPerFrame) {
      const remainingChars = activeConfig.maxCharsPerFrame - writtenChars;
      const chunkChars = writeNextChunk(remainingChars);
      if (chunkChars === 0) {
        continue;
      }

      writtenChars += chunkChars;
      if (performance.now() - startedAt >= activeConfig.maxFrameMs) {
        break;
      }
    }

    if (!hasQueuedOutput() && closePending) {
      closePending = false;
      options.write(options.closeMessage);
    }

    if (hasQueuedOutput() || closePending) {
      scheduleFlush();
    }
  };

  const scheduleFlush = () => {
    if (disposed || rafId !== null || fallbackTimer !== null || (!hasQueuedOutput() && !closePending)) {
      return;
    }
    rafId = requestAnimationFrame(flushFrame);
    fallbackTimer = window.setTimeout(() => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      flushFrame();
    }, 50);
  };

  return {
    enqueue(data: string) {
      if (disposed || data.length === 0) {
        return;
      }
      if (!hasQueuedOutput() && !closePending && data.length <= activeConfig.directWriteThreshold) {
        options.write(data);
        return;
      }
      queue.push(data);
      scheduleFlush();
    },
    setActive(isActive: boolean) {
      activeConfig = isActive ? ACTIVE_CONFIG : INACTIVE_CONFIG;
    },
    close() {
      if (disposed) {
        return;
      }
      closePending = true;
      scheduleFlush();
    },
    dispose() {
      disposed = true;
      closePending = false;
      queue.length = 0;
      queueHead = 0;
      headOffset = 0;
      clearScheduledFlush();
    },
  };
}
