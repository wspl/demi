/**
 * The UI text check: finds every literal the web app shows as UI text and
 * checks its capitalization against the style its place declares (the
 * gallery's Writing page has the rule).
 *
 * A place declares its style in one of two ways. A prop or a field declares
 * it by its type, `TitleText`, `SentenceText`, `HeadlineText` or
 * `PlaceholderText` (`packages/web-ui/src/ui/ui-text.ts`); the check builds
 * the TypeScript program of the Vue packages, templates included, and
 * follows each string literal to the property, parameter or variable it is
 * written into. Text an element holds as content, such as a button's label or
 * a heading, takes the style `TEXT_CONTENT_STYLES` gives its element.
 *
 * `bun scripts/ui-text-check.ts [path prefix]` prints each violation with its
 * line, then the counts per package and place. `--unchecked` prints instead
 * the text the check cannot see: literals in places that sound like UI text
 * but declare no style, and template text in elements that declare none. The
 * test in `scripts/__tests__/ui-text.test.ts` compares the violations with
 * the baseline of those not yet fixed.
 */
import { readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import ts from 'typescript'
import { NodeTypes, type ElementNode } from '@vue/compiler-dom'
import { VueVirtualCode, forEachElementNode, type Language } from '@vue/language-core'
import {
  PLACEHOLDER,
  TEXT_ATTRIBUTE_STYLES,
  TEXT_CONTENT_STYLES,
  TEXT_STYLE_TYPES,
  styleProblems,
  type StyleProblem,
  type TextStyle,
} from '../packages/web-ui/src/ui/ui-text'
import { createVueProgram } from './vue-program'

const ROOT = resolve(import.meta.dir, '..')

/** The packages whose sources hold UI text. The gallery's config resolves them all. */
const PACKAGES = [
  'web-ui',
  'web',
  'web-gallery',
  'plugin-browser',
  'plugin-changes',
  'plugin-expose',
  'plugin-file-browser',
  'plugin-skills',
]
const PROJECT = resolve(ROOT, 'packages/web-gallery/tsconfig.app.json')

/** The violations not yet fixed, one `baselineLine` each. */
export const BASELINE = resolve(ROOT, 'scripts/__tests__/ui-text-baseline.txt')

export interface UiText {
  /** The file, relative to the repository root. */
  file: string
  line: number
  text: string
  style: TextStyle
  /** What declares the style: a type's property, or an element's content or attribute. */
  place: string
  problems: StyleProblem[]
}

/** Text the check cannot see, because its place declares no style. */
export interface UncheckedText {
  file: string
  line: number
  text: string
  /** The property's declaration, `?` when the literal's object has no declared type, or the element. */
  place: string
}

/** Property names that usually carry UI text: where an unchecked literal is worth a look. */
const TEXT_PROPERTY_NAMES = new Set([
  'action', 'caption', 'confirmLabel', 'description', 'detail', 'disabledReason', 'emptyText', 'label',
  'message', 'note', 'placeholder', 'summary', 'text', 'title', 'tooltip',
])

interface Found {
  file: string
  offset: number
  text: string
  place: string
}

export function checkUiText(): { checked: UiText[]; unchecked: UncheckedText[] } {
  const { program, language } = buildProgram()
  const checker = program.getTypeChecker()
  const styled = new Map<string, Found & { style: TextStyle }>()
  const unstyled = new Map<string, Found>()
  const key = (found: Found) => `${found.file}:${found.offset}:${found.text}`
  const record = (found: Found, style: TextStyle | undefined): void => {
    if (!/[A-Za-z]/.test(found.text))
      return
    if (style)
      styled.set(key(found), { ...found, style })
    else
      unstyled.set(key(found), found)
  }
  for (const sourceFile of program.getSourceFiles()) {
    if (!isChecked(sourceFile.fileName))
      continue
    const file = sourceFile.fileName
    const toSource = sourceOffsets(language, file)
    visitLiterals(sourceFile, (node) => {
      const target = textTarget(node, checker)
      // The text after the opening quote: a template's attribute maps from there, not from the quote.
      const offset = target && toSource(node.getStart(sourceFile) + 1)
      if (!target || offset === undefined || (!target.style && !TEXT_PROPERTY_NAMES.has(target.name)))
        return
      record({ file, offset, text: literalText(node), place: target.place }, target.style)
    })
    const root = language.scripts.get(file)?.generated?.root
    if (root instanceof VueVirtualCode && root.ir.template?.ast) {
      const base = root.ir.template.startTagEnd
      for (const element of forEachElementNode(root.ir.template.ast)) {
        visitElement(element, (offset, text, style, place) => record({ file, offset: base + offset, text, place }, style))
      }
    }
  }
  // Vue generates a template's props twice, and only one copy declares their style.
  for (const found of styled.keys())
    unstyled.delete(found)
  // A sentence may name another element in that element's own capitals: any
  // UI text short enough to be a name, without a sentence's ending.
  const references = new Set([...styled.values()].map((found) => found.text).filter(isName))
  const checked = [...styled.values()].map((found) => ({
    ...locate(found),
    style: found.style,
    place: found.place,
    problems: styleProblems(found.text, found.style, references),
  }))
  return { checked: checked.sort(byPosition), unchecked: [...unstyled.values()].map(locate).sort(byPosition) }
}

function isName(text: string): boolean {
  return !/[.?!:]$/.test(text) && text.split(/\s+/).length <= 5
}

const sources = new Map<string, string>()

function locate(found: Found): { file: string; line: number; text: string; place: string } {
  let source = sources.get(found.file)
  if (source === undefined) {
    source = readFileSync(found.file, 'utf8')
    sources.set(found.file, source)
  }
  const line = source.slice(0, found.offset).split('\n').length
  return { file: relative(ROOT, found.file), line, text: found.text, place: found.place }
}

function byPosition(a: { file: string; line: number }, b: { file: string; line: number }): number {
  return a.file.localeCompare(b.file) || a.line - b.line
}

function isChecked(fileName: string): boolean {
  const file = relative(ROOT, fileName)
  return PACKAGES.some((name) => file.startsWith(`packages/${name}/src/`))
    && !file.includes('/__tests__/')
    && !file.endsWith('.test.ts')
}

function buildProgram(): { program: ts.Program; language: Language<string> } {
  const rootNames = PACKAGES.flatMap((name) => [...new Bun.Glob(`packages/${name}/src/**/*.{ts,vue}`).scanSync({ cwd: ROOT, absolute: true })])
    .filter(isChecked)
  const created = createVueProgram(PROJECT, rootNames)
  if (!created.ok)
    throw new Error(ts.flattenDiagnosticMessageText(created.errors[0]?.messageText ?? 'no config', '\n'))
  return created
}

/**
 * Maps an offset in the program's text of `fileName` to the offset in the
 * file itself. A .vue file's program text is generated, and a literal it
 * generates with no source maps to nothing.
 */
function sourceOffsets(language: Language<string>, fileName: string): (offset: number) => number | undefined {
  const sourceScript = language.scripts.get(fileName)
  const generated = sourceScript?.generated
  const serviceScript = generated?.languagePlugin.typescript?.getServiceScript(generated.root)
  if (!sourceScript || !serviceScript)
    return (offset) => offset
  const map = language.maps.get(serviceScript.code, sourceScript)
  // Unless the script asks otherwise, the program's text puts the generated code after blanks as long as the file.
  const leading = serviceScript.preventLeadingOffset ? 0 : sourceScript.snapshot.getLength()
  return (offset) => {
    for (const [sourceOffset] of map.toSourceLocation(offset - leading))
      return sourceOffset
    return undefined
  }
}

type TextLiteral = ts.StringLiteral | ts.NoSubstitutionTemplateLiteral | ts.TemplateExpression

function visitLiterals(node: ts.Node, visit: (literal: TextLiteral) => void): void {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateExpression(node)) {
    visit(node)
    return
  }
  ts.forEachChild(node, (child) => visitLiterals(child, visit))
}

function literalText(node: TextLiteral): string {
  if (!ts.isTemplateExpression(node))
    return node.text
  return node.head.text + node.templateSpans.map((span) => PLACEHOLDER + span.literal.text).join('')
}

/**
 * The property, parameter or variable the literal, or a choice between
 * literals, is written into, and the style its declared type names.
 */
function textTarget(literal: TextLiteral, checker: ts.TypeChecker): { name: string; place: string; style?: TextStyle } | undefined {
  let value: ts.Node = literal
  while (isPassThrough(value.parent, value))
    value = value.parent
  const target = value.parent
  let symbol: ts.Symbol | undefined
  if (ts.isPropertyAssignment(target) && target.initializer === value) {
    const owner = checker.getContextualType(target.parent)
    const name = target.name.getText().replace(/^['"]|['"]$/g, '')
    symbol = owner && propertyOf(checker, owner, name)
    if (!symbol)
      return { name, place: '?' }
  }
  else if (ts.isCallExpression(target) || ts.isNewExpression(target)) {
    const index = target.arguments?.indexOf(value as ts.Expression) ?? -1
    const signature = index >= 0 ? checker.getResolvedSignature(target) : undefined
    const parameters = signature?.getParameters() ?? []
    symbol = parameters[Math.min(index, parameters.length - 1)]
  }
  else if (ts.isVariableDeclaration(target) && target.initializer === value) {
    symbol = checker.getSymbolAtLocation(target.name)
  }
  if (!symbol)
    return undefined
  const declarations = symbol.declarations ?? []
  const place = (declaration: ts.Declaration) => `${relative(ROOT, declaration.getSourceFile().fileName)}#${symbol.getName()}`
  for (const declaration of declarations) {
    const type = typeAnnotation(declaration)
    const style = type && styleOfType(type)
    if (style)
      return { name: symbol.getName(), place: place(declaration), style }
  }
  return { name: symbol.getName(), place: declarations[0] ? place(declarations[0]) : '?' }
}

/** The type a declaration of a property, parameter or variable is written with. */
function typeAnnotation(declaration: ts.Declaration): ts.TypeNode | undefined {
  if (ts.isPropertySignature(declaration) || ts.isPropertyDeclaration(declaration) || ts.isParameter(declaration) || ts.isVariableDeclaration(declaration))
    return declaration.type
  return undefined
}

/** A node that hands `child`'s value on as its own: a choice, a fallback, an array of such values. */
function isPassThrough(node: ts.Node, child: ts.Node): boolean {
  if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isSatisfiesExpression(node) || ts.isArrayLiteralExpression(node))
    return true
  if (ts.isConditionalExpression(node))
    return node.condition !== child
  if (ts.isBinaryExpression(node)) {
    const kind = node.operatorToken.kind
    return kind === ts.SyntaxKind.QuestionQuestionToken || kind === ts.SyntaxKind.BarBarToken
  }
  return false
}

function propertyOf(checker: ts.TypeChecker, type: ts.Type, name: string): ts.Symbol | undefined {
  const owner = checker.getNonNullableType(type)
  if (owner.isUnion())
    return owner.types.map((member) => member.getProperty(name)).find(Boolean)
  return owner.getProperty(name)
}

/** The style a declared type names: `TitleText`, or a union or array that holds it. */
function styleOfType(type: ts.TypeNode): TextStyle | undefined {
  if (ts.isTypeReferenceNode(type) && ts.isIdentifier(type.typeName))
    return TEXT_STYLE_TYPES[type.typeName.text]
  if (ts.isUnionTypeNode(type))
    return type.types.map(styleOfType).find(Boolean)
  if (ts.isArrayTypeNode(type))
    return styleOfType(type.elementType)
  if (ts.isTypeOperatorNode(type) || ts.isParenthesizedTypeNode(type))
    return styleOfType(type.type)
  return undefined
}

/** `style` is undefined for text in an element that declares none. Offsets count from the template's start. */
type TemplateVisitor = (offset: number, text: string, style: TextStyle | undefined, place: string) => void

/** The text an element holds as content, and the HTML attributes that show text. */
function visitElement(element: ElementNode, visit: TemplateVisitor): void {
  const contentStyle = TEXT_CONTENT_STYLES[element.tag]
  for (const [offset, text] of contentTexts(element, contentStyle !== undefined))
    visit(offset, text, contentStyle, `<${element.tag}>`)
  if (element.tag !== element.tag.toLowerCase())
    return
  for (const prop of element.props) {
    if (prop.type === NodeTypes.ATTRIBUTE && prop.value && TEXT_ATTRIBUTE_STYLES[prop.name])
      visit(prop.value.loc.start.offset, prop.value.content, TEXT_ATTRIBUTE_STYLES[prop.name], `<${element.tag} ${prop.name}>`)
    if (prop.type === NodeTypes.DIRECTIVE && prop.name === 'bind' && prop.arg?.type === NodeTypes.SIMPLE_EXPRESSION && prop.exp?.type === NodeTypes.SIMPLE_EXPRESSION) {
      const style = TEXT_ATTRIBUTE_STYLES[prop.arg.content]
      if (style) {
        for (const text of expressionTexts(prop.exp.content))
          visit(prop.exp.loc.start.offset, text, style, `<${element.tag} ${prop.arg.content}>`)
      }
    }
  }
}

/**
 * The texts an element shows as its content: its text with each expression
 * as a placeholder, or, when its content is one expression choosing between
 * literals, each of them. Nested elements, such as an icon, take no part.
 * Content that is only an expression counts only where a style is declared:
 * elsewhere it is data, not text the template writes.
 */
function contentTexts(element: ElementNode, styled: boolean): [offset: number, text: string][] {
  const parts = element.children.filter((child) => child.type === NodeTypes.TEXT || child.type === NodeTypes.INTERPOLATION)
  const first = parts[0]
  if (!first)
    return []
  if (parts.length === 1 && first.type === NodeTypes.INTERPOLATION && first.content.type === NodeTypes.SIMPLE_EXPRESSION)
    return styled ? expressionTexts(first.content.content).map((text) => [first.loc.start.offset, text]) : []
  const text = parts.map((part) => (part.type === NodeTypes.TEXT ? part.content : ` ${PLACEHOLDER} `)).join('').replace(/\s+/g, ' ').trim()
  return [[first.loc.start.offset, text]]
}

/** The literals an expression chooses between, when it is nothing but such a choice. */
function expressionTexts(expression: string): string[] {
  const source = ts.createSourceFile('expression.ts', `(${expression})`, ts.ScriptTarget.Latest)
  const texts: string[] = []
  let literalsOnly = true
  const visit = (node: ts.Node): void => {
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateExpression(node))
      texts.push(literalText(node))
    else if (ts.isParenthesizedExpression(node))
      visit(node.expression)
    else if (ts.isConditionalExpression(node)) {
      visit(node.whenTrue)
      visit(node.whenFalse)
    }
    else
      literalsOnly = false
  }
  const statement = source.statements[0]
  if (statement && ts.isExpressionStatement(statement))
    visit(statement.expression)
  return literalsOnly ? texts : []
}

/** One line per violation, the form the baseline keeps: no line number, so an edit elsewhere in the file leaves it. */
export function baselineLine(text: UiText): string {
  return `${text.file}\t${text.style}\t${JSON.stringify(text.text)}`
}

/** The baseline's lines, without its comments. */
export function readBaseline(): Set<string> {
  return new Set(readFileSync(BASELINE, 'utf8').split('\n').filter((line) => line !== '' && !line.startsWith('#')))
}

/** The violation as the check reports it: where, what, and the words to change. */
export function describe(text: UiText): string {
  const fixes = text.problems.map((problem) => `${problem.word} → ${problem.want || '(drop)'}`).join(', ')
  const sentence = text.style === 'headline' ? ', or end it as a sentence' : ''
  return `${text.file}:${text.line}  ${text.style}  ${JSON.stringify(text.text)}  (${fixes}${sentence})  [${text.place}]`
}

if (import.meta.main) {
  const listUnchecked = process.argv.includes('--unchecked')
  const prefix = process.argv.slice(2).find((argument) => !argument.startsWith('--')) ?? ''
  const result = checkUiText()
  if (listUnchecked) {
    for (const text of result.unchecked.filter((entry) => entry.file.startsWith(prefix)))
      console.log(`${text.file}:${text.line}  ${JSON.stringify(text.text)}  [${text.place}]`)
    process.exit(0)
  }
  const texts = result.checked.filter((text) => text.file.startsWith(prefix))
  const counts = new Map<string, { checked: number; violations: number }>()
  for (const text of texts) {
    if (text.problems.length > 0)
      console.log(describe(text))
    const key = `${text.file.split('/').slice(0, 2).join('/')}  ${text.style}  ${text.place}`
    const count = counts.get(key) ?? { checked: 0, violations: 0 }
    count.checked += 1
    count.violations += text.problems.length > 0 ? 1 : 0
    counts.set(key, count)
  }
  console.log('\npackage  style  place: checked texts, violations')
  for (const [key, count] of [...counts].sort())
    console.log(`${key}: ${count.checked}, ${count.violations}`)
}
