import { onScopeDispose, ref } from 'vue'

export type PairingDevice = {
  id: string
  name: string
}
export type PairingResult =
  | {
      ok: true
      device: PairingDevice
    }
  | {
      ok: false
      code: 'invalid_code' | 'rate_limited' | 'unavailable'
    }
export type PairingPhase =
  | { kind: 'setup' }
  | {
      kind: 'code'
      error?: string
    }
  | { kind: 'pairing' }
  | {
      kind: 'done'
      device: PairingDevice
    }

export const pairingErrors = {
  invalid_code:
    'This code is unavailable. Keep the runner open and paste its latest code.',
  rate_limited: 'Too many attempts. Wait a minute before trying again.',
  unavailable: 'Could not reach Demi. Check your connection and try again.',
}

/** UI lifecycle shared by settings, onboarding and host-selection entry points. */
export function useDevicePairing(
  claim: (code: string, signal?: AbortSignal) => Promise<PairingResult>,
) {
  const isOpen = ref(false)
  const phase = ref<PairingPhase>({ kind: 'setup' })
  let controller: AbortController | null = null
  function cancelRequest() {
    controller?.abort()
    controller = null
  }
  let generation = 0
  /** Hears of the device this opening pairs: the device menu it was started beside, if any. */
  let onPaired: ((device: PairingDevice) => void) | null = null
  function reset(next: PairingPhase = { kind: 'setup' }) {
    cancelRequest()
    generation++
    phase.value = next
  }
  function close() {
    cancelRequest()
    generation++
    onPaired = null
    isOpen.value = false
  }
  /** Opens on its first step; `paired` hears of the device once the claim succeeds. */
  function open(paired?: (device: PairingDevice) => void) {
    cancelRequest()
    generation++
    onPaired = paired ?? null
    phase.value = { kind: 'setup' }
    isOpen.value = true
  }
  async function submit(code: string) {
    if (phase.value.kind !== 'code' || !code.trim()) {
      return
    }
    const request = ++generation
    phase.value = { kind: 'pairing' }
    let result: PairingResult
    try {
      controller = new AbortController()
      result = await claim(code.trim(), controller.signal)
    } catch {
      result = {
        ok: false,
        code: 'unavailable',
      }
    }
    if (request !== generation) {
      return
    }
    if (!result.ok) {
      phase.value = {
        kind: 'code',
        error: pairingErrors[result.code],
      }
      return
    }
    phase.value = {
      kind: 'done',
      device: result.device,
    }
    onPaired?.(result.device)
  }
  // Disposed with the component that opened it, or with any effect scope.
  onScopeDispose(close)
  return {
    isOpen,
    phase,
    open,
    close,
    reset,
    submit,
  }
}
