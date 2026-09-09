import { onBeforeUnmount, ref } from 'vue'

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
  function reset(next: PairingPhase = { kind: 'setup' }) {
    cancelRequest()
    generation++
    phase.value = next
  }
  function close() {
    cancelRequest()
    generation++
    isOpen.value = false
  }
  function open() {
    cancelRequest()
    generation++
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
    phase.value = result.ok
      ? {
          kind: 'done',
          device: result.device,
        }
      : {
          kind: 'code',
          error: pairingErrors[result.code],
        }
  }
  onBeforeUnmount(close)
  return {
    isOpen,
    phase,
    open,
    close,
    reset,
    submit,
  }
}
