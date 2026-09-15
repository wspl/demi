import { isAbortError, throwIfAborted, type ActivityGate } from '@demicodes/utils'

const FRAME_ADMISSION_TIMEOUT_MS = 30_000

export type FrameAdmission = (id: string, signal: AbortSignal) => Promise<(() => void) | null>

/** Admits conversation frames after idle retirement, refusing forced transitions. */
export async function admitConversationFrame(
  gate: ActivityGate,
  signal: AbortSignal,
): Promise<(() => void) | null> {
  throwIfAborted(signal)
  if (gate.reservationPurpose !== 'idle')
    return gate.tryEnter()

  const wait = new AbortController()
  const timer = setTimeout(() => wait.abort(), FRAME_ADMISSION_TIMEOUT_MS)
  const unsubscribe = gate.subscribe(() => {
    if (gate.reservationPurpose === 'forced')
      wait.abort()
  })
  try {
    return await gate.enter(AbortSignal.any([signal, wait.signal]))
  } catch (error) {
    // Only this entrance's bound or a forced transition becomes a busy refusal.
    // Caller cancellation and unexpected failures retain their own errors.
    if (isAbortError(error) && wait.signal.aborted && !signal.aborted)
      return null
    throw error
  } finally {
    clearTimeout(timer)
    unsubscribe()
  }
}
