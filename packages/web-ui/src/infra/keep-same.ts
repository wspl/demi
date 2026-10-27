import { computed, isProxy, type ComputedRef } from 'vue'

/**
 * `next`, a value a computed derives anew each time what it reads changes,
 * with every part that says what the same part of `previous` said replaced by
 * that one, and `previous` itself when all of it does: a component given the
 * value, or a part of it, is drawn again only when what it shows changed, not
 * each time the value is derived. TanStack Query keeps the data it fetches
 * again so (`replaceEqualDeep`).
 *
 * Arrays, maps, sets and plain objects are compared part by part, in
 * order; an array of objects that each have an `id` pairs them by it, so an
 * item that moved is still the one it was. Anything else, a function, a
 * component or the state's reactive objects among them, is the same only as
 * itself.
 */
export function keepSame<T>(previous: T | undefined, next: T): T {
  return kept(previous, next) as T
}

/** A computed of `derive`, kept as `keepSame` keeps it. */
export function keptComputed<T>(derive: () => T): ComputedRef<T> {
  return computed<T>((previous) => keepSame(previous, derive()))
}

function kept(previous: unknown, next: unknown): unknown {
  if (previous === next) {
    return previous
  }
  if (isList(previous) && isList(next)) {
    const byId = identified(previous) && identified(next) ? new Map(previous.map((item) => [item.id, item])) : null
    const items = next.map((item, index) => kept(byId ? byId.get((item as { id: string }).id) : previous[index], item))
    return items.length === previous.length && items.every((item, index) => item === previous[index]) ? previous : items
  }
  if (isMap(previous) && isMap(next)) {
    const before = [...previous]
    const entries = [...next].map(([key, value]): [unknown, unknown] => [key, kept(previous.get(key), value)])
    return entries.length === before.length && entries.every(([key, value], index) => before[index]![0] === key && before[index]![1] === value) ? previous : new Map(entries)
  }
  if (isSet(previous) && isSet(next)) {
    const before = [...previous]
    const items = [...next]
    return items.length === before.length && items.every((item, index) => before[index] === item) ? previous : next
  }
  if (isRecord(previous) && isRecord(next)) {
    const fields = Object.keys(next)
    const copy = Object.fromEntries(fields.map((field) => [field, kept(previous[field], next[field])]))
    return fields.length === Object.keys(previous).length && fields.every((field) => field in previous && copy[field] === previous[field]) ? previous : copy
  }
  return next
}

function isList(value: unknown): value is unknown[] {
  return Array.isArray(value) && !isProxy(value)
}

function isMap(value: unknown): value is Map<unknown, unknown> {
  return value instanceof Map && !isProxy(value)
}

function isSet(value: unknown): value is Set<unknown> {
  return value instanceof Set && !isProxy(value)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !isProxy(value) && Object.getPrototypeOf(value) === Object.prototype
}

function identified(items: readonly unknown[]): items is { id: string }[] {
  return items.every((item) => isRecord(item) && typeof item.id === 'string')
}

/**
 * `record` with the entries of `added`: `record` itself while it holds each
 * of them already, as when an update of the transcript adds no failure, so
 * the state does not change.
 */
export function withAdded<V>(record: Readonly<Record<string, V>>, added: Readonly<Record<string, V>>): Record<string, V> {
  return Object.entries(added).every(([key, value]) => key in record && record[key] === value) ? record as Record<string, V> : { ...record, ...added }
}
