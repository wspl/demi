import type { FlashKind } from './types'

const activeAnimations = new WeakMap<HTMLElement, Animation>()

const framesByKind: Record<FlashKind, Keyframe[]> = {
  range: [
    { backgroundColor: 'color-mix(in srgb, var(--color-on-accent) 26%, transparent)' },
    { backgroundColor: 'color-mix(in srgb, var(--color-on-accent) 12%, transparent)', offset: 0.55 },
    { backgroundColor: 'transparent' },
  ],
  'diff-changed': [
    { backgroundColor: 'color-mix(in srgb, var(--color-on-accent) 28%, transparent)' },
    { backgroundColor: 'color-mix(in srgb, var(--color-on-success) 18%, transparent)', offset: 0.55 },
    { backgroundColor: 'color-mix(in srgb, var(--color-on-success) 15%, transparent)' },
  ],
  'diff-deleted': [
    { backgroundColor: 'color-mix(in srgb, var(--color-on-accent) 28%, transparent)' },
    { backgroundColor: 'color-mix(in srgb, var(--color-on-danger) 18%, transparent)', offset: 0.55 },
    { backgroundColor: 'color-mix(in srgb, var(--color-on-danger) 12%, transparent)' },
  ],
}

export function nextReplayToken(previous?: number) {
  return (previous ?? 0) + 1
}

export function clearFlashMarkers(root: ParentNode) {
  for (const node of root.querySelectorAll<HTMLElement>('[data-nav-flash-kind], [data-nav-flash-token]')) {
    delete node.dataset['navFlashKind']
    delete node.dataset['navFlashToken']
  }
}

export function flashElements(
  elements: Iterable<HTMLElement>,
  kind: FlashKind,
  token: number,
  durationMs = 800,
) {
  const nodes = [...elements]
  for (const node of nodes) {
    node.dataset['navFlashKind'] = kind
    node.dataset['navFlashToken'] = String(token)
    activeAnimations.get(node)?.cancel()
    const animation = node.animate(framesByKind[kind], {
      duration: durationMs,
      easing: 'ease-out',
      fill: 'none',
    })
    activeAnimations.set(node, animation)
    animation.finished
      .catch(() => {})
      .finally(() => {
        if (node.dataset['navFlashToken'] !== String(token)) return
        delete node.dataset['navFlashKind']
        delete node.dataset['navFlashToken']
        if (activeAnimations.get(node) === animation) {
          activeAnimations.delete(node)
        }
      })
  }
}
