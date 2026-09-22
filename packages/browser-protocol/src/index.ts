import { z } from 'zod'

export const BROWSER_TIMEOUT_MS = 30_000
export const BROWSER_MAX_TIMEOUT_MS = 300_000
export const BROWSER_MAX_NODES = 1_000
export const BROWSER_DEFAULT_NODES = 100
export const BROWSER_INLINE_BYTES = 64 * 1024
export const BROWSER_STDIN_BYTES = 1024 * 1024
export const BROWSER_CLIPBOARD_PNG_BYTES = 16 * 1024 * 1024
export const BROWSER_CLIPBOARD_PNG_PIXELS = 16_000_000
export const BROWSER_FETCH_URLS = 10
export const BROWSER_CONSOLE_ENTRIES = 1000
export const BROWSER_CONSOLE_BYTES = 1024 * 1024
export const BROWSER_CDP_EVENTS = 10000
export const BROWSER_CDP_BYTES = 8 * 1024 * 1024

const tab = z.string().regex(/^t_[A-Za-z0-9_-]{22}$/).describe('Browser tab ID returned by open or tabs')
const reference = z.string().regex(/^e_[A-Za-z0-9_-]{22}$/)
const text = z.string().max(BROWSER_STDIN_BYTES)
const locator = z.string().min(1).max(4096)
const timeout = z.number().int().min(1).max(BROWSER_MAX_TIMEOUT_MS).optional()
  .describe('Whole operation deadline in milliseconds')
const offset = z.number().int().min(0).optional()
const bounded = z.number().int().min(1).max(BROWSER_MAX_NODES).optional()
const output = z.string().min(1).max(4096).optional().describe('New output file on this Host')

/** Shared element grammar; native admission also checks mutually exclusive fields. */
export const browserTargetShape = {
  ref: reference.optional().describe('A node reference returned by inspect or find'),
  role: locator.optional().describe('Accessible role, ASCII case-insensitive, such as button, textbox, or date'),
  name: locator.optional().describe('Accessible name, with --role'),
  'name-pattern': locator.optional().describe('Accessible-name regular expression, with --role'),
  'text-pattern': locator.optional().describe('Rendered-text regular expression'),
  frame: z.array(reference).optional().describe('Frame references, outermost to innermost'),
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

/** Declarative browser queries are data; Rust validates base and locator combinations. */
export const browserQuerySchema = z.strictObject({
  match: browserTargetSchema.omit({ frame: true, within: true, nth: true }).optional(),
  get within() { return browserQuerySchema.optional() },
  get frame() { return browserQuerySchema.optional() },
  get and() { return z.array(browserQuerySchema).min(1).optional() },
  get or() { return z.array(browserQuerySchema).min(1).optional() },
  get has() { return browserQuerySchema.optional() },
  get hasNot() { return browserQuerySchema.optional() },
  hasText: text.optional(), hasNotText: text.optional(), visible: z.boolean().optional(),
  nth: z.number().int().min(0).optional(),
})

/**
 * A tab's viewport (`browser-live-view.md` § Modes): its CSS size, the pixel
 * ratio it renders at, and who decides them — the user's panel (`web`), a
 * phone (`mobile`), or the agent (`custom`).
 */
export const browserViewportModeSchema = z.enum(['web', 'mobile', 'custom'])
export const browserViewportSchema = z.strictObject({
  width: z.number().int().positive(),
  height: z.number().int().positive(),
  devicePixelRatio: z.number().positive(),
  mode: browserViewportModeSchema,
})
const viewport = browserViewportSchema
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
  z.strictObject({ kind: z.literal('user') }),
])
export const browserTabSchema = z.strictObject({
  id: z.string(), title: z.string(), url: z.string(), createdBy: browserCreatedBySchema,
})
const dialog = z.strictObject({ type: z.enum(['alert', 'confirm', 'prompt', 'beforeunload']), message: z.string() })
const dialogOutcome = z.strictObject({ type: dialog.shape.type, outcome: z.enum(['accepted', 'dismissed']) })
const contentFormat = z.enum(['text', 'html', 'dom'])
const fileOptions = { output, overwrite: z.boolean().optional() }
const pageList = { offset, limit: bounded }
const info = z.strictObject({ tab: z.string(), url: z.string(), title: z.string(), viewport, dialog: dialog.nullable().optional() })
const opened = z.strictObject({ tab: z.string(), url: z.string(), title: z.string().optional(), viewport: viewport.optional() })
const navigated = z.strictObject({ tab: z.string(), url: z.string(), title: z.string().optional() })
export const browserTreeNodeSchema = z.strictObject({
  ref: reference.optional(), role: z.string().optional(), name: z.string().optional(),
  value: browserNodeSchema.shape.value, tag: z.string().optional(), states: z.array(z.string()).optional(),
  get children() { return z.array(browserTreeNodeSchema).optional() },
})
const matches = z.strictObject({ matches: z.array(browserNodeSchema), count: z.number().int().min(0), truncated: z.boolean() })
const action = z.strictObject({
  operation: z.string(), target: z.string().optional(), result: z.unknown(),
  url: z.string().optional(), openedTabs: z.array(z.string()).optional(), dialog: dialog.optional(),
})
const load = z.enum(['commit', 'domcontentloaded', 'load']).optional()
const targeted = { tab, ...browserTargetShape, timeout }
const modifiers = z.array(z.enum(['Alt', 'Control', 'ControlOrMeta', 'Meta', 'Shift'])).optional()
const pointer = { ...targeted, xy: locator.optional().describe('Viewport CSS coordinates: x,y'), modifier: modifiers }
const waitUrl = locator.optional().describe('Expected URL glob after the action')
const mime = z.enum(['text/plain', 'text/html', 'image/png'])
const outputDir = locator.describe('Output directory on the invoking Host')
const cursor = locator.optional().describe('Cursor returned by a previous read')
const assetKind = z.enum(['font', 'image', 'stylesheet', 'video'])
const asset = z.strictObject({ id: z.string(), kind: assetKind, url: z.string(), mimeType: z.string().optional() })
const logEntry = z.strictObject({ sequence: z.number().int().min(0), level: z.string(), text: z.string(), url: z.string().optional(), timestamp: z.number() })
const cdpEvent = z.strictObject({ sequence: z.number().int().min(0), method: z.string(), params: z.unknown(), target: z.string() })
export const browserErrorCodeSchema = z.enum([
  'invalid_input', 'tab_not_found', 'tab_busy', 'stale_ref', 'stale_cursor',
  'stale_inventory', 'stale_tools', 'target_not_found', 'ambiguous_target', 'not_actionable',
  'timeout', 'dialog_blocked', 'dialog_not_found', 'invalid_dialog_action', 'history_boundary',
  'navigation_failed', 'protected_value', 'side_effect_rejected', 'unsupported_result',
  'unsupported_capability', 'cdp_method_denied', 'output_exists', 'io_error', 'result_too_large',
  'partial_failure', 'driver_error', 'browser_unavailable', 'browser_lost', 'cancelled', 'outcome_unknown',
])
export const browserErrorSchema = z.strictObject({ code: browserErrorCodeSchema, message: z.string(), details: z.record(z.string(), z.unknown()).optional() })

/** Command arguments and results share this authority in declarations and native code. */
export const browserOperations = {
  open: { input: z.strictObject({ url: locator, load, timeout }), result: opened },
  tabs: { input: z.strictObject({ ...pageList, timeout }), result: z.strictObject({ tabs: z.array(browserTabSchema), truncated: z.boolean() }) },
  info: { input: z.strictObject({ tab, timeout }), result: info },
  goto: { input: z.strictObject({ tab, url: locator, load, timeout }), result: navigated },
  back: { input: z.strictObject({ tab, load, timeout }), result: navigated },
  forward: { input: z.strictObject({ tab, load, timeout }), result: navigated },
  reload: { input: z.strictObject({ tab, load, timeout }), result: navigated },
  history: { input: z.strictObject({ tab, ...pageList, timeout }), result: z.strictObject({ entries: z.array(z.strictObject({ index: z.number().int(), url: z.string(), title: z.string(), current: z.boolean() })), truncated: z.boolean() }) },
  close: { input: z.strictObject({ tab, timeout }), result: z.strictObject({ closed: z.string() }) },
  inspect: { input: z.strictObject({ tab, view: z.enum(['accessibility', 'dom']).optional(), within: reference.optional(), frame: browserTargetShape.frame, limit: bounded, timeout }), result: z.strictObject({ tab: z.string(), url: z.string(), title: z.string(), view: z.enum(['accessibility', 'dom']), tree: z.array(browserTreeNodeSchema), truncated: z.boolean() }) },
  find: { input: z.strictObject({ ...targeted, ...pageList, query: z.boolean().optional().describe('Read a declarative query tree from stdin'), body: text.optional().describe('JSON query tree when --query is supplied') }), result: matches },
  read: { input: z.strictObject({ ...targeted, property: z.enum(['text', 'text-content', 'html', 'value', 'visible', 'enabled', 'checked']).optional(), attribute: locator.optional(), all: z.boolean().optional() }), result: z.union([z.strictObject({ value: z.unknown() }), z.strictObject({ values: z.array(z.unknown()), truncated: z.boolean() })]) },
  screenshot: { input: z.strictObject({ tab, ...fileOptions, 'full-page': z.boolean().optional(), clip: locator.optional().describe('CSS rectangle: x,y,width,height'), timeout }), result: z.strictObject({ path: z.string(), mimeType: z.literal('image/png'), width: z.number().int().positive(), height: z.number().int().positive(), viewport }) },
  probe: { input: z.strictObject({ tab, xy: locator, 'include-non-interactable': z.boolean().optional(), ...fileOptions, timeout }), result: z.strictObject({ matches: z.array(browserNodeSchema), viewport, path: z.string().optional(), truncated: z.boolean() }) },
  click: { input: z.strictObject({ ...pointer, count: z.number().int().min(1).max(2).optional(), button: z.enum(['left', 'middle', 'right']).optional(), 'wait-url': waitUrl }), result: action },
  move: { input: z.strictObject(pointer), result: action },
  drag: { input: z.strictObject({ tab, point: z.array(locator).min(2), modifier: modifiers, timeout }), result: action },
  scroll: { input: z.strictObject({ ...pointer, dx: z.number().optional(), dy: z.number().optional() }), result: action },
  fill: { input: z.strictObject({ ...targeted, text, 'wait-url': waitUrl }), result: action },
  type: { input: z.strictObject({ ...targeted, text }), result: action },
  key: { input: z.strictObject({ ...targeted, key: locator, 'wait-url': waitUrl }), result: action },
  check: { input: z.strictObject({ ...targeted, value: z.boolean() }), result: action },
  select: { input: z.strictObject({ ...targeted, value: z.array(text).optional(), 'option-label': z.array(text).optional(), 'option-index': z.array(z.number().int().min(0)).optional() }), result: action },
  'select-text': { input: z.strictObject({ ...targeted, text, cursor: z.enum(['before', 'after']).optional(), prefix: text.optional(), suffix: text.optional() }), result: action },
  wait: { input: z.strictObject({ ...targeted, url: locator.optional(), load, state: z.enum(['visible', 'hidden', 'attached', 'detached', 'enabled']).optional() }), result: z.strictObject({ condition: z.string(), matched: z.boolean(), url: z.string().optional(), ref: z.string().optional() }) },
  upload: { input: z.strictObject({ ...targeted, file: z.array(locator).min(1) }), result: z.strictObject({ files: z.array(z.string()), attached: z.number().int().min(0) }) },
  download: { input: z.strictObject({ ...pointer, ...fileOptions }), result: z.strictObject({ path: z.string(), suggestedFilename: z.string(), bytes: z.number().int().min(0), mimeType: z.string() }) },
  'clipboard.write': { input: z.strictObject({ tab, mime: mime.optional(), timeout }), result: z.strictObject({ mimeType: mime, bytes: z.number().int().min(0) }) },
  'clipboard.read': { input: z.strictObject({ tab, format: z.enum(['text']).optional(), 'output-dir': outputDir.optional(), overwrite: z.boolean().optional(), timeout }), result: z.union([z.strictObject({ text: z.string() }), z.strictObject({ items: z.array(z.strictObject({ mimeType: mime, path: z.string(), bytes: z.number().int().min(0) })) })]) },
  eval: { input: z.strictObject({ ...targeted, expression: text, all: z.boolean().optional() }), result: z.strictObject({ value: z.unknown() }) },
  logs: { input: z.strictObject({ tab, level: z.array(z.enum(['debug', 'info', 'log', 'warning', 'error'])).optional(), filter: text.optional(), after: cursor, limit: bounded, timeout }), result: z.strictObject({ entries: z.array(logEntry), cursor: z.string(), hasMore: z.boolean(), truncated: z.boolean() }) },
  'viewport.set': { input: z.strictObject({ tab, width: z.number().int().min(1).max(4096), height: z.number().int().min(1).max(4096), scale: z.number().min(0.5).max(4).optional().describe('Device pixel ratio, 1 by default'), timeout }), result: z.strictObject({ viewport }) },
  'viewport.reset': { input: z.strictObject({ tab, timeout }), result: z.strictObject({ viewport }) },
  'dialog.inspect': { input: z.strictObject({ tab, timeout }), result: z.strictObject({ dialog: dialog.nullable() }) },
  'dialog.accept': { input: z.strictObject({ tab, text: text.optional(), timeout }), result: dialogOutcome },
  'dialog.dismiss': { input: z.strictObject({ tab, timeout }), result: dialogOutcome },
  'cdp.targets': { input: z.strictObject({ tab, ...pageList, timeout }), result: z.strictObject({ targets: z.array(z.strictObject({ id: z.string(), kind: z.string(), url: z.string() })), truncated: z.boolean() }) },
  'cdp.detach': { input: z.strictObject({ tab, timeout }), result: z.strictObject({ detached: z.string() }) },
  'cdp.send': { input: z.strictObject({ tab, method: locator, params: text, target: locator.optional(), timeout }), result: z.strictObject({ method: z.string(), result: z.unknown() }) },
  'cdp.events': { input: z.strictObject({ tab, method: z.array(locator).optional(), after: cursor, limit: bounded, target: locator.optional(), timeout }), result: z.strictObject({ events: z.array(cdpEvent), cursor: z.string(), hasMore: z.boolean(), truncated: z.boolean() }) },
  'content.read': { input: z.strictObject({ tab, format: contentFormat.optional(), ...fileOptions, timeout }), result: z.union([z.strictObject({ url: z.string(), title: z.string(), format: contentFormat, content: z.string(), truncated: z.boolean() }), z.strictObject({ url: z.string(), title: z.string(), format: contentFormat, path: z.string() })]) },
  'content.fetch': { input: z.strictObject({ url: z.array(locator).min(1).max(BROWSER_FETCH_URLS), format: contentFormat.optional(), timeout }), result: z.strictObject({ pages: z.array(z.strictObject({ requestedUrl: z.string(), url: z.string(), title: z.string(), content: z.string(), error: browserErrorSchema.optional() })), truncated: z.boolean() }) },
  'assets.list': { input: z.strictObject({ tab, timeout }), result: z.strictObject({ inventory: z.string(), assets: z.array(asset), inlineSvgs: z.array(z.strictObject({ id: z.string(), html: z.string() })), truncated: z.boolean() }) },
  'assets.export': { input: z.strictObject({ tab, inventory: locator, id: z.array(locator).optional(), kind: z.array(assetKind).optional(), 'output-dir': outputDir, overwrite: z.boolean().optional(), timeout }), result: z.strictObject({ directory: z.string(), manifest: z.string(), files: z.array(z.strictObject({ id: z.string(), path: z.string(), bytes: z.number().int().min(0), mimeType: z.string() })) }) },
  capabilities: { input: z.strictObject({ tab, timeout }), result: z.strictObject({ capabilities: z.array(z.strictObject({ id: z.string(), available: z.boolean(), reason: z.string().optional(), schema: z.unknown().optional() })) }) },
  'webmcp.list': { input: z.strictObject({ tab, timeout }), result: z.strictObject({ tools: z.string(), entries: z.array(z.strictObject({ name: z.string(), description: z.string(), inputSchema: z.unknown(), outputSchema: z.unknown().optional() })), truncated: z.boolean() }) },
  'webmcp.call': { input: z.strictObject({ tab, tool: locator, tools: locator, arguments: text, timeout }), result: z.strictObject({ name: z.string(), result: z.unknown() }) },
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
