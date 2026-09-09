import { onBeforeUnmount, ref } from 'vue'

export type PairingDevice = {
  id: string;
  name: string
}
export type PairingResult =
  | {
    ok: true;
    device: PairingDevice
  }
  | {
    ok: false;
    code: 'invalid_code' | 'rate_limited' | 'unavailable'
  }
export type PairingPhase =
  | { kind: 'setup' }
  | {
    kind: 'code';
    error?: string
  }
  | { kind: 'pairing' }
  | {
    kind: 'done';
    device: PairingDevice
  }

export const pairingErrors = {
  invalid_code: 'This code is unavailable. Keep the runner open and paste its latest code.',
  rate_limited: 'Too many attempts. Wait a minute before trying again.',
  unavailable: 'Could not reach Demi. Check your connection and try again.',
}

/** UI lifecycle shared by settings, onboarding and host-selection entry points. */
export function useDevicePairing(claim: (code: string) => Promise<PairingResult>) {
  const isOpen = ref(false)
  const phase = ref<PairingPhase>({ kind: 'setup' })
  let generation = 0
  function reset(next: PairingPhase = { kind: 'setup' }) {
    generation++;
    phase.value = next
  }
  function close() {
    generation++;
    isOpen.value = false
  }
  function open() {
    generation++;
    phase.value = { kind: 'setup' };
    isOpen.value = true
  }
  async function submit(code: string) {
    if (phase.value.kind !== 'code' || !code.trim())
      return
    const request = ++generation
    phase.value = { kind: 'pairing' }
    let result: PairingResult
    try {
      result = await claim(code.trim())
    } catch {
      result = { ok: false, code: 'unavailable' }
    }
    if (request !== generation)
      return
    phase.value = result.ok ? { kind: 'done', device: result.device } : {
      kind: 'code',
      error: pairingErrors[result.code]
    }
  }
  onBeforeUnmount(close)
  return { isOpen, phase, open, close, reset, submit }
}
