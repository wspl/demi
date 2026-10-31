// An audio file's waveform, as its player draws it: the loudness of each of
// `count` equal stretches of it, 0 to 1, read once per file and kept for
// every player that shows the same file. A reading no player waits for any
// more stops, and a reading that fails is forgotten, so the next player that
// shows the file tries again.

interface Reading {
  peaks: Promise<number[] | null>
  stop: AbortController
  /** How many players wait for it. */
  waiting: number
}

/** The waveforms read or being read, by their count and the file's address. */
const readings = new Map<string, Reading>()

/**
 * The loudness of each of `count` equal stretches of the file at `src`, its
 * root mean square across all its channels, smoothed with its neighbors and
 * scaled so the loudest is 1; null for a file the browser cannot read or
 * decode, and for a reading `signal` gave up, whose player keeps its flat line.
 */
export function waveformOf(src: string, count: number, signal: AbortSignal): Promise<number[] | null> {
  const key = `${count}:${src}`
  let reading = readings.get(key)
  if (!reading) {
    const stop = new AbortController()
    const peaks = decode(src, stop.signal).then((channels) => (channels ? loudness(channels, count) : null), () => null)
    reading = { peaks, stop, waiting: 0 }
    const started = reading
    void peaks.then((read) => {
      if (read === null && readings.get(key) === started)
        readings.delete(key)
    })
    readings.set(key, reading)
  }
  const mine = reading
  mine.waiting += 1
  signal.addEventListener('abort', () => {
    mine.waiting -= 1
    if (mine.waiting === 0 && !mine.stop.signal.aborted)
      mine.stop.abort()
  }, { once: true })
  return Promise.race([mine.peaks, new Promise<null>((resolve) => signal.addEventListener('abort', () => resolve(null), { once: true }))])
}

async function decode(src: string, signal: AbortSignal): Promise<Float32Array[] | null> {
  const response = await fetch(src, { signal })
  if (!response.ok)
    return null
  const bytes = await response.arrayBuffer()
  if (signal.aborted)
    return null
  const context = new AudioContext()
  try {
    const audio = await context.decodeAudioData(bytes)
    return Array.from({ length: audio.numberOfChannels }, (_, channel) => audio.getChannelData(channel))
  } finally {
    void context.close()
  }
}

function loudness(channels: Float32Array[], count: number): number[] {
  const length = channels[0]?.length ?? 0
  const span = Math.max(1, Math.floor(length / count))
  const levels = Array.from({ length: count }, (_, i) => {
    let sum = 0
    const end = Math.min(length, (i + 1) * span)
    for (const samples of channels) {
      for (let at = i * span; at < end; at++)
        sum += samples[at]! * samples[at]!
    }
    return Math.sqrt(sum / (span * channels.length))
  })
  const smooth = levels.map((level, i) => (levels[Math.max(0, i - 1)]! + 2 * level + levels[Math.min(count - 1, i + 1)]!) / 4)
  const loudest = Math.max(...smooth) || 1
  // A gentle curve, so quiet speech still shows and pauses read as pauses.
  return smooth.map((level) => (level / loudest) ** 1.2)
}
