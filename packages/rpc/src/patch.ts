import type { Block } from '@demi/core'
import type { TranscriptPatch } from './frames'

export function diffTranscriptBlocks(previous: Block[], next: Block[]): TranscriptPatch[] {
  let prefix = 0
  while (prefix < previous.length && prefix < next.length && blocksEqual(previous[prefix], next[prefix])) {
    prefix += 1
  }

  if (prefix === previous.length && prefix === next.length) return []

  const canSpliceTail =
    previous.slice(0, prefix).every((block, index) => blocksEqual(block, next[index])) &&
    previous.slice(prefix).length + next.slice(prefix).length < Math.max(previous.length, next.length) + 2

  if (!canSpliceTail) return [{ op: 'replace', path: ['blocks'], value: next }]

  const patches: TranscriptPatch[] = []
  for (let index = previous.length - 1; index >= prefix; index -= 1) {
    patches.push({ op: 'remove', path: ['blocks', index] })
  }
  for (let index = prefix; index < next.length; index += 1) {
    patches.push({ op: 'add', path: ['blocks', index], value: next[index] })
  }
  return patches
}

export function applyTranscriptPatches(blocks: Block[], patches: TranscriptPatch[]): Block[] {
  const next = [...blocks]
  for (const patch of patches) {
    if (patch.op === 'replace') return [...patch.value]
    const index = patch.path[1]
    if (patch.op === 'remove') next.splice(index, 1)
    else next.splice(index, 0, patch.value)
  }
  return next
}

export function cloneBlocks(blocks: Block[]): Block[] {
  return structuredClone(blocks)
}

function blocksEqual(left: Block, right: Block): boolean {
  return deepEqual(left, right, new WeakMap<object, WeakSet<object>>())
}

function deepEqual(left: unknown, right: unknown, seen: WeakMap<object, WeakSet<object>>): boolean {
  if (Object.is(left, right)) return true
  if (typeof left !== 'object' || left === null || typeof right !== 'object' || right === null) return false

  const seenRight = seen.get(left)
  if (seenRight?.has(right)) return true
  if (seenRight) seenRight.add(right)
  else seen.set(left, new WeakSet([right]))

  if (left instanceof Uint8Array || right instanceof Uint8Array) {
    if (!(left instanceof Uint8Array) || !(right instanceof Uint8Array)) return false
    if (left.byteLength !== right.byteLength) return false
    return left.every((byte, index) => byte === right[index])
  }

  if (left instanceof Date || right instanceof Date) {
    return left instanceof Date && right instanceof Date && left.getTime() === right.getTime()
  }

  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false
    return left.every((value, index) => deepEqual(value, right[index], seen))
  }

  if (left instanceof Map || right instanceof Map) {
    if (!(left instanceof Map) || !(right instanceof Map) || left.size !== right.size) return false
    const leftEntries = [...left.entries()]
    const rightEntries = [...right.entries()]
    return leftEntries.every(([leftKey, leftValue], index) => {
      const [rightKey, rightValue] = rightEntries[index]
      return deepEqual(leftKey, rightKey, seen) && deepEqual(leftValue, rightValue, seen)
    })
  }

  if (left instanceof Set || right instanceof Set) {
    if (!(left instanceof Set) || !(right instanceof Set) || left.size !== right.size) return false
    const leftValues = [...left.values()]
    const rightValues = [...right.values()]
    return leftValues.every((value, index) => deepEqual(value, rightValues[index], seen))
  }

  const leftKeys = Object.keys(left)
  const rightKeys = Object.keys(right)
  if (leftKeys.length !== rightKeys.length) return false
  return leftKeys.every((key) => {
    return Object.hasOwn(right, key) && deepEqual((left as Record<string, unknown>)[key], (right as Record<string, unknown>)[key], seen)
  })
}
