import { Extension } from '@tiptap/core'
import { Fragment, Slice, type Node as ProseMirrorNode, type ResolvedPos } from '@tiptap/pm/model'
import { Plugin, PluginKey, Selection } from '@tiptap/pm/state'
import type { EditorView } from '@tiptap/pm/view'
import { transferHasFiles } from '../../composables/useFileDrop'
import { ATTACHMENT_MARK, serializeUserMarkdown } from '../../markdown/user-markdown'
import { dataTransferFiles, pastedTextFile, pastedTextIsLong } from '../message-input/attachments'
import { transferCopiedMessage, type CopiedMessage } from '../message-input/copied-message'
import { capsuleOf, type MessageCapsule } from './capsules'
import { fragmentDocument, markdownSlice, trimmedSlice } from './clipboard-slices'

// What a composer takes in (`product.md` § Attachments): whatever is pasted
// or dropped, from Finder, a web page, another app or Demi's own transcript,
// becomes text in the user dialect or a capsule as it enters. Files and
// images whose bytes the page can read become capsules, as a pasted
// screenshot does; an image it cannot read becomes its link; rich text keeps
// the dialect's formats, which the schema's parse rules are, and every other
// element becomes its text.

/**
 * The attribute an image of the dialect, `![alt](target)`, carries in the
 * HTML a message editor writes, which is the one `<img>` the schema reads
 * back: an image from anywhere else is a file or a link, never a picture set
 * into the text.
 */
export const MESSAGE_IMAGE_ATTRIBUTE = 'data-message-image'

/** How long the page tries to read another site's image before it takes its link instead. */
const IMAGE_READ_TIMEOUT_MS = 10_000

export interface IntakeOptions {
  /** Whether the composer carries the capsule's file, as it carries one cut from this message. */
  carries: (id: string) => boolean
  /** Where the next files or pasted content land, before they are taken. */
  land: (at: number) => void
  /** Files pasted, or a paste long enough to be one; they land where `land` said. */
  files: (files: File[]) => void
  /**
   * A copied message in Demi's own form, pasted with the picture a copy of a
   * lone image carried, or dropped; it lands where `land` said.
   */
  message: (message: CopiedMessage, picture: File | null) => void
  /**
   * Text with an attachment mark where each image the page read stands, and
   * those images as files, in the order of the marks; it lands where `land`
   * said, each file uploaded as a pasted one is.
   */
  content: (markdown: string, files: File[]) => void
  /** An image's bytes, when the page can read them; `readImage` unless a test says otherwise. */
  readImage?: (src: string) => Promise<File | null>
}

/** A picture's file name, as a web browser names a pasted screenshot: `image.png`, `image.jpg`. */
function imageName(mediaType: string): string {
  const subtype = mediaType.split('/')[1]?.split(/[+;]/)[0] ?? 'png'
  return `image.${subtype === 'jpeg' ? 'jpg' : subtype}`
}

/**
 * An image's bytes as a file, when the page can read them: a `data:` or
 * `blob:` image, one of Demi's own page, or one another site lets the page
 * fetch; null for any other.
 */
export async function readImage(src: string): Promise<File | null> {
  let url: URL
  try {
    url = new URL(src, document.baseURI)
  } catch {
    // Not an address at all: nothing to read.
    return null
  }
  if (!['data:', 'blob:', 'http:', 'https:'].includes(url.protocol)) {
    return null
  }
  const own = url.origin === location.origin
  try {
    const response = await fetch(url, {
      mode: 'cors',
      credentials: own ? 'include' : 'omit',
      signal: AbortSignal.timeout(IMAGE_READ_TIMEOUT_MS),
    })
    const blob = response.ok ? await response.blob() : null
    return blob?.type.startsWith('image/') ? new File([blob], imageName(blob.type), { type: blob.type }) : null
  } catch {
    // A site that does not let the page read its image, or does not answer:
    // the image is its link, which is what the null says.
    return null
  }
}

function inCode($pos: ResolvedPos): boolean {
  return !!$pos.parent.type.spec.code
}

/** Whether parsed HTML holds anything of the dialect beyond its text: a format, a link, code, an image or a capsule. */
function holdsFormatting(slice: Slice): boolean {
  let formatted = false
  slice.content.descendants((node) => {
    formatted ||= node.marks.length > 0 || !!node.type.spec.code || node.type.name === 'image' || node.type.name === 'attachment'
    return !formatted
  })
  return formatted
}

/** HTML from anywhere, made ready for the schema to read (`prepareHtml`). */
interface PreparedHtml {
  /** The HTML as the paste or the drop brought it. */
  source: string
  html: string
  /** The images it held, under the ids of the capsules that stand for them. */
  images: Map<string, string>
  /**
   * Its text is laid out as written, as a code editor's copy is, whose
   * spaces and line breaks the schema's reading would collapse.
   */
  preformatted: boolean
}

/**
 * HTML from anywhere, made ready for the schema to read: each image that is
 * not the dialect's own stands as a capsule under a new id, and each table
 * row is a line of its cells' text.
 */
function prepareHtml(html: string): PreparedHtml {
  const page = new DOMParser().parseFromString(html, 'text/html')
  const images = new Map<string, string>()
  const preformatted = [...page.body.querySelectorAll<HTMLElement>('[style]')]
    .some((element) => element.tagName !== 'PRE' && element.style.whiteSpace.startsWith('pre'))
  for (const image of page.body.querySelectorAll('img')) {
    if (image.hasAttribute(MESSAGE_IMAGE_ATTRIBUTE)) {
      continue
    }
    const id = `incoming-${crypto.randomUUID()}`
    images.set(id, image.getAttribute('src') ?? '')
    const stand = page.createElement('span')
    stand.setAttribute('data-attachment', JSON.stringify({ id, name: 'image' }))
    image.replaceWith(stand)
  }
  for (const table of page.body.querySelectorAll('table')) {
    const lines = [...table.querySelectorAll('tr')].map((row) => {
      const line = page.createElement('p')
      for (const [index, cell] of [...row.children].entries()) {
        line.append(...(index > 0 ? [' '] : []), ...cell.childNodes)
      }
      return line
    })
    table.replaceWith(...lines)
  }
  return { source: html, html: page.body.innerHTML, images, preformatted }
}

/** The text an image the page could not read leaves: its address when it is a web address, else nothing. */
function imageLink(src: string): string {
  try {
    const url = new URL(src, document.baseURI)
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.href : ''
  } catch {
    // Not an address: nothing to link to.
    return ''
  }
}

function intakePlugin(options: IntakeOptions): Plugin {
  const readBytes = options.readImage ?? readImage
  /** The HTML ProseMirror last read from a paste or a drop, as `prepareHtml` made it ready. */
  let prepared: PreparedHtml | null = null

  /** The HTML of `transfer` as ProseMirror read it; null when it read the plain text instead. */
  function preparedFor(transfer: DataTransfer): PreparedHtml | null {
    return prepared?.source === transfer.getData('text/html') ? prepared : null
  }

  /**
   * Pasted or dropped nodes keep the capsules of files this composer
   * carries and does not have already, and those that stand for images on
   * their way in: a capsule cut from this message can be pasted back, one
   * from elsewhere has no file here to send.
   */
  function keepCapsules(fragment: Fragment, view: EditorView): Fragment {
    const present = new Set<string>()
    view.state.doc.descendants((node) => {
      const id = capsuleOf(node)?.id
      if (id) {
        present.add(id)
      }
    })
    const keep = (content: Fragment): Fragment => {
      const nodes: ProseMirrorNode[] = []
      content.forEach((node) => {
        if (node.type.name !== 'attachment') {
          nodes.push(node.isLeaf ? node : node.copy(keep(node.content)))
          return
        }
        // A capsule whose file its HTML did not say has nothing to stand for.
        const capsule = capsuleOf(node)
        if (capsule && (prepared?.images.has(capsule.id) || (options.carries(capsule.id) && !present.has(capsule.id)))) {
          present.add(capsule.id)
          nodes.push(node)
        }
      })
      return Fragment.from(nodes)
    }
    return keep(fragment)
  }

  /** Where pasted content lands: at the selection, which it replaces. */
  function landAtSelection(view: EditorView): void {
    if (!view.state.selection.empty) {
      view.dispatch(view.state.tr.deleteSelection())
    }
    options.land(view.state.selection.from)
  }

  /** Puts content in at `at`, the cursor after it, or in place of the selection. */
  function insert(view: EditorView, content: Slice, at: number | null): void {
    const tr = view.state.tr
    if (at === null) {
      tr.replaceSelection(content)
    } else {
      tr.replaceRange(at, at, content)
      tr.setSelection(Selection.near(tr.doc.resolve(tr.mapping.map(at, 1)), -1))
    }
    view.dispatch(tr.scrollIntoView())
  }

  /**
   * Text and images taken from HTML: each image the page reads becomes a
   * capsule where it stood, uploaded as a pasted file is, and each it cannot
   * read its link. Any other capsule the slice held has no file here, as a
   * copy from elsewhere has none, and stands as its name.
   */
  async function bringImages(slice: Slice, taken: Map<string, string>): Promise<void> {
    const { markdown, attachments } = serializeUserMarkdown<MessageCapsule>(fragmentDocument(slice.content))
    const files = await Promise.all(attachments.map((capsule) => {
      const src = taken.get(capsule.id)
      return src === undefined ? null : readBytes(src)
    }))
    const parts = markdown.split(ATTACHMENT_MARK)
    let text = parts[0] ?? ''
    const brought: File[] = []
    attachments.forEach((capsule, index) => {
      const file = files[index]
      const src = taken.get(capsule.id)
      if (file) {
        text += ATTACHMENT_MARK
        brought.push(file)
      } else {
        text += src === undefined ? capsule.name : imageLink(src)
      }
      text += parts[index + 1] ?? ''
    })
    options.content(text, brought)
  }

  /**
   * What a paste or a drop brings that is not a file: images become
   * capsules or links, HTML keeps the dialect's formats, and plain text is
   * read as the dialect. `at` is where a drop landed, null for a paste.
   */
  function take(view: EditorView, transfer: DataTransfer, slice: Slice, at: number | null): boolean {
    const parsed = preparedFor(transfer)
    const taken = parsed?.images ?? new Map<string, string>()
    let holdsImages = false
    slice.content.descendants((node) => {
      holdsImages ||= taken.has(capsuleOf(node)?.id ?? '')
      return !holdsImages
    })
    if (holdsImages) {
      if (at === null) {
        landAtSelection(view)
      } else {
        options.land(at)
      }
      void bringImages(slice, taken)
      return true
    }
    const html = transfer.getData('text/html')
    const text = transfer.getData('text/plain') || transfer.getData('text/uri-list').replace(/\r?\n/g, ' ')
    // HTML a message editor wrote, HTML with nothing else, or HTML whose
    // formats the text would lose; HTML laid out as written, a code
    // editor's, only colors what its text holds with its indentation.
    const readsHtml = !!parsed && (html.includes('data-pm-slice') || !text || (!parsed.preformatted && holdsFormatting(slice)))
    const { schema } = view.state
    if (readsHtml) {
      const { markdown, attachments } = serializeUserMarkdown<MessageCapsule>(fragmentDocument(slice.content))
      insert(view, markdownSlice(schema, markdown, attachments), at)
    } else {
      insert(view, markdownSlice(schema, text), at)
    }
    return true
  }

  /** What a paste brings: files, a copied message, a paste long enough to be a file, or what `take` reads. */
  function paste(view: EditorView, event: ClipboardEvent, slice: Slice): boolean {
    const transfer = event.clipboardData
    if (!transfer) {
      return false
    }
    const code = inCode(view.state.selection.$from)
    const pasted = dataTransferFiles(transfer)
    // A copied message comes back with its files where they stood; code takes text alone.
    const copied = code ? null : transferCopiedMessage(transfer)
    if (copied) {
      landAtSelection(view)
      options.message(copied, pasted.find((file) => file.type.startsWith('image/')) ?? null)
      return true
    }
    if (pasted.length) {
      landAtSelection(view)
      options.files(pasted)
      return true
    }
    // Code takes pasted text as it is.
    if (code) {
      return false
    }
    const text = transfer.getData('text/plain')
    if (pastedTextIsLong(text)) {
      landAtSelection(view)
      options.files([pastedTextFile(text)])
      return true
    }
    return take(view, transfer, slice, null)
  }

  /**
   * What a drop brings, from this composer or from anywhere else; false
   * when ProseMirror's own drop takes it.
   */
  function drop(view: EditorView, event: DragEvent, slice: Slice, moved: boolean): boolean {
    // A drag within this composer: a move is ProseMirror's; a copy
    // brings its nodes but not a second capsule of a file the message
    // holds already.
    if (view.dragging) {
      if (moved) {
        return false
      }
      const at = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos
      if (at === undefined) {
        return false
      }
      const kept = trimmedSlice(new Slice(keepCapsules(slice.content, view), slice.openStart, slice.openEnd), view.state.schema)
      insert(view, kept, at)
      return true
    }
    const transfer = event.dataTransfer
    if (!transfer) {
      return false
    }
    const pos = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos
    const $pos = pos === undefined ? null : view.state.doc.resolve(pos)
    // A code block holds only text: anything else lands after it.
    const at = $pos && inCode($pos) ? $pos.after() : pos
    // A message dragged out of another message or composer brings its
    // files back without their bytes, whatever else the drag carries.
    const copied = transferCopiedMessage(transfer)
    if (copied && at !== undefined) {
      options.land(at)
      options.message(copied, null)
      return true
    }
    // Files are the composer's to take, where they were dropped.
    if (transferHasFiles(transfer)) {
      return true
    }
    // Code takes dropped text as it is.
    if (!$pos || at === undefined || (inCode($pos) && transfer.getData('text/plain'))) {
      return false
    }
    return take(view, transfer, slice, at)
  }

  return new Plugin({
    key: new PluginKey('composerIntake'),
    props: {
      transformPastedHTML(html) {
        prepared = prepareHtml(html)
        return prepared.html
      },
      // A paste or a drop brings no blank lines at its ends. A drag within
      // this composer brings its own nodes: a capsule it moves is still in
      // the message until it lands.
      transformPasted(slice, view) {
        const kept = view.dragging ? slice : new Slice(keepCapsules(slice.content, view), slice.openStart, slice.openEnd)
        return trimmedSlice(kept, view.state.schema)
      },
      handlePaste: paste,
      handleDrop(view, event, slice, moved) {
        // What a drop brings is typed on from, as a text field takes a drop.
        const taken = drop(view, event, slice, moved)
        if (taken) {
          view.focus()
        }
        return taken
      },
    },
  })
}

/** The composer's intake of pasted and dropped content (`IntakeOptions`). */
export const ComposerIntake = Extension.create<IntakeOptions>({
  name: 'composerIntake',
  addOptions() {
    return {
      carries: () => false,
      land: () => {},
      files: () => {},
      message: () => {},
      content: () => {},
    }
  },
  addProseMirrorPlugins() {
    return [intakePlugin(this.options)]
  },
})
