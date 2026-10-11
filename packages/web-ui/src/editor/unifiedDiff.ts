import { RangeSetBuilder, StateEffect, StateField, Text, type EditorState, type Extension, type Range } from '@codemirror/state'
import { Decoration, EditorView, WidgetType, type DecorationSet } from '@codemirror/view'
import { highlightingFor, language } from '@codemirror/language'
import { Change, Chunk, uncollapseUnchanged, type DiffConfig } from '@codemirror/merge'
import { highlightTree } from '@lezer/highlight'

/**
 * How long one diff may take, in milliseconds, before the rest of it is
 * matched roughly. The merge package's own default instead gives up on any
 * stretch of more than 16,000 changed-or-not characters and calls all of it
 * changed, so a file of 25 KB with edits spread through it showed as wholly
 * replaced, and each read of a new version swung the whole view.
 */
const DIFF_CONFIG: DiffConfig = { timeout: 500 }

/** Unchanged lines kept around each change, as GitHub and git show. */
const MARGIN = 3
/** The fewest unchanged lines that fold into one row. */
const MIN_FOLDED = 4
/** Removed text longer than this is not colored by its language, as the merge package's default. */
const HIGHLIGHT_LIMIT = 3000
/** What git writes after a line the file ends with no newline after. */
const NO_NEWLINE = '\\ No newline at end of file'

/** `text`, with a newline after its last line when it has none. */
function terminated(text: Text): Text {
  return text.line(text.lines).length > 0 ? text.append(Text.of(['', ''])) : text
}

/**
 * The changed stretches between `original` and `modified`, as git shows
 * them: by whole lines, each of which ends with its newline. A created file
 * is only added lines and a removed one only removed lines; neither side's
 * empty last line, which a text has after its final newline or when it is
 * empty, ever counts as a line. A line the text ends without a newline
 * differs from the same line with one, so when only one side has a final
 * newline, the other side's last line changed. Positions are the texts'
 * own; a chunk's `toA` or `toB` past its side's end marks a last line it
 * covers that has no newline after it, where git writes "\ No newline at
 * end of file".
 *
 * The merge package's diff keeps a side's empty last line as a line, so a
 * created file without a final newline showed an empty removed line above
 * its first; and its unified view takes no chunks from outside. Its diff
 * runs here on the texts each given a final newline, and this puts back
 * the change of the final newline, which that hides.
 */
export function diffChunks(original: Text, modified: Text): readonly Chunk[] {
  const a = terminated(original)
  const b = terminated(modified)
  const chunks = [...Chunk.build(a, b, DIFF_CONFIG)]
  const openA = a !== original
  if (openA === (b !== modified))
    return chunks
  // One side ends without a newline and the other with one: no line of the
  // other side is the same as that side's last, which then changed. The
  // last chunk takes it: the one that reaches both texts' ends, which every
  // chunk after the last change does, or a new one there. What lies before
  // a chunk is the same on both sides, line for line, so a line it takes in
  // on one side it takes in on both.
  const last = chunks.at(-1)
  const ending = last && last.toA === a.length ? chunks.pop()! : new Chunk([], a.length, a.length, b.length, b.length)
  /** Where the side's last line starts. */
  const lastLine = (text: Text) => text.line(text.lines - 1).from
  let chunk = ending
  if (openA ? ending.fromA > lastLine(a) : ending.fromB > lastLine(b)) {
    const fromA = a.lineAt(ending.fromA - 1).from
    const fromB = b.lineAt(ending.fromB - 1).from
    chunk = new Chunk(shifted(ending.changes, ending.fromA - fromA, ending.fromB - fromB), fromA, ending.toA, fromB, ending.toB, ending.precise)
  }
  // A chunk it now touches joins it, so what was removed shows above what replaced it.
  const before = chunks.at(-1)
  if (before && (before.toA >= chunk.fromA || before.toB >= chunk.fromB)) {
    chunks.pop()
    chunk = new Chunk(
      [...before.changes, ...shifted(chunk.changes, chunk.fromA - before.fromA, chunk.fromB - before.fromB)],
      before.fromA,
      chunk.toA,
      before.fromB,
      chunk.toB,
      before.precise && chunk.precise,
    )
  }
  chunks.push(chunk)
  return chunks
}

function shifted(changes: readonly Change[], byA: number, byB: number): Change[] {
  return changes.map((change) => new Change(change.fromA + byA, change.toA + byA, change.fromB + byB, change.toB + byB))
}

/** A diff's original text and its chunks against the editor's text. */
export interface DiffState {
  original: Text
  chunks: readonly Chunk[]
}

/** Gives a unified diff a new original text to compare against. */
export const setDiffOriginal = StateEffect.define<Text>()

const diffField = StateField.define<DiffState | null>({
  create: () => null,
  update(value, tr) {
    if (!value)
      return value
    let original = value.original
    for (const effect of tr.effects) {
      if (effect.is(setDiffOriginal))
        original = effect.value
    }
    return original === value.original && !tr.docChanged ? value : { original, chunks: diffChunks(original, tr.newDoc) }
  },
})

/** The editor's diff, when it shows one. */
export function diffOf(state: EditorState): DiffState | null {
  return state.field(diffField, false) ?? null
}

/** A row in git's words after a last line without a newline. */
function noNewlineRow(): HTMLElement {
  const row = document.createElement('div')
  row.className = 'cm-noNewline'
  row.textContent = NO_NEWLINE
  return row
}

/** The lines a chunk removed, colored by the file's language, shown above what replaced them. */
class RemovedLines extends WidgetType {
  constructor(
    readonly text: string,
    /** The removed stretches inside the lines, as positions in `text`. */
    readonly changed: readonly { from: number; to: number }[],
    readonly noNewline: boolean,
  ) {
    super()
  }

  eq(other: RemovedLines): boolean {
    return other.text === this.text
      && other.noNewline === this.noNewline
      && other.changed.length === this.changed.length
      && other.changed.every((range, index) => range.from === this.changed[index]!.from && range.to === this.changed[index]!.to)
  }

  toDOM(view: EditorView): HTMLElement {
    const { text, changed } = this
    const dom = document.createElement('div')
    dom.className = 'cm-deletedChunk'
    const styled: { from: number; to: number; className: string }[] = []
    const lang = view.state.facet(language)
    if (lang && text.length <= HIGHLIGHT_LIMIT) {
      highlightTree(lang.parser.parse(text), { style: (tags) => highlightingFor(view.state, tags) }, (from, to, className) => {
        styled.push({ from, to, className })
      })
    }
    const cuts = new Set([0, text.length])
    for (const range of [...styled, ...changed]) {
      cuts.add(range.from)
      cuts.add(range.to)
    }
    for (let at = text.indexOf('\n'); at >= 0; at = text.indexOf('\n', at + 1)) {
      cuts.add(at)
      cuts.add(at + 1)
    }
    const stops = [...cuts].sort((x, y) => x - y)
    const newLine = () => dom.appendChild(document.createElement('div')).appendChild(document.createElement('del'))
    const endLine = (line: HTMLElement) => {
      line.parentElement!.className = 'cm-deletedLine'
      if (!line.firstChild)
        line.appendChild(document.createElement('br'))
    }
    // Both lists run in order without overlaps, so each piece's classes are
    // those of the ranges the two pointers stand at.
    let style = 0
    let change = 0
    let line = newLine()
    for (let index = 0; index + 1 < stops.length; index += 1) {
      const from = stops[index]!
      const to = stops[index + 1]!
      while (style < styled.length && styled[style]!.to <= from)
        style += 1
      while (change < changed.length && changed[change]!.to <= from)
        change += 1
      if (text[from] === '\n') {
        endLine(line)
        line = newLine()
        continue
      }
      const classes = [
        ...(style < styled.length && styled[style]!.from <= from ? [styled[style]!.className] : []),
        ...(change < changed.length && changed[change]!.from <= from ? ['cm-deletedText'] : []),
      ]
      const piece = document.createTextNode(text.slice(from, to))
      if (classes.length) {
        const span = line.appendChild(document.createElement('span'))
        span.className = classes.join(' ')
        span.appendChild(piece)
      } else {
        line.appendChild(piece)
      }
    }
    endLine(line)
    if (this.noNewline)
      dom.appendChild(noNewlineRow())
    return dom
  }
}

/** The marker after the editor's last line, when the file ends it without a newline. */
class NoNewline extends WidgetType {
  eq(): boolean {
    return true
  }

  toDOM(): HTMLElement {
    return noNewlineRow()
  }
}

const changedLine = Decoration.line({ class: 'cm-changedLine' })
const changedText = Decoration.mark({ class: 'cm-changedText' })
const noNewline = Decoration.widget({ widget: new NoNewline(), block: true, side: 1 })
const hidden = Decoration.replace({ block: true })

/** The lines of `doc` as git counts them: not the empty one after a final newline, nor that of an empty text. */
function gitLines(doc: Text): number {
  return doc.line(doc.lines).length > 0 ? doc.lines : doc.lines - 1
}

/** Each chunk's added lines marked, and its removed lines above them. */
function chunkDecorations({ original, chunks }: DiffState, doc: Text): DecorationSet {
  const ranges: Range<Decoration>[] = []
  for (const chunk of chunks) {
    if (chunk.toA > chunk.fromA) {
      const text = original.sliceString(chunk.fromA, chunk.endA)
      const changed = chunk.changes
        .filter((change) => change.toA > change.fromA)
        .map((change) => ({ from: change.fromA, to: Math.min(text.length, change.toA) }))
      const widget = new RemovedLines(text, changed, chunk.toA > original.length)
      ranges.push(Decoration.widget({ widget, block: true, side: -1 }).range(chunk.fromB))
    }
    if (chunk.toB > chunk.fromB) {
      for (let at = chunk.fromB; at <= chunk.endB; at = doc.lineAt(at).to + 1)
        ranges.push(changedLine.range(at))
      for (const change of chunk.changes) {
        const to = Math.min(doc.length, chunk.fromB + change.toB)
        if (chunk.fromB + change.fromB < to)
          ranges.push(changedText.range(chunk.fromB + change.fromB, to))
      }
      if (chunk.toB > doc.length)
        ranges.push(noNewline.range(doc.length))
    }
  }
  // The editor's empty last line is no line of the file, as git shows it.
  if (gitLines(doc) < doc.lines)
    ranges.push(hidden.range(doc.length))
  return Decoration.set(ranges, true)
}

const changeDecorations = StateField.define<DecorationSet>({
  create: (state) => {
    const diff = diffOf(state)
    return diff ? chunkDecorations(diff, state.doc) : Decoration.none
  },
  update(value, tr) {
    const diff = diffOf(tr.state)
    return diff === diffOf(tr.startState) ? value : diff ? chunkDecorations(diff, tr.state.doc) : Decoration.none
  },
  provide: (field) => EditorView.decorations.from(field),
})

/** A stretch of unchanged lines folded into one row that a click unfolds. */
class FoldedLines extends WidgetType {
  constructor(readonly lines: number) {
    super()
  }

  eq(other: FoldedLines): boolean {
    return other.lines === this.lines
  }

  toDOM(view: EditorView): HTMLElement {
    const row = document.createElement('div')
    row.className = 'cm-collapsedLines'
    row.textContent = view.state.phrase('$ unchanged lines', this.lines)
    row.addEventListener('click', (event) => {
      const at = view.posAtDOM(event.target as Node)
      view.dispatch({ effects: uncollapseUnchanged.of(at) })
    })
    return row
  }

  ignoreEvent(event: Event): boolean {
    return event instanceof MouseEvent
  }

  get estimatedHeight(): number {
    return 27
  }
}

interface Folds {
  /** The folded stretches. */
  folded: DecorationSet
  /** The stretches the user unfolded, which stay unfolded as the texts change. */
  unfolded: DecorationSet
}

const unfoldedMark = Decoration.mark({})

/** The stretches of unchanged lines between the chunks, but those the user unfolded. */
function foldsOf(state: EditorState, unfolded: DecorationSet): DecorationSet {
  const chunks = diffOf(state)?.chunks ?? []
  const builder = new RangeSetBuilder<Decoration>()
  let previous = 1
  for (let index = 0; ; index += 1) {
    const chunk = index < chunks.length ? chunks[index] : null
    const from = index ? previous + MARGIN : 1
    const to = chunk ? state.doc.lineAt(chunk.fromB).number - 1 - MARGIN : gitLines(state.doc)
    if (to - from + 1 >= MIN_FOLDED) {
      const start = state.doc.line(from).from
      const end = state.doc.line(to).to
      let opened = false
      unfolded.between(start, end, () => {
        opened = true
        return false
      })
      if (!opened)
        builder.add(start, end, Decoration.replace({ widget: new FoldedLines(to - from + 1), block: true }))
    }
    if (!chunk)
      break
    previous = state.doc.lineAt(Math.min(state.doc.length, chunk.toB)).number
  }
  return builder.finish()
}

/**
 * The unchanged stretches, folded, built again whenever the chunks change,
 * so a change that a new version brought into a folded stretch never stays
 * hidden in it, counted as unchanged.
 */
const folds = StateField.define<Folds>({
  create(state) {
    return { folded: foldsOf(state, Decoration.none), unfolded: Decoration.none }
  },
  update(value, tr) {
    let unfolded = value.unfolded.map(tr.changes)
    let folded = value.folded.map(tr.changes)
    for (const effect of tr.effects) {
      if (!effect.is(uncollapseUnchanged))
        continue
      folded.between(effect.value, effect.value, (from, to) => {
        unfolded = unfolded.update({ add: [unfoldedMark.range(from, to)] })
      })
      folded = folded.update({ filter: (from) => from !== effect.value })
    }
    if (diffOf(tr.startState)?.chunks !== diffOf(tr.state)?.chunks)
      folded = foldsOf(tr.state, unfolded)
    return { folded, unfolded }
  },
  provide: (field) => EditorView.decorations.from(field, (value) => value.folded),
})

/**
 * The diff's rows' layout; its colors are the editor's (`DiffEditor`),
 * under the editor's `cm-diff` class, which outweighs the theme's own line
 * colors, such as the active line's.
 */
const diffBaseTheme = EditorView.baseTheme({
  '.cm-deletedChunk': { paddingLeft: '6px' },
  '.cm-deletedLine del': { textDecoration: 'none' },
  // Under the last line, where its text starts; in a removed stretch, under the removed lines'.
  '.cm-noNewline': { padding: '0 2px 0 6px', color: 'var(--color-fg-muted)', fontStyle: 'italic' },
  '.cm-deletedChunk .cm-noNewline': { padding: '0' },
  '.cm-collapsedLines': {
    padding: '5px 5px 5px 10px',
    cursor: 'pointer',
    '&:before': { content: '"⦚"', marginInlineEnd: '7px' },
    '&:after': { content: '"⦚"', marginInlineStart: '7px' },
  },
  '&light .cm-collapsedLines': {
    color: '#444',
    background: 'linear-gradient(to bottom, transparent 0, #f3f3f3 30%, #f3f3f3 70%, transparent 100%)',
  },
  '&dark .cm-collapsedLines': {
    color: '#ddd',
    background: 'linear-gradient(to bottom, transparent 0, #222 30%, #222 70%, transparent 100%)',
  },
})

/**
 * A unified diff of the editor's text against `original`, by git's lines
 * (`diffChunks`): each removed stretch above what replaced it, a last line
 * without a newline followed by git's note of it, and the unchanged lines
 * between changes folded but for three around each, a stretch of four or
 * more into one row a click unfolds. New texts of either side replace the
 * old in place (`useCodeView`), and the folding follows the chunks they
 * make.
 *
 * The merge package's own unified view draws its chunks, which count a
 * side's empty last line as a line, and takes none from outside; this
 * draws those of `diffChunks` instead, with the merge package's classes.
 */
export function unifiedDiff(original: string): Extension {
  return [
    diffField.init((state) => {
      const text = state.toText(original)
      return { original: text, chunks: diffChunks(text, state.doc) }
    }),
    changeDecorations,
    folds,
    diffBaseTheme,
    EditorView.editorAttributes.of({ class: 'cm-diff' }),
  ]
}
