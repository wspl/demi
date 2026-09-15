import { z } from 'zod'

export const BROWSER_RESOURCE_KIND = 'browser'
export const BROWSER_IDLE_MS = 10 * 60_000
export const BROWSER_TIMEOUT_MS = 30_000
export const BROWSER_MAX_TIMEOUT_MS = 300_000
export const BROWSER_MAX_NODES = 1_000
export const BROWSER_DEFAULT_NODES = 100
export const BROWSER_INLINE_BYTES = 64 * 1024

const tab = z.string().regex(/^t_[A-Za-z0-9_-]{22}$/).describe('Browser tab ID returned by open or tabs')
const reference = z.string().regex(/^e_[A-Za-z0-9_-]{22}$/)
const text = z.string().max(1024 * 1024)
const locator = z.string().min(1).max(4096)
const timeout = z.number().int().min(1).max(BROWSER_MAX_TIMEOUT_MS).optional()
  .describe('Whole operation deadline in milliseconds')
const bounded = z.number().int().min(1).max(BROWSER_MAX_NODES).optional()
const output = z.string().min(1).max(4096).optional().describe('New output file on this Host')

/** Shared element grammar; native admission also checks mutually exclusive fields. */
export const browserTargetShape = {
  ref: reference.optional().describe('A node reference returned by inspect or find'),
  role: locator.optional().describe('Accessible role, such as button or textbox'),
  name: locator.optional().describe('Accessible name, with --role'),
  label: locator.optional().describe('Associated label text (label for, wrapping label, or aria-labelledby)'),
  placeholder: locator.optional(),
  'text-match': locator.optional().describe('Visible text to match'),
  'test-id': locator.optional().describe('data-testid attribute'),
  css: locator.optional().describe('CSS selector'),
  exact: z.boolean().optional().describe('Match the complete name or text'),
  nth: z.number().int().min(0).max(BROWSER_MAX_NODES - 1).optional()
    .describe('Explicit zero-based match index'),
  within: reference.optional().describe('Container node reference'),
}
export const browserTargetSchema = z.strictObject(browserTargetShape)
export type BrowserTarget = z.infer<typeof browserTargetSchema>

const viewport = z.strictObject({ width: z.number().int().positive(), height: z.number().int().positive() })
const bounds = z.strictObject({ x: z.number(), y: z.number(), width: z.number(), height: z.number() })
export const browserNodeSchema = z.strictObject({
  ref: reference.optional(),
  role: z.string(),
  name: z.string(),
  value: z.union([z.string(), z.number()]).optional(),
  depth: z.number().int().min(0),
  states: z.array(z.string()),
  bounds: bounds.optional(),
})
export type BrowserNode = z.infer<typeof browserNodeSchema>
export const browserCreatedBySchema = z.union([
  z.strictObject({ kind: z.literal('agent'), nodeId: z.string() }),
  z.strictObject({ kind: z.literal('page'), opener: z.string() }),
  z.strictObject({ kind: z.literal('temporary'), nodeId: z.string() }),
])
export const browserTabSchema = z.strictObject({
  id: z.string(), title: z.string(), url: z.string(), createdBy: browserCreatedBySchema,
})
const info = z.strictObject({ tab: z.string(), url: z.string(), title: z.string(), viewport })
const opened = z.strictObject({ tab: z.string(), url: z.string(), title: z.string().optional(), viewport: viewport.optional() })
const navigated = z.strictObject({ tab: z.string(), url: z.string(), title: z.string().optional() })
const matches = z.strictObject({ matches: z.array(browserNodeSchema), count: z.number().int().min(0), truncated: z.boolean() })
const action = z.strictObject({
  operation: z.string(), target: z.string().optional(), result: z.unknown(),
  url: z.string().optional(), openedTabs: z.array(z.string()).optional(),
})
const load = z.enum(['commit', 'domcontentloaded', 'load']).optional()
const targeted = { tab, ...browserTargetShape, timeout }
const pointer = {
  ...targeted, xy: locator.optional().describe('Viewport CSS coordinates: x,y'),
}
const modifiers = z.array(z.enum(['Alt', 'Control', 'ControlOrMeta', 'Meta', 'Shift'])).optional()
const waitUrl = locator.optional().describe('Expected URL glob after the action')

/** Command arguments and results share this authority in declarations and native code. */
export const browserOperations = {
  open: { input: z.strictObject({ url: locator, load, timeout }), result: opened },
  tabs: { input: z.strictObject({ timeout }), result: z.strictObject({ tabs: z.array(browserTabSchema), truncated: z.boolean() }) },
  info: { input: z.strictObject({ tab, timeout }), result: info },
  goto: { input: z.strictObject({ tab, url: locator, load, timeout }), result: navigated },
  back: { input: z.strictObject({ tab, load, timeout }), result: navigated },
  forward: { input: z.strictObject({ tab, load, timeout }), result: navigated },
  reload: { input: z.strictObject({ tab, load, timeout }), result: navigated },
  history: { input: z.strictObject({ tab, timeout }), result: z.strictObject({ entries: z.array(z.strictObject({ index: z.number().int(), url: z.string(), title: z.string(), current: z.boolean() })), truncated: z.boolean() }) },
  close: { input: z.strictObject({ tab, timeout }), result: z.strictObject({ closed: z.string() }) },
  inspect: { input: z.strictObject({ tab, limit: bounded, timeout }), result: z.strictObject({ tab: z.string(), url: z.string(), title: z.string(), tree: z.array(browserNodeSchema), truncated: z.boolean() }) },
  find: { input: z.strictObject({ ...targeted, limit: bounded }), result: matches },
  read: { input: z.strictObject({ ...targeted, property: z.enum(['text', 'text-content', 'html', 'value', 'visible', 'enabled', 'checked', 'attribute']), attribute: locator.optional(), all: z.boolean().optional() }), result: z.union([z.strictObject({ value: z.unknown() }), z.strictObject({ values: z.array(z.unknown()), truncated: z.boolean() })]) },
  screenshot: { input: z.strictObject({ tab, output, 'full-page': z.boolean().optional(), timeout }), result: z.strictObject({ path: z.string(), mimeType: z.literal('image/png'), width: z.number().int().positive(), height: z.number().int().positive(), viewport }) },
  click: { input: z.strictObject({ ...pointer, count: z.number().int().min(1).max(2).optional(), button: z.enum(['left', 'middle', 'right']).optional(), modifier: modifiers, 'wait-url': waitUrl }), result: action },
  move: { input: z.strictObject(pointer), result: action },
  scroll: { input: z.strictObject({ ...pointer, dx: z.number().optional(), dy: z.number().optional() }), result: action },
  fill: { input: z.strictObject({ ...targeted, text, 'wait-url': waitUrl }), result: action },
  type: { input: z.strictObject({ ...targeted, text }), result: action },
  key: { input: z.strictObject({ ...targeted, key: locator, 'wait-url': waitUrl }), result: action },
  check: { input: z.strictObject({ ...targeted, value: z.boolean() }), result: action },
  select: { input: z.strictObject({ ...targeted, value: z.array(text).optional(), 'option-label': z.array(text).optional(), 'option-index': z.array(z.number().int().min(0)).optional() }), result: action },
  wait: { input: z.strictObject({ ...targeted, url: locator.optional(), state: z.enum(['visible', 'hidden', 'attached', 'detached', 'enabled']).optional() }), result: z.strictObject({ condition: z.string(), matched: z.boolean(), url: z.string().optional(), ref: z.string().optional() }) },
  eval: { input: z.strictObject({ tab, expression: text, timeout }), result: z.strictObject({ value: z.unknown() }) },
  'viewport.set': { input: z.strictObject({ tab, width: z.number().int().min(1).max(4096), height: z.number().int().min(1).max(4096), timeout }), result: viewport },
  'viewport.reset': { input: z.strictObject({ tab, timeout }), result: viewport },
  'dialog.inspect': { input: z.strictObject({ tab, timeout }), result: z.strictObject({ dialog: z.union([z.null(), z.strictObject({ type: z.string(), message: z.string(), defaultPrompt: z.string() })]) }) },
  'dialog.accept': { input: z.strictObject({ tab, text: text.optional(), timeout }), result: z.strictObject({ handled: z.boolean() }) },
  'dialog.dismiss': { input: z.strictObject({ tab, timeout }), result: z.strictObject({ handled: z.boolean() }) },
  'content.read': { input: z.strictObject({ tab, format: z.enum(['text', 'html', 'dom']).optional(), output, timeout }), result: z.strictObject({ tab: z.string(), url: z.string(), title: z.string(), content: text, path: z.string().optional(), truncated: z.boolean() }) },
  capabilities: { input: z.strictObject({ tab, timeout }), result: z.strictObject({ capabilities: z.array(z.strictObject({ id: z.string(), available: z.boolean(), reason: z.string().optional() })) }) },
} as const
export type BrowserOperation = keyof typeof browserOperations

/** Cold browser startup and ordinary page actions have distinct default budgets. */
export function browserDefaultTimeout(operation: string): number {
  if (operation === 'open') return BROWSER_MAX_TIMEOUT_MS
  return BROWSER_TIMEOUT_MS
}

/** Immutable Chrome archive records; installers never resolve a moving channel. */
export const browserReleaseSchema = z.strictObject({
  version: z.string().regex(/^\d+\.\d+\.\d+\.\d+$/),
  platforms: z.array(z.strictObject({
    target: z.string().min(1),
    url: z.url(),
    size: z.number().int().positive(),
    sha256: z.string().regex(/^[a-f0-9]{64}$/),
    executable: z.string().min(1),
  })),
})
export type BrowserRelease = z.infer<typeof browserReleaseSchema>
export const browserInstallationSchema = z.strictObject({
  archiveHash: z.string().regex(/^[a-f0-9]{64}$/),
  executableHash: z.string().regex(/^[a-f0-9]{64}$/),
})
export const browserRuntimeConfigSchema = z.strictObject({
  home: z.string().min(1).regex(/^[^\0]*$/),
})
