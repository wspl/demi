import { readFile, readdir } from 'node:fs/promises'
import { extname, join } from 'node:path'
import { parse as parseVue } from '@vue/compiler-sfc'
import {
  parseSync,
  Visitor,
  type ArrowFunctionExpression,
  type Function as OxcFunction,
  type Program,
} from 'oxc-parser'

const sourceExtensions = new Set(['.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs', '.vue'])
const excludedDirectories = new Set(['__tests__', 'testing', 'fixtures', 'node_modules', 'dist', '.cache'])

/** Production source only; tests, declarations and fixture programs are separate inputs. */
export async function listProductionSources(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true })
  const files: string[] = []
  for (const entry of entries) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) {
      if (!excludedDirectories.has(entry.name))
        files.push(...await listProductionSources(path))
    } else if (entry.isFile() && sourceExtensions.has(extname(entry.name))
      && !/\.(?:test|spec|d)\.[cm]?[jt]sx?$/.test(entry.name)
      && !/^testing\.[cm]?[jt]sx?$/.test(entry.name)) {
      files.push(path)
    }
  }
  return files.sort()
}

export interface SourceProgram {
  source: string
  program: Program
}

/** Parse actual Vue script blocks; template text and comments are not JavaScript. */
export function parseSourcePrograms(file: string, source: string): SourceProgram[] {
  if (extname(file) !== '.vue')
    return [parseScript(file, source)]
  const { descriptor, errors } = parseVue(source, { filename: file })
  if (errors.length)
    throw new Error(`Unable to parse ${file}: ${errors.map(String).join('; ')}`)
  const programs: SourceProgram[] = []
  for (const block of [descriptor.script, descriptor.scriptSetup]) {
    if (!block)
      continue
    const language = block.lang ?? 'js'
    if (!['js', 'jsx', 'ts', 'tsx'].includes(language))
      throw new Error(`Unsupported script language in ${file}: ${language}`)
    const content = block.src ? `import ${JSON.stringify(block.src)}` : block.content
    programs.push(parseScript(`${file}.${language}`, content))
  }
  return programs
}

export async function readSourcePrograms(file: string): Promise<SourceProgram[]> {
  return parseSourcePrograms(file, await readFile(file, 'utf8'))
}

function parseScript(file: string, source: string): SourceProgram {
  const parsed = parseSync(file, source, { sourceType: 'module' })
  if (parsed.errors.length) {
    throw new Error(`Unable to parse ${file}: ${parsed.errors.map(error => error.message).join('; ')}`)
  }
  return { source, program: parsed.program }
}

export function moduleSpecifiers(program: Program): string[] {
  const specifiers = new Set<string>()
  new Visitor({
    ImportDeclaration(node) { specifiers.add(node.source.value) },
    ExportNamedDeclaration(node) {
      if (node.source)
        specifiers.add(node.source.value)
    },
    ExportAllDeclaration(node) { specifiers.add(node.source.value) },
    ImportExpression(node) {
      if (node.source.type === 'Literal' && typeof node.source.value === 'string')
        specifiers.add(node.source.value)
    },
    CallExpression(node) {
      if (node.callee.type === 'Identifier' && node.callee.name === 'require') {
        const argument = node.arguments[0]
        if (argument?.type === 'Literal' && typeof argument.value === 'string')
          specifiers.add(argument.value)
      }
    },
    TSImportType(node) {
      specifiers.add(node.source.value)
    },
  }).visit(program)
  return [...specifiers]
}

export interface FunctionImplementation {
  name: string
  fingerprint: string
}

type FunctionNode = OxcFunction | ArrowFunctionExpression

/**
 * Exact runtime-AST comparison, not a proof of arbitrary semantic equivalence.
 * It ignores types, formatting and parameter spelling and normalizes expression
 * arrows to returns. Different constants, properties and operations stay distinct.
 */
export function functionImplementations(program: Program): FunctionImplementation[] {
  const functions: FunctionImplementation[] = []
  const collect = (name: string, node: FunctionNode) => {
    if (!node.body)
      return
    // Cross-form comparison is unsafe for lexical this/arguments and nested scopes.
    if (containsScopeSensitiveCode(node.body))
      return
    const parameters = new Map<string, string>()
    for (const [index, parameter] of node.params.entries()) {
      if (parameter.type === 'Identifier')
        parameters.set(parameter.name, `$parameter${index}`)
      else
        return
    }
    const body = node.body.type === 'BlockStatement'
      ? node.body
      : { type: 'BlockStatement', body: [{ type: 'ReturnStatement', argument: node.body }] }
    functions.push({
      name,
      fingerprint: JSON.stringify({
        async: node.async,
        generator: node.type === 'ArrowFunctionExpression' ? false : node.generator,
        arity: node.params.length,
        body: runtimeAst(body, parameters),
      }),
    })
  }
  new Visitor({
    FunctionDeclaration(node) {
      if (node.id)
        collect(node.id.name, node)
    },
    VariableDeclarator(node) {
      if (node.id.type === 'Identifier' &&
        (node.init?.type === 'ArrowFunctionExpression' || node.init?.type === 'FunctionExpression')) {
        collect(node.id.name, node.init)
      }
    },
  }).visit(program)
  return functions
}

function containsScopeSensitiveCode(value: unknown): boolean {
  if (value === null || typeof value !== 'object')
    return false
  if (Array.isArray(value))
    return value.some(containsScopeSensitiveCode)
  const record = value as Record<string, unknown>
  if (['ThisExpression', 'Super', 'MetaProperty', 'FunctionDeclaration', 'FunctionExpression', 'ArrowFunctionExpression'].includes(String(record.type)))
    return true
  if (record.type === 'Identifier' && record.name === 'arguments')
    return true
  return Object.values(record).some(containsScopeSensitiveCode)
}

const erasedAstKeys = new Set(['start', 'end', 'loc', 'range', 'raw', 'typeAnnotation', 'returnType', 'typeParameters', 'typeArguments', 'declare'])
function runtimeAst(value: unknown, parameters: Map<string, string>, preserveIdentifier = false): unknown {
  if (value === null || typeof value !== 'object')
    return value
  if (Array.isArray(value))
    return value.map(item => runtimeAst(item, parameters))
  const record = value as Record<string, unknown>
  if (['TSAsExpression', 'TSTypeAssertion', 'TSNonNullExpression', 'TSSatisfiesExpression'].includes(String(record.type)))
    return runtimeAst(record.expression, parameters)
  const output: Record<string, unknown> = {}
  for (const [key, child] of Object.entries(record)) {
    if (erasedAstKeys.has(key) || (record.type === 'Identifier' && key === 'optional') ||
      (key === 'decorators' && Array.isArray(child) && child.length === 0))
      continue
    if (record.type === 'Identifier' && key === 'name' && !preserveIdentifier) {
      output.name = parameters.get(String(child)) ?? child
    } else {
      const propertyName = !record.computed &&
        ((record.type === 'MemberExpression' && key === 'property') ||
          (record.type === 'Property' && key === 'key'))
      output[key] = runtimeAst(child, parameters, propertyName)
    }
  }
  return output
}

/** Direct JSON assertions bypass validation; this check does not infer data flow. */
export function unvalidatedJsonAssertions(program: Program): number[] {
  const positions = new Set<number>()
  const inspect = (expression: import('oxc-parser').Expression, annotation: unknown, start: number) => {
    const unknownAnnotation = annotation !== null && typeof annotation === 'object' &&
      'type' in annotation && annotation.type === 'TSUnknownKeyword'
    if (!unknownAnnotation && isDecodedJson(expression))
      positions.add(start)
  }
  new Visitor({
    TSAsExpression(node) { inspect(node.expression, node.typeAnnotation, node.start) },
    TSTypeAssertion(node) { inspect(node.expression, node.typeAnnotation, node.start) },
    TSSatisfiesExpression(node) { inspect(node.expression, node.typeAnnotation, node.start) },
    VariableDeclarator(node) {
      // Oxc's BindingIdentifier type omits variable annotations present in its AST.
      const annotation: unknown = Reflect.get(node.id, 'typeAnnotation')
      if (node.init && annotation !== null && typeof annotation === 'object' &&
        'typeAnnotation' in annotation) {
        inspect(node.init, annotation.typeAnnotation, node.start)
      }
    },
  }).visit(program)
  return [...positions]
}

function isDecodedJson(expression: import('oxc-parser').Expression): boolean {
  if (expression.type === 'AwaitExpression')
    return isDecodedJson(expression.argument)
  if (expression.type === 'TSAsExpression' || expression.type === 'TSTypeAssertion'
    || expression.type === 'TSSatisfiesExpression' || expression.type === 'TSNonNullExpression'
    || expression.type === 'ParenthesizedExpression')
    return isDecodedJson(expression.expression)
  if (expression.type !== 'CallExpression' || expression.callee.type !== 'MemberExpression')
    return false
  const { callee } = expression
  if (callee.computed || callee.property.type !== 'Identifier')
    return false
  return (callee.object.type === 'Identifier' && callee.object.name === 'JSON' && callee.property.name === 'parse')
    || (callee.property.name === 'json' && expression.arguments.length === 0)
}
