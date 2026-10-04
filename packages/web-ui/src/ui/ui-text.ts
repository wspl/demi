/**
 * Capitalization of UI text, as macOS writes it. The gallery's Writing page
 * holds the rule, its sources and its examples; this module is the rule in
 * code, for that page and for the UI text check (`scripts/ui-text-check.ts`).
 *
 * A prop or field that carries UI text names its style through one of the
 * four types below, and the check reads that type to know how to check every
 * literal written into it. Text that is content rather than UI (a
 * conversation's title, a file's name, what a model wrote) stays `string`.
 */

/** Title style: a menu item, a button, a tab, a section or group title. */
export type TitleText = string

/** Sentence style: a switch or checkbox label, a settings row label, a tooltip, a description, a message. */
export type SentenceText = string

/**
 * A headline that is either a fragment or a sentence: an alert's, a toast's
 * or a dialog's title, or what an empty list says. Title style for a fragment
 * ("Rename Failed", "No Results"), sentence style with its ending punctuation
 * for a complete sentence ("Remove this project?").
 */
export type HeadlineText = string

/**
 * A field's hint: sentence style without ending punctuation ("Search
 * conversations"), or an example value written as the person would type it
 * ("name@example.com").
 */
export type PlaceholderText = string

export type TextStyle = 'title' | 'sentence' | 'headline' | 'placeholder'

/** The style each of the types above declares, by the type's name. */
export const TEXT_STYLE_TYPES: Readonly<Record<string, TextStyle>> = {
  TitleText: 'title',
  SentenceText: 'sentence',
  HeadlineText: 'headline',
  PlaceholderText: 'placeholder',
}

/**
 * The style of the text an element holds as its content rather than in a
 * prop: a component's default slot, or an HTML element.
 */
export const TEXT_CONTENT_STYLES: Readonly<Record<string, TextStyle>> = {
  Button: 'title',
  DropdownTrigger: 'title',
  button: 'title',
  option: 'title',
  th: 'title',
  h1: 'headline',
  h2: 'headline',
  h3: 'headline',
  h4: 'headline',
  label: 'sentence',
}

/** The style of an HTML element's attributes that show text: a native tooltip, a field's hint. */
export const TEXT_ATTRIBUTE_STYLES: Readonly<Record<string, TextStyle>> = {
  title: 'sentence',
  placeholder: 'placeholder',
}

/**
 * Words title style keeps lowercase unless they come first, last or after a
 * colon (Apple Style Guide, "capitalization"). Apple lists the prepositions of
 * four letters or fewer it means; the rest of that length follow its rule.
 */
export const TITLE_LOWERCASE_WORDS = {
  articles: ['a', 'an', 'the'],
  coordinatingConjunctions: ['and', 'but', 'or', 'nor', 'for', 'yet', 'so'],
  prepositions: [
    'at', 'by', 'for', 'from', 'in', 'into', 'of', 'off', 'on', 'onto', 'out', 'over', 'to', 'up', 'with',
    'as', 'down', 'like', 'near', 'past', 'per', 'plus', 'than', 'till', 'upon', 'via', 'vs',
  ],
} as const

const LOWERCASE = new Set<string>(Object.values(TITLE_LOWERCASE_WORDS).flat())

/**
 * Lowercase words that are never another part of speech in UI text, so a
 * capital one inside a title is a mistake. The others ("Up", "Down", "Over")
 * may be adverbs or particles, which title style capitalizes.
 */
const NEVER_CAPITALIZED_MID_TITLE = new Set(['a', 'an', 'the', 'and', 'or', 'nor', 'as', 'to', 'of'])

/**
 * Verbs whose particle title style capitalizes ("Sign In", "Set Up", "Turn
 * Off"), in the forms UI text uses, each with the particles it takes.
 */
const PHRASAL_VERBS: ReadonlyMap<string, readonly string[]> = new Map(
  [
    [['sign', 'signs', 'signing', 'signed'], ['in', 'out', 'up']],
    [['log', 'logs', 'logging', 'logged'], ['in', 'out', 'off']],
    [['set', 'sets', 'setting'], ['up']],
    [['turn', 'turns', 'turning', 'turned'], ['on', 'off']],
    [['back', 'backs', 'backing', 'backed'], ['up']],
    [['start', 'starts', 'starting', 'started'], ['up']],
    [['shut', 'shuts', 'shutting'], ['down']],
    [['look', 'looks', 'looking', 'looked'], ['up']],
    [['clean', 'cleans', 'cleaning', 'cleaned'], ['up']],
    [['wake', 'wakes', 'waking', 'woke'], ['up']],
    [['opt', 'opts', 'opting', 'opted'], ['in', 'out']],
    [['zoom', 'zooms', 'zooming', 'zoomed'], ['in', 'out']],
    [['check', 'checks', 'checking', 'checked'], ['out']],
  ].flatMap(([forms, particles]) => forms!.map((form) => [form, particles!] as const)),
)

/**
 * Prefixes a hyphen joins to a word: title style leaves the word after them
 * lowercase ("Re-enter"), as The Chicago Manual of Style does, which Apple
 * follows where its own guide is silent.
 */
const PREFIXES = new Set(['anti', 'co', 'de', 'multi', 'non', 'post', 'pre', 're', 'self', 'semi', 'sub', 'un'])

/** Hyphenated compounds Apple keeps lowercase after the hyphen in title style. */
const LOWERCASE_COMPOUNDS = new Set(['built-in', 'plug-in'])

/**
 * Names both styles keep as they are spelled. Names with a capital inside
 * them (OpenAI, macOS) or in capitals (MCP, URL) need no entry: a word
 * spelled so is kept wherever it stands.
 */
export const PROPER_NAMES: readonly string[] = [
  'Anthropic',
  'Apple',
  'Apple Style Guide',
  'Chrome',
  'Chromium',
  'Claude',
  'Claude Code',
  'Cloud',
  'Codex',
  'Demi',
  'Dock',
  'Finder',
  'Gemini',
  'Git',
  'Google',
  'Grok',
  'Grok Build',
  'Human Interface Guidelines',
  'Linux',
  'Lucide',
  'Mac',
  'Markdown',
  'Material Icon Theme',
  'Safari',
  'System Settings',
  'VS Code',
  'Windows',
]

/** Keys, which the Apple Style Guide names with a capital: press Escape, Shift-click. */
export const KEY_NAMES: readonly string[] = [
  'Command',
  'Control',
  'Delete',
  'End',
  'Enter',
  'Escape',
  'Home',
  'Option',
  'Page Down',
  'Page Up',
  'Return',
  'Shift',
  'Space',
  'Tab',
]

/** Lowercase identifiers UI text shows as written, such as a model's id. */
export const CODE_WORDS: readonly string[] = [
  'claude-sonnet',
]

/**
 * Titles that are lead-ins a control completes, as System Settings' "Click
 * in the scroll bar to": sentence style wherever they stand.
 */
export const SENTENCE_LEAD_INS: readonly string[] = [
  'Notify me when',
]

/** The stand-in for an expression inside a template string or a template's text. */
export const PLACEHOLDER = '{}'

export interface StyleProblem {
  /** The word or mark as written. */
  word: string
  /** What the style writes instead; empty to drop it. */
  want: string
}

interface Token {
  /** The word without the punctuation around it. */
  core: string
  /** The token ends a clause the next word starts afresh: a colon. */
  endsClause: boolean
  endsSentence: boolean
  /** Spelled as a name or code, so neither style changes it. */
  fixed: boolean
  quoted: boolean
}

const LEADING = /^["'“‘([{<]+/
const TRAILING = /["'”’)\]}>.,;:!?…]+$/

/**
 * The problems `text` has against `style`: every word to change, with what it
 * should be. `references` are names of other UI elements, as they are
 * spelled, that a sentence may name in their own capitals ("Choose Add
 * Source", "Click Copy").
 */
export function styleProblems(text: string, style: TextStyle, references: Iterable<string> = []): StyleProblem[] {
  const problems: StyleProblem[] = []
  if (text.includes('...'))
    problems.push({ word: '...', want: '…' })
  const titled = (style === 'title' && !SENTENCE_LEAD_INS.includes(text)) || (style === 'headline' && !endsAsSentence(text))
  // A title's own words are what it is checked for, so only a sentence takes references.
  const names = [...PROPER_NAMES, ...KEY_NAMES, ...CODE_WORDS]
  const tokens = tokenize(text, titled ? names : [...names, ...references])
  problems.push(...(titled ? titleProblems(tokens) : sentenceProblems(tokens, style !== 'placeholder')))
  // No label ends with a colon; a title has no ending punctuation at all.
  const ending = text.trim().match(style === 'title' ? /[.:]$/ : /:$/)?.[0]
  if (ending && !text.trim().endsWith('...'))
    problems.push({ word: ending, want: '' })
  return problems
}

/** The text in `style`, with every problem `styleProblems` finds fixed. */
export function applyStyle(text: string, style: TextStyle, references: Iterable<string> = []): string {
  const problems = styleProblems(text, style, references)
  // Word problems come in the order of the words they name.
  const words = problems.filter((problem) => problem.word !== '...' && problem.want !== '')
  let next = text.split(/(\s+)/).map((raw) => {
    const problem = words[0]
    if (!problem || raw.replace(LEADING, '').replace(TRAILING, '') !== problem.word)
      return raw
    words.shift()
    return raw.replace(problem.word, problem.want)
  }).join('')
  next = next.replaceAll('...', '…')
  const ending = problems.find((problem) => problem.want === '')
  return ending ? next.trimEnd().slice(0, -ending.word.length) : next
}

function endsAsSentence(text: string): boolean {
  return /[.?!]["”’)]*$/.test(text.trim())
}

function tokenize(text: string, names: readonly string[]): Token[] {
  const tokens: Token[] = []
  let quoted = false
  for (const raw of text.split(/\s+/).filter(Boolean)) {
    const opensQuote = /^["“‘]/.test(raw)
    const closesQuote = /["”’][.,;:!?…)]*$/.test(raw) && !/^[A-Za-z]+’[a-z]+$/.test(raw)
    if (opensQuote)
      quoted = true
    const trailing = raw.match(TRAILING)?.[0] ?? ''
    const core = raw.replace(LEADING, '').replace(TRAILING, '')
    tokens.push({
      core,
      endsClause: trailing.includes(':'),
      endsSentence: /[.?!]/.test(trailing.replace('…', '')),
      fixed: isFixed(core),
      quoted,
    })
    if (closesQuote)
      quoted = false
  }
  markNames(tokens, names)
  return tokens
}

/**
 * A token both styles leave as written: a placeholder, a number, code, a
 * letter standing for itself (a git status's M; not the word A or I), or a
 * name spelled with inner capitals.
 */
function isFixed(core: string): boolean {
  if (!/^[A-Za-z]/.test(core) || /^[B-HJ-Z]$/.test(core))
    return true
  if (core.includes(PLACEHOLDER) || /[0-9/\\._@#=<>[\]{}$%+*|~^`]/.test(core))
    return true
  return core.split(/[-–]/).some((part) => /[A-Z]/.test(part.slice(1)))
}

/** Marks the words of each name as fixed, the longest name first. */
function markNames(tokens: Token[], names: readonly string[]): void {
  const phrases = names.map((name) => name.replace(/…$/, '').split(' ')).sort((a, b) => b.length - a.length)
  for (let index = 0; index < tokens.length; index++) {
    for (const words of phrases) {
      const matches = words.every((word, offset) => namesWord(tokens[index + offset]?.core ?? '', word))
      if (!matches)
        continue
      for (let offset = 0; offset < words.length; offset++)
        tokens[index + offset]!.fixed = true
      index += words.length - 1
      break
    }
  }
}

/** The token is the name's word, possessive ("Demi's") or joined to a gesture ("Shift-click") included. */
function namesWord(core: string, word: string): boolean {
  const name = core.replace(/['’]s$/, '').replace(/-(click|drag)$/, '')
  return name === word
}

function titleProblems(tokens: readonly Token[]): StyleProblem[] {
  const problems: StyleProblem[] = []
  for (const [index, token] of tokens.entries()) {
    if (token.fixed)
      continue
    const head = token.core.split(/(?<=.)[-–](?=.)/)[0]!
    const lower = head.toLowerCase()
    const previous = tokens[index - 1]
    const startsClause = index === 0 || previous!.endsClause || previous!.endsSentence
    const isLast = index === tokens.length - 1
    const isParticle = previous !== undefined && (PHRASAL_VERBS.get(previous.core.toLowerCase())?.includes(lower) ?? false)
    const minor = LOWERCASE.has(lower) && !startsClause && !isLast && !isParticle
    const want = titleWord(token.core, minor)
    if (want !== token.core)
      problems.push({ word: token.core, want })
  }
  return problems
}

/**
 * The word as title style writes it. Any word that is not minor, and each
 * part of a hyphenated compound, starts with a capital; a part after a
 * prefix, a minor part, and the second part of Apple's Built-in and Plug-in
 * stay as written. A minor word stays as written too, unless it can be
 * nothing but minor: then it is lowercase.
 */
function titleWord(word: string, minor: boolean): string {
  const parts = word.split(/(?<=.)([-–])(?=.)/)
  const first = parts[0]!.toLowerCase()
  if (minor)
    return NEVER_CAPITALIZED_MID_TITLE.has(first) ? word.toLowerCase() : word
  if (LOWERCASE_COMPOUNDS.has(word.toLowerCase()))
    return capitalize(word)
  return parts.map((part, index) => {
    if (index === 0)
      return capitalize(part)
    if (part === '-' || part === '–' || PREFIXES.has(first) || LOWERCASE.has(part.toLowerCase()))
      return part
    return capitalize(part)
  }).join('')
}

/**
 * Sentence style: the first word of each sentence starts with a capital, and
 * the others do not unless they are names. `capitalizeFirst` is false for a
 * hint, whose first word may be an example value.
 */
function sentenceProblems(tokens: readonly Token[], capitalizeFirst: boolean): StyleProblem[] {
  const problems: StyleProblem[] = []
  for (const [index, token] of tokens.entries()) {
    const previous = tokens[index - 1]
    const startsSentence = index === 0 || previous!.endsSentence
    const afterColon = previous?.endsClause ?? false
    if (token.fixed || token.quoted)
      continue
    if (startsSentence && isLowercase(token.core) && (capitalizeFirst || index > 0))
      problems.push({ word: token.core, want: capitalize(token.core) })
    else if (!startsSentence && !afterColon && !isLowercase(token.core) && !/^I(['’].*)?$/.test(token.core))
      problems.push({ word: token.core, want: token.core.toLowerCase() })
  }
  return problems
}

function isLowercase(word: string): boolean {
  return /^[a-z]/.test(word)
}

function capitalize(word: string): string {
  return word.charAt(0).toUpperCase() + word.slice(1)
}
