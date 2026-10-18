import { shallowRef, type Component, type Directive, type VNode } from 'vue'
import { Copy, Download, ExternalLink, Link, PanelRight, SquareArrowOutUpRight } from '@lucide/vue'
import type { CopySource } from '../composables/useCopy'
import type { Operation } from './operations'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import type { SentenceText, TitleText } from './ui-text'

/**
 * The page's context menus (`web-application.md` § Context menus): one
 * handler at the page's root (`installContextMenus`) takes each
 * `contextmenu` event, the menu key and Shift-F10, and on a phone each long
 * press, and walks from the element under the pointer up to the root. On
 * the way it gathers the entries of content the browser's own menu would
 * offer, the selection under the pointer, a link, a picture, a code block,
 * and a link to an object (a file, a commit, an agent), whose group is that
 * object's own menu as the renderer around it answers it (`vLinkMenus`),
 * and it stops at the first object, an element that declares its menu
 * (`vContextMenu`), whose entries come last: an object's menu never holds
 * its container's. Each is a group, innermost first; the menu parts the
 * groups with dividers. A text field, Shift held, and an
 * element that stopped or prevented the event keep the browser's menu;
 * where nothing declares one, no menu opens at all. On a phone a message's
 * text keeps the system's long press, which selects words; the selection's
 * toolbar (`selectionTools`, SelectionToolbar) offers what a menu would.
 */

/** One row of a context menu: what it says and what choosing it does. */
export interface ContextMenuEntry {
  icon?: Component
  label: TitleText
  shortcut?: string
  disabled?: boolean
  /** Why it is disabled, as its tooltip. */
  disabledReason?: SentenceText
  danger?: boolean
  /**
   * What choosing the row does, which may return the promise of its work:
   * its menu row shows it working and reports its failure as a control's
   * handler does (MenuItem). A row with a submenu opens it instead.
   */
  run?: () => unknown
  /** The operation choosing the row starts, as a menu row's (`operations.ts`). */
  operation?: Operation
  /**
   * A selection entry that acts on the selected text where it stands, as a
   * comment on it does: the selection's toolbar offers it on the desktop
   * too, where every other row is the right-click menu's alone.
   */
  inPlace?: boolean
  /** The row's submenu, drawn as it opens; on a phone a sheet over the menu. */
  submenu?: () => VNode
}

/** Rows that belong together; the menu draws a divider between groups. */
export type ContextMenuGroup = readonly ContextMenuEntry[]

/** What an element answers when asked for its menu. */
export interface ContextMenuDeclaration {
  /**
   * The element's groups, read while its menu shows, so a row follows what
   * it reads (a Fork that becomes possible); none, an empty list.
   */
  entries: () => readonly ContextMenuGroup[]
  /** Called once as its menu opens, before `entries`: a row's menu selects the row first. */
  opened?: () => void
  /** Called once as its menu closes. */
  closed?: () => void
  /**
   * Its text is the user's to select, as a message's: on a phone a long
   * press there selects words, as the system does, and opens no menu.
   */
  text?: boolean
  /**
   * What the object offers for text selected in it, after the selection's
   * Copy in its menu, and on the selection's toolbar (`selectionTools`).
   */
  selectionEntries?: (text: string) => readonly ContextMenuEntry[]
}

/** What `vContextMenu` takes: a declaration, its entries alone, or nothing for no menu. */
export type ContextMenuValue = ContextMenuDeclaration | (() => readonly ContextMenuGroup[]) | null | undefined

const declarations = new WeakMap<Element, ContextMenuDeclaration>()

function declare(el: Element, value: ContextMenuValue): void {
  if (value == null) {
    declarations.delete(el)
    el.removeAttribute('data-context-menu')
    el.removeAttribute('data-context-text')
    return
  }
  const declaration = typeof value === 'function' ? { entries: value } : value
  declarations.set(el, declaration)
  el.setAttribute('data-context-menu', '')
  el.toggleAttribute('data-context-text', declaration.text === true)
}

/** An element whose menu the page shows goes: its menu goes with it. */
function undeclare(el: Element): void {
  declare(el, null)
  if (shownContextMenu.value?.menu.objects.includes(el))
    closeContextMenu()
}

/**
 * Declares an element's context menu; the root handler opens it. The plugin
 * SDK passes it on, so a plugin's objects declare theirs the same way.
 */
export const vContextMenu: Directive<Element, ContextMenuValue> = {
  mounted: (el, binding) => declare(el, binding.value),
  updated: (el, binding) => declare(el, binding.value),
  unmounted: undeclare,
}

/** What the content's own rows do, which the page supplies. */
export interface ContextMenuServices {
  copy: (source: CopySource) => void
  copyImage: (image: HTMLImageElement) => void
  save: (url: string, name: string) => void
  openLink: (url: string) => void
  /**
   * Opens a web page in a new tab of the conversation's browser, the work
   * panel shown; absent where no conversation's panel takes one, and a link
   * then has no Open in Work Panel.
   */
  openInWorkPanel?: (url: string) => void
}

/**
 * The menu of the object a link inside a rendered text names, the one its
 * row or tab declares (`web-application.md` § Context menus), read while
 * the menu shows; null for a link it knows nothing of, which then has Open
 * alone.
 */
export type LinkMenus = (link: Element) => (() => readonly ContextMenuGroup[]) | null

const linkMenus = new WeakMap<Element, LinkMenus>()

function declareLinkMenus(el: Element, value: LinkMenus | null | undefined): void {
  if (value)
    linkMenus.set(el, value)
  else
    linkMenus.delete(el)
  el.toggleAttribute('data-link-menus', !!value)
}

/**
 * Declares what the links to objects inside an element, a rendered message
 * whose links are its HTML's, offer: each object's own menu. Unlike an
 * object's menu it ends no walk, so the message around the link still adds
 * its group below.
 */
export const vLinkMenus: Directive<Element, LinkMenus | null | undefined> = {
  mounted: (el, binding) => declareLinkMenus(el, binding.value),
  updated: (el, binding) => declareLinkMenus(el, binding.value),
  unmounted: (el) => declareLinkMenus(el, null),
}

/** A menu the walk found: its groups, and the declarations it asked. */
export interface ComposedMenu {
  groups: () => ContextMenuGroup[]
  /** The elements whose declarations it asked, whose going closes it. */
  objects: readonly Element[]
  opened: () => void
  closed: () => void
}

/** Where the menu was asked for: the pointer, or none from the keyboard; and whether by a long press. */
export interface ContextMenuAsk {
  target: Element
  point: { x: number; y: number } | null
  phone: boolean
  longPress: boolean
}

/** A field whose own menu holds spelling, the system's services and Paste. */
const TEXT_FIELD = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])'
/** A control: its long press opens nothing of what holds it. */
const CONTROL = 'button, [role="button"], [role="switch"], [role="checkbox"], [role="radio"], [role="menuitem"], [role="option"], [role="slider"], summary, label'
/**
 * A link to a file, an agent, a commit or an attachment, which has no
 * address to copy: its menu is its object's, as the renderer around it
 * answers it (`vLinkMenus`), else its click is its Open.
 */
const OBJECT_LINK = '[data-agent-link][role="button"], a[data-file-link], a[data-commit-link], a[data-attachment-image], a[data-attachment-video], a[download]'

export function inTextField(target: Element): boolean {
  return target.closest(TEXT_FIELD) !== null
}

/** Text whose long press, on a phone, is the system's selection (`ContextMenuDeclaration.text`). */
function inSelectableText(target: Element): boolean {
  return target.closest('[data-context-text]') !== null
}

/** The selected text when the pointer is on it, or from the keyboard when it is in `target`. */
function selectionUnder(ask: ContextMenuAsk): string | null {
  const selection = window.getSelection()
  if (!selection || selection.isCollapsed || selection.rangeCount === 0)
    return null
  const text = selection.toString()
  if (text.trim() === '')
    return null
  const range = selection.getRangeAt(0)
  const point = ask.point
  if (point === null)
    return ask.target.contains(range.commonAncestorContainer) || range.intersectsNode(ask.target) ? text : null
  const on = [...range.getClientRects()].some((rect) =>
    point.x >= rect.left && point.x <= rect.right && point.y >= rect.top && point.y <= rect.bottom)
  return on ? text : null
}

/**
 * The groups the browser's own menu would give `el`, or null when it is no
 * such content; a link to an object gives that object's menu, as the
 * renderer around it answers it.
 */
function contentGroups(el: Element, services: ContextMenuServices): (() => readonly ContextMenuGroup[]) | null {
  if (!el.matches(OBJECT_LINK)) {
    const group = contentGroup(el, services)
    return group ? () => [group] : null
  }
  const answer = el.closest('[data-link-menus]')
  const menu = answer ? linkMenus.get(answer)?.(el) : null
  if (menu)
    return menu
  if (!(el instanceof HTMLElement))
    return null
  return () => [[{ icon: SquareArrowOutUpRight, label: t('contextMenu.open'), run: () => el.click() }]]
}

/** Whether `url` is a page of the web, which a browser's tab opens. */
function webPage(url: string): boolean {
  return url.startsWith('https://') || url.startsWith('http://')
}

/** The entries the browser's own menu would give `el`, which is no link to an object, or null. */
function contentGroup(el: Element, services: ContextMenuServices): ContextMenuGroup | null {
  if (el instanceof HTMLAnchorElement && el.href !== '') {
    const href = el.href
    const inPanel = services.openInWorkPanel
    return [
      { icon: ExternalLink, label: t('contextMenu.openLink'), run: () => services.openLink(href) },
      ...(inPanel && webPage(href)
        ? [{ icon: PanelRight, label: t('contextMenu.openInWorkPanel'), run: () => inPanel(href) }]
        : []),
      { icon: Link, label: t('contextMenu.copyLink'), run: () => services.copy(href) },
    ]
  }
  // An icon, which the page hides from assistive technology as decoration, is no picture to copy or save.
  if (el instanceof HTMLImageElement && el.currentSrc + el.src !== '' && !el.closest('[aria-hidden="true"]')) {
    const src = el.currentSrc || el.src
    const name = el.alt || src.split(/[/?#]/).filter(Boolean).at(-1) || 'image'
    return [
      { icon: Copy, label: t('contextMenu.copyImage'), run: () => services.copyImage(el) },
      { icon: Download, label: t('contextMenu.saveImage'), run: () => services.save(src, name) },
    ]
  }
  const block = el.matches('.code-block') ? el.querySelector('pre') : el.matches('pre') && !el.parentElement?.matches('.code-block') ? el : null
  if (block) {
    return [{ icon: Copy, label: t('contextMenu.copyCode'), run: () => services.copy((block.querySelector('code') ?? block).textContent ?? '') }]
  }
  return null
}

/**
 * The menu for `ask`: the selection under the pointer, then each content
 * element from the target up gives a group, up to the first object, whose
 * groups end the menu. Null when nothing does, and on a
 * long press when a control comes first that has no menu of its own.
 */
export function composeContextMenu(ask: ContextMenuAsk, services: ContextMenuServices): ComposedMenu | null {
  const parts: (() => ContextMenuGroup[])[] = []
  const asked: ContextMenuDeclaration[] = []
  const objects: Element[] = []
  const selected = selectionUnder(ask)
  if (selected !== null) {
    // The object the walk ends at adds what it offers for its selected text.
    parts.push(() => [[
      { icon: Copy, label: shared('common.copy'), run: () => services.copy(selected) },
      ...(asked[0]?.selectionEntries?.(selected) ?? []),
    ]])
  }
  for (let el: Element | null = ask.target; el; el = el.parentElement) {
    const content = contentGroups(el, services)
    const declaration = declarations.get(el)
    if (ask.longPress && parts.length === 0 && !content && !declaration && el.matches(CONTROL))
      return null
    if (content)
      parts.push(() => [...content()])
    if (declaration) {
      asked.push(declaration)
      objects.push(el)
      parts.push(() => declaration.entries().filter((group) => group.length > 0))
      // An object's menu is its own: it never holds its container's.
      break
    }
  }
  if (parts.length === 0)
    return null
  return {
    groups: () => parts.flatMap((part) => part()).filter((group) => group.length > 0),
    objects,
    opened: () => asked.forEach((declaration) => declaration.opened?.()),
    closed: () => asked.forEach((declaration) => declaration.closed?.()),
  }
}

/** A press held this long, without moving, is a long press. */
export const LONG_PRESS_MS = 500
/** A finger that moves farther than this scrolls or drags, and is no long press. */
export const LONG_PRESS_SLOP_PX = 10

/** Where the root handler opens a menu. */
export interface ContextMenuOpening {
  menu: ComposedMenu
  x: number
  y: number
  /** The element the menu follows as it scrolls. */
  anchor: Element
  /** The dialog or sheet it opened in, which it stands on rather than replaces (`overlayLayerOf`). */
  layer: OverlayLayerOf | null
}

/** A dialog's or a sheet's layer, as its element names it: its id in the overlay store and its z-index. */
export interface OverlayLayerOf {
  owner: string
  z: number
}

/** The attributes a dialog's or sheet's element carries, which a menu opened in it reads. */
export function overlayLayerAttributes(owner: string, z: number): Record<string, string> {
  return { 'data-overlay-layer': owner, 'data-overlay-z': String(z) }
}

/** The dialog or sheet `target` stands in, innermost; null on the page itself. */
export function overlayLayerOf(target: Element): OverlayLayerOf | null {
  const layer = target.closest<HTMLElement>('[data-overlay-layer]')
  const owner = layer?.dataset.overlayLayer
  return layer && owner ? { owner, z: Number(layer.dataset.overlayZ) || 0 } : null
}

export interface ContextMenuRoot {
  services: ContextMenuServices
  phone: () => boolean
  /** Shows the menu; false when it has no entries after all, and nothing opened. */
  open: (opening: ContextMenuOpening) => boolean
}

/** The element the keys are on: a list's active row (`aria-activedescendant`), or the focused element. */
function keyTarget(): Element | null {
  const focused = document.activeElement
  if (!focused || focused === document.body)
    return null
  const descendant = focused.getAttribute('aria-activedescendant')
  return (descendant ? document.getElementById(descendant) : null) ?? focused
}

/**
 * The root handler: listens on `document` until the returned function is
 * called. A `contextmenu` event reaches it after every element it passes
 * through, so one an element stopped never comes, and one it prevented is
 * left to it.
 */
export function installContextMenus(root: ContextMenuRoot): () => void {
  const listeners = new AbortController()
  const { signal } = listeners
  /** The element the menu key opened a menu for: the browser's own event for the key that follows is that one. */
  let keyed: Element | null = null
  let press: { timer: number; x: number; y: number; target: Element; opened: boolean } | null = null

  const endPress = (): void => {
    if (press)
      window.clearTimeout(press.timer)
    press = null
  }

  const ask = (target: Element, point: { x: number; y: number } | null, longPress: boolean): ComposedMenu | null =>
    composeContextMenu({ target, point, phone: root.phone(), longPress }, root.services)

  document.addEventListener('contextmenu', (event) => {
    // A long press the page opened a menu for, or Android's own for a press still held, which the page then opens.
    if (press?.opened) {
      event.preventDefault()
      return
    }
    // Android's own event for a long press still held: the page opens it as its own long press would.
    const longPress = press !== null
    endPress()
    const target = event.target instanceof Element ? event.target : null
    if (keyed && target && keyed.contains(target)) {
      event.preventDefault()
      return
    }
    keyed = null
    if (event.defaultPrevented || event.shiftKey || !target || inTextField(target))
      return
    // A phone's long press on a message's text selects words: the system keeps it.
    if (root.phone() && inSelectableText(target))
      return
    event.preventDefault()
    const menu = ask(target, { x: event.clientX, y: event.clientY }, longPress)
    if (menu)
      root.open({ menu, x: event.clientX, y: event.clientY, anchor: target, layer: overlayLayerOf(target) })
  }, { signal })

  document.addEventListener('keydown', (event) => {
    keyed = null
    if (event.defaultPrevented || (event.key !== 'ContextMenu' && !(event.key === 'F10' && event.shiftKey)))
      return
    const target = keyTarget()
    if (!target || inTextField(target))
      return
    event.preventDefault()
    keyed = target
    const menu = ask(target, null, false)
    if (!menu)
      return
    const rect = target.getBoundingClientRect()
    root.open({ menu, x: rect.left, y: rect.bottom, anchor: target, layer: overlayLayerOf(target) })
  }, { signal })

  document.addEventListener('pointerdown', () => {
    keyed = null
  }, { capture: true, signal })

  document.addEventListener('touchstart', (event) => {
    endPress()
    const finger = event.touches[0]
    const target = event.target instanceof Element ? event.target : null
    if (!root.phone() || event.touches.length !== 1 || !finger || !target || inTextField(target) || inSelectableText(target))
      return
    const x = finger.clientX
    const y = finger.clientY
    press = {
      x,
      y,
      target,
      opened: false,
      timer: window.setTimeout(() => {
        if (!press)
          return
        // A press whose element went meanwhile, as a row the list replaced, finds no declaration and opens nothing.
        const menu = ask(target, { x, y }, true)
        if (!menu) {
          press = null
          return
        }
        press.opened = root.open({ menu, x, y, anchor: target, layer: overlayLayerOf(target) })
        if (!press.opened)
          press = null
      }, LONG_PRESS_MS),
    }
  }, { passive: true, signal })
  document.addEventListener('touchmove', (event) => {
    const finger = event.touches[0]
    if (press && !press.opened && (!finger || Math.hypot(finger.clientX - press.x, finger.clientY - press.y) > LONG_PRESS_SLOP_PX))
      endPress()
  }, { passive: true, signal })
  // A scroll that starts first, of the page or of a scroller the finger is in, is no long press;
  // another scroller's, such as the sidebar revealing its open row, leaves it.
  document.addEventListener('scroll', (event) => {
    const scrolled = event.target
    if (press && !press.opened && (scrolled === document || (scrolled instanceof Node && scrolled.contains(press.target))))
      endPress()
  }, { capture: true, passive: true, signal })
  document.addEventListener('touchend', (event) => {
    // The press opened a menu: its release is no tap on what lies under it.
    if (press?.opened && event.cancelable)
      event.preventDefault()
    endPress()
    // Before the page's other release handlers, such as a screen edge's tap (`phone-page.ts`), which then see it prevented.
  }, { capture: true, passive: false, signal })
  document.addEventListener('touchcancel', endPress, { signal })

  return () => {
    listeners.abort()
    endPress()
  }
}

/** The menu the page shows now, which the page's one `ContextMenu` draws. */
export const shownContextMenu = shallowRef<ContextMenuOpening | null>(null)

/**
 * Shows `opening`'s menu in place of the one shown; an object that turns
 * out to have no entries, as text in the middle of a reply on a desktop,
 * opens nothing.
 */
export function showContextMenu(opening: ContextMenuOpening): boolean {
  closeContextMenu()
  opening.menu.opened()
  if (opening.menu.groups().length === 0) {
    opening.menu.closed()
    return false
  }
  shownContextMenu.value = opening
  return true
}

/** Closes the menu shown, if any. */
export function closeContextMenu(): void {
  const shown = shownContextMenu.value
  if (!shown)
    return
  shownContextMenu.value = null
  shown.menu.closed()
}

/** What the selection's toolbar shows: the selected text, where it is, and its tools. */
export interface SelectionTools {
  text: string
  range: Range
  entries: readonly ContextMenuEntry[]
}

/**
 * The tools for the selection `selection` holds, when it holds text inside
 * one object that offers tools for its selected text, as a message does
 * (`selectionEntries`). On the desktop they are those that act in place
 * (`inPlace`), since the right-click menu holds Copy and the rest; on a
 * phone, which has no right-click, every one of the object's, Copy being
 * the system's callout's. Null when there are none, and for any other
 * selection, one across two messages or in a text field included.
 */
export function selectionTools(selection: Selection | null, phone: boolean): SelectionTools | null {
  if (!selection || selection.isCollapsed || selection.rangeCount === 0)
    return null
  const text = selection.toString()
  if (text.trim() === '')
    return null
  const range = selection.getRangeAt(0)
  const start = range.startContainer instanceof Element ? range.startContainer : range.startContainer.parentElement
  if (!start || inTextField(start))
    return null
  for (let el: Element | null = start; el; el = el.parentElement) {
    const declaration = declarations.get(el)
    if (!declaration)
      continue
    if (!declaration.selectionEntries || !el.contains(range.endContainer))
      return null
    const entries = declaration.selectionEntries(text).filter((entry) => phone || entry.inPlace === true)
    return entries.length > 0 ? { text, range, entries } : null
  }
  return null
}
