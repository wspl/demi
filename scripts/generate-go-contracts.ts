import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { z } from 'zod'
import * as commandProtocol from '../packages/command-protocol/src/index'
import {
  artifactLocationSchema, commandCallerSchema, commandContextSchema, commandErrorSchema,
  commandLocaleSchema, completionSchema, conversationRequestSchema, conversationStatusSchema,
  editContextSchema, editCopiesSchema, editFileSchema, editJournalSchema, invocationSchema,
  localInvocationSchema, nativeArtifactSchema, nativeBindingSchema, nativePackageSchema,
  nativeTargetSchema, serviceInfoSchema,
} from '../packages/command-protocol/src/index'
import { manifestLeafSchema, manifestNodeSchema, manifestSchema } from '../packages/command-loader/src/manifest/schema'
import * as browserIndex from '../packages/browser-protocol/src/index'
import {
  browserCreatedBySchema, browserDefaultTimeout, browserErrorCodeSchema, browserErrorSchema,
  browserInstallationSchema, browserNodeSchema, browserOperations, browserQuerySchema, browserReleaseSchema,
  browserRuntimeConfigSchema, browserTabSchema, browserTargetSchema, browserTreeNodeSchema,
  browserViewportModeSchema, browserViewportSchema,
} from '../packages/browser-protocol/src/index'
import * as browserLive from '../packages/browser-protocol/src/live'
import {
  liveControlOptionSchema, liveControlSchema, liveDialogSchema, liveModuleMessageSchema, liveTabSchema,
  liveViewerMessageSchema,
} from '../packages/browser-protocol/src/live'
import * as runnerMessages from '../packages/runner-protocol/src/messages'
import * as runnerSchemas from '../packages/runner-protocol/src/schemas'
import {
  backendToRunnerMessageSchema, bytesSchema, fsOps, gitChangeSchema, gitOps, helloErrorCodeSchema,
  jobFileChangeSchema, logLineSchema, logPageSchema, netErrorCodeSchema, pipeRefSchema,
  runnerToBackendMessageSchema, serviceErrorCodeSchema,
} from '../packages/runner-protocol/src/schemas'
import { managedBootSchema } from '../packages/runner-protocol/src/managed-boot'
import { machineOps, machineRequestSchema, machineResponseSchema } from '../packages/machines/src/protocol'
import { imageStateSchema, runtimeStateSchema } from '../packages/machines/src/provisioner'
import { GoZodTypes, goConstant, goName, goString, resolveLazy, unionOptions, type GoForeign } from './go-zod'

// Writes the Go bindings of every contract package to
// internal/contract/<name>/zz_generated.go (`bun run go:contracts`).

type Schema = z.core.$ZodType
type Codec = 'json' | 'msgpack'

const MODULE = 'github.com/wspl/demi/internal/contract'
const RUNTIME = `${MODULE}/zodrt`

export interface GoContractRoot {
  name: string
  schema: Schema
  codecs: Codec[]
}

export interface GoContractPackage {
  name: string
  /** The Zod sources, for the generated file's header. */
  sources: string[]
  portableJson: boolean
  roots: GoContractRoot[]
  /** The generated Go after the package clause and imports. */
  body: string
  /** The import paths the body uses. */
  imports: string[]
}

/** Another contract package's named types, which this package imports. */
function foreign(pkg: string, names: [string, Schema][]): Map<Schema, GoForeign> {
  const kinds: Record<string, GoForeign['kind']> = { object: 'struct', union: 'union', enum: 'enum' }
  return new Map(names.map(([name, schema]) => {
    const kind = kinds[(schema as z.core.$ZodTypes)._zod.def.type]
    if (!kind)
      throw new Error(`${pkg}.${name} is not a declared type`)
    return [schema, { pkg, path: `${MODULE}/${pkg}`, name, kind }]
  }))
}

/** Every all-caps export of the modules as a Go constant, each name once. */
function exportedConstants(...modules: Record<string, unknown>[]): string {
  const constants = new Map<string, unknown>()
  for (const module of modules) {
    for (const [key, value] of Object.entries(module)) {
      if (/^[A-Z][A-Z0-9_]*$/.test(key))
        constants.set(key, value)
    }
  }
  return [...constants].map(([key, value]) => goConstant(key, value)).join('\n')
}

/** The single-value literal of an object option's field. */
function tagOf(schema: Schema, key: string): string {
  const shape = (schema as z.ZodObject).shape
  return (shape[key] as z.ZodLiteral<string>).value
}

function optionTagged(union: Schema, key: string, tag: string): z.ZodObject {
  const option = unionOptions(union).find(item => tagOf(item, key) === tag)
  if (!option)
    throw new Error(`No ${key} ${tag} option`)
  return option as z.ZodObject
}

function unwrap(schema: Schema): Schema {
  return (schema as z.ZodOptional<z.ZodType>).unwrap()
}

function rootCode(generator: GoZodTypes, roots: GoContractRoot[]): string {
  return roots.map(root => generator.root(root.schema, root.name, root.codecs)).join('\n\n')
}

/**
 * A package from its generator and the extra code the package adds, which
 * uses the imports `extraImports` names.
 */
function assemble(name: string, sources: string[], portableJson: boolean, roots: GoContractRoot[], generator: GoZodTypes, extra: string, extraImports: string[] = []): GoContractPackage {
  const code = rootCode(generator, roots)
  const body = [generator.finish(), code, extra].filter(Boolean).join('\n\n')
  const imports = [...new Set([...generator.imports, ...extraImports])].sort()
  return { name, sources, portableJson, roots, body, imports }
}

/** The named types of cmdservice, which runnerwire and manifest share. */
const cmdserviceNames: [string, Schema][] = [
  ['NativeTarget', nativeTargetSchema], ['NativeArtifact', nativeArtifactSchema],
  ['NativeBinding', nativeBindingSchema], ['EditContext', editContextSchema], ['EditCopies', editCopiesSchema],
  ['EditFileKind', editFileSchema.shape.kind], ['EditFile', editFileSchema], ['EditJournal', editJournalSchema],
  ['ConversationRequest', conversationRequestSchema], ['ConversationStatus', conversationStatusSchema],
  ['ArtifactLocation', artifactLocationSchema], ['PackageDescriptor', nativePackageSchema],
  ['ServiceInfo', serviceInfoSchema], ['CommandCaller', commandCallerSchema], ['CommandLocale', commandLocaleSchema],
  ['CommandContext', commandContextSchema], ['CommandError', commandErrorSchema], ['Completion', completionSchema],
  ['Invocation', invocationSchema], ['LocalInvocation', localInvocationSchema],
]

function cmdservice(): GoContractPackage {
  const generator = new GoZodTypes({ runtime: RUNTIME })
  generator
    .name(artifactLocationSchema.options[0], 'ArtifactURL')
    .name(artifactLocationSchema.options[1], 'ArtifactPath')
    .name(commandCallerSchema.options[0], 'AgentCaller')
    .name(commandCallerSchema.options[1], 'UserCaller')
  for (const [name, schema] of cmdserviceNames)
    generator.name(schema, name)
  const roots = cmdserviceNames.map(([name, schema]) => ({ name, schema, codecs: ['json'] as Codec[] }))
  return assemble('cmdservice', ['packages/command-protocol/src/index.ts'], false, roots, generator,
    exportedConstants(commandProtocol))
}

function runnerwire(): GoContractPackage {
  const generator = new GoZodTypes({ runtime: RUNTIME, bytes: bytesSchema, dates: true, portableJson: true, foreign: foreign('cmdservice', cmdserviceNames) })
  const hello = optionTagged(runnerToBackendMessageSchema, 'type', 'hello')
  const runnerInfo = hello.shape.runner as z.ZodObject
  const jobExit = optionTagged(runnerToBackendMessageSchema, 'type', 'job_exit')
  const spawnExit = optionTagged(runnerToBackendMessageSchema, 'type', 'spawn_exit')
  const owner = optionTagged(runnerToBackendMessageSchema, 'type', 'artifact_resolve').shape.owner as z.ZodUnion<[z.ZodObject, z.ZodObject]>
  const readdir = fsOps.readdir.result
  generator
    .name(managedBootSchema, 'ManagedBoot')
    .name(pipeRefSchema, 'PipeRef')
    .name(gitChangeSchema, 'GitChange')
    .name(gitOps.changes.result, 'GitChangesResult')
    .name(logLineSchema, 'LogLine')
    .name(logPageSchema, 'LogPage')
    .name(jobFileChangeSchema, 'JobFileChange')
    .name(fsOps.stat.result, 'HostFileStat')
    .name(readdir, 'ReaddirResult')
    .name(readdir.options[0], 'ReaddirNames')
    .name(readdir.options[1], 'ReaddirEntries')
    .name(readdir.options[1].element, 'HostDirent')
    .name(runnerInfo, 'RunnerInfo')
    .name(runnerInfo.shape.identity, 'HostIdentity')
    .name(unwrap(spawnExit.shape.spawnError), 'HostSpawnError')
    .name(unwrap(jobExit.shape.output), 'JobExitOutput')
    .name(owner, 'ArtifactOwner')
    .name(owner.options[0], 'JobArtifactOwner')
    .name(owner.options[1], 'StreamArtifactOwner')
    .name(netErrorCodeSchema, 'NetErrorCode')
    .name(serviceErrorCodeSchema, 'ServiceErrorCode')
    .name(helloErrorCodeSchema, 'HelloErrorCode')
    .variantPrefix(runnerToBackendMessageSchema, '')
    .variantPrefix(backendToRunnerMessageSchema, '')
  const roots: GoContractRoot[] = [
    { name: 'RunnerMessage', schema: runnerToBackendMessageSchema, codecs: ['json', 'msgpack'] },
    { name: 'BackendMessage', schema: backendToRunnerMessageSchema, codecs: ['json', 'msgpack'] },
    { name: 'ManagedBoot', schema: managedBootSchema, codecs: ['json'] },
  ]
  for (const root of roots)
    generator.type(root.schema, root.name)
  const inbound = unionOptions(backendToRunnerMessageSchema)
  // The id a file or working-tree request answers under (`runner.md`).
  const requestIds = (prefix: string): string => inbound
    .filter(option => tagOf(option, 'type').startsWith(prefix))
    .map(option => `case ${goName(tagOf(option, 'type'))}:\nreturn m.ID, true`).join('\n')
  const frames = ['RunnerMessage', 'BackendMessage'].map(name => `// Decode${name}Frame decodes a runner-protocol frame within MaxMessageBytes.
func Decode${name}Frame(frame []byte) (${name}, error) {
if len(frame) > MaxMessageBytes {
return nil, fmt.Errorf("malformed runner-protocol frame: %d bytes is over the %d-byte limit", len(frame), MaxMessageBytes)
}
return Decode${name}Msgpack(frame)
}

// Encode${name}Frame encodes a runner-protocol frame; a frame over
// MaxMessageBytes fails with a MessageTooLargeError.
func Encode${name}Frame(message ${name}) ([]byte, error) {
frame, err := Encode${name}Msgpack(message)
if err != nil {
return nil, err
}
if len(frame) > MaxMessageBytes {
return nil, &MessageTooLargeError{Size: len(frame)}
}
return frame, nil
}`).join('\n\n')
  const extra = `${exportedConstants(runnerMessages, runnerSchemas)}

// FsRequestID is the id of a file request, which its reply names.
func FsRequestID(message BackendMessage) (string, bool) {
switch m := message.(type) {
${requestIds('fs_')}
}
return "", false
}

// GitRequestID is the id of a working-tree request, which its reply names.
func GitRequestID(message BackendMessage) (string, bool) {
switch m := message.(type) {
${requestIds('git_')}
}
return "", false
}

// MessageTooLargeError is a message over MaxMessageBytes that this end was
// about to send: the request it belongs to fails with too_large, and the
// connection stays.
type MessageTooLargeError struct {
Size int
}

func (e *MessageTooLargeError) Error() string {
return fmt.Sprintf("a %d-byte runner message is over the %d-byte limit", e.Size, MaxMessageBytes)
}

// Code is the failure code a sender reports for the request.
func (e *MessageTooLargeError) Code() string {
return "too_large"
}

${frames}`
  return assemble('runnerwire', [
    'packages/runner-protocol/src/schemas.ts', 'packages/runner-protocol/src/messages.ts',
    'packages/runner-protocol/src/managed-boot.ts',
  ], true, roots, generator, extra, ['fmt'])
}

function manifest(): GoContractPackage {
  const generator = new GoZodTypes({ runtime: RUNTIME, foreign: foreign('cmdservice', cmdserviceNames) })
  const node = resolveLazy(manifestNodeSchema) as z.ZodUnion<[z.ZodObject, typeof manifestLeafSchema]>
  const [rpcLeaf, nativeLeaf] = manifestLeafSchema.options
  generator
    .name(manifestNodeSchema, 'Node')
    .name(node.options[0], 'Group')
    .name(rpcLeaf, 'RPCLeaf')
    .name(nativeLeaf, 'NativeLeaf')
    .name(unwrap(rpcLeaf.shape.output), 'LeafOutput')
    .name(nativeLeaf.shape.binding, 'NativeLeafBinding')
    .name(manifestSchema.shape.roots.valueType, 'Root')
  const roots: GoContractRoot[] = [
    { name: 'Manifest', schema: manifestSchema, codecs: ['json'] },
    { name: 'Node', schema: manifestNodeSchema, codecs: ['json'] },
  ]
  for (const root of roots)
    generator.type(root.schema, root.name)
  return assemble('manifest', ['packages/command-loader/src/manifest/schema.ts'], false, roots, generator, '')
}

function browser(): GoContractPackage {
  const generator = new GoZodTypes({ runtime: RUNTIME })
  const operations = Object.entries(browserOperations)
  const kind = (operation: string): string => goName(operation)
  const inputShape = (operation: keyof typeof browserOperations): Record<string, Schema> => browserOperations[operation].input.shape
  const info = browserOperations.info.result
  generator
    .name(browserErrorSchema, 'BrowserFailure')
    .name(browserErrorCodeSchema, 'BrowserErrorCode')
    .name(browserTargetSchema, 'BrowserTarget')
    .name(browserQuerySchema, 'BrowserQuery')
    .name(browserNodeSchema, 'BrowserNode')
    .name(browserCreatedBySchema, 'BrowserCreatedBy')
    .variantPrefix(browserCreatedBySchema, 'CreatedBy')
    .name(browserTabSchema, 'BrowserTab')
    .name(browserTreeNodeSchema, 'BrowserTreeNode')
    .name(browserViewportSchema, 'BrowserViewport')
    .name(browserViewportModeSchema, 'BrowserViewportMode')
    .name(browserReleaseSchema, 'BrowserRelease')
    .name(browserInstallationSchema, 'BrowserInstallation')
    .name(browserRuntimeConfigSchema, 'BrowserRuntimeConfig')
    .name(browserOperations.click.result, 'ActionResult')
    .name(browserOperations.goto.result, 'NavigatedResult')
    .name(browserOperations.find.result, 'MatchesResult')
    .name(browserOperations['dialog.accept'].result, 'DialogOutcome')
    .name(unwrap(unwrap(info.shape.dialog)), 'Dialog')
    .name((unwrap(unwrap(info.shape.dialog)) as z.ZodObject).shape.type, 'DialogType')
    .name(unwrap(inputShape('open').load), 'LoadEvent')
    .name((unwrap(inputShape('click').modifier) as z.ZodArray<z.ZodType>).element, 'Modifier')
    .name(unwrap(inputShape('clipboard.write').mime), 'MimeType')
    .name(unwrap(inputShape('content.read').format), 'ContentFormat')
    .name(unwrap(inputShape('screenshot').output), 'OutputFile')
    .name((browserOperations['assets.list'].result.shape.assets as z.ZodArray<z.ZodObject>).element.shape.kind, 'AssetKind')
    .name(browserNodeSchema.shape.value.unwrap().options[0], 'NodeTextValue')
    .name(browserNodeSchema.shape.value.unwrap().options[1], 'NodeNumberValue')
    .name(browserOperations.read.result.options[0], 'ReadValue')
    .name(browserOperations.read.result.options[1], 'ReadValues')
    .name(browserOperations['clipboard.read'].result.options[0], 'ClipboardText')
    .name(browserOperations['clipboard.read'].result.options[1], 'ClipboardItems')
    .name(browserOperations['content.read'].result.options[0], 'ContentInline')
    .name(browserOperations['content.read'].result.options[1], 'ContentFile')
    .name(liveControlOptionSchema, 'LiveControlOption')
    .name(liveControlSchema, 'LiveControl')
    .name(liveTabSchema, 'LiveTab')
    .name(liveDialogSchema, 'LiveDialog')
    .variantPrefix(liveViewerMessageSchema, 'LiveViewer')
    .variantPrefix(liveModuleMessageSchema, 'LiveModule')
  const roots: GoContractRoot[] = [
    ['BrowserFailure', browserErrorSchema], ['BrowserTarget', browserTargetSchema],
    ['BrowserQuery', browserQuerySchema], ['BrowserNode', browserNodeSchema],
    ['BrowserCreatedBy', browserCreatedBySchema], ['BrowserViewport', browserViewportSchema],
    ['BrowserRelease', browserReleaseSchema], ['BrowserInstallation', browserInstallationSchema],
    ['BrowserRuntimeConfig', browserRuntimeConfigSchema],
    ['LiveViewerMessage', liveViewerMessageSchema], ['LiveModuleMessage', liveModuleMessageSchema],
  ].map(([name, schema]) => ({ name: name as string, schema: schema as Schema, codecs: ['json'] as Codec[] }))
  for (const root of roots)
    generator.type(root.schema, root.name)
  const inputs = operations.map(([operation, schemas]) => generator.type(schemas.input, `${kind(operation)}Input`))
  const results = operations.map(([operation, schemas]) => generator.type(schemas.result, `${kind(operation)}Result`))
  const allRoots = [
    ...roots,
    ...operations.map(([, schemas], index) => ({ name: inputs[index]!, schema: schemas.input as Schema, codecs: ['json'] as Codec[] })),
    ...operations.flatMap(([, schemas], index) => roots.some(root => root.name === results[index])
      || operations.findIndex(([, other]) => other.result === schemas.result) !== index
      ? []
      : [{ name: results[index]!, schema: schemas.result as Schema, codecs: ['json'] as Codec[] }]),
  ]
  const targetKeys = Object.keys(browserTargetSchema.shape)
  const byInput = (filter: (shape: Record<string, Schema>) => boolean, body: (input: string, shape: Record<string, Schema>) => string): string =>
    operations.flatMap(([, schemas], index) => filter(schemas.input.shape)
      ? [`case ${inputs[index]}:\n${body(inputs[index]!, schemas.input.shape)}`]
      : []).join('\n')
  const extra = `${exportedConstants(browserIndex, browserLive)}

// BrowserCommand is the arguments of one browser operation.
type BrowserCommand interface {
isBrowserCommand()
Validate() error
ToValue() any
}

${inputs.map(input => `func (${input}) isBrowserCommand() {}`).join('\n')}

// ParseBrowserCommand parses and validates an operation's arguments.
func ParseBrowserCommand(operation string, args any) (BrowserCommand, error) {
switch operation {
${operations.map(([operation], index) => `case ${goString(operation)}:\nreturn Parse${inputs[index]}(args)`).join('\n')}
}
return nil, zodrt.Invalid("unknown browser operation %q", operation)
}

// CommandOperation names the operation of a command.
func CommandOperation(command BrowserCommand) string {
switch command.(type) {
${operations.map(([operation], index) => `case ${inputs[index]}:\nreturn ${goString(operation)}`).join('\n')}
}
return ""
}

// CommandTimeout is the command's whole-operation deadline, or the
// operation's default.
func CommandTimeout(command BrowserCommand) time.Duration {
millis := int64(0)
switch input := command.(type) {
${operations.map(([operation], index) => `case ${inputs[index]}:\nmillis = ${browserDefaultTimeout(operation)}\nif input.Timeout.Present {\nmillis = input.Timeout.Value\n}`).join('\n')}
}
return time.Duration(millis) * time.Millisecond
}

// CommandTab is the tab a command acts on.
func CommandTab(command BrowserCommand) (string, bool) {
switch input := command.(type) {
${byInput(shape => 'tab' in shape, () => 'return input.Tab, true')}
}
return "", false
}

// CommandWaitURL is the URL a command expects after its action.
func CommandWaitURL(command BrowserCommand) (string, bool) {
switch input := command.(type) {
${byInput(shape => 'wait-url' in shape, () => 'return input.WaitURL.Value, input.WaitURL.Present')}
}
return "", false
}

// CommandTarget is the element a command addresses.
func CommandTarget(command BrowserCommand) (BrowserTarget, bool) {
switch input := command.(type) {
${byInput(shape => 'ref' in shape, () => `return BrowserTarget{${targetKeys.map(key => `${goName(key)}: input.${goName(key)}`).join(', ')}}, true`)}
}
return BrowserTarget{}, false
}

// ValidateResult validates an operation's result and returns it encoded.
func ValidateResult(operation string, value any) (any, error) {
switch operation {
${operations.map(([operation], index) => `case ${goString(operation)}:\nresult, err := Parse${results[index]}(value)\nif err != nil {\nreturn nil, err\n}\nreturn result.ToValue(), nil`).join('\n')}
}
return nil, zodrt.Invalid("unknown browser operation %q", operation)
}

// Operations lists the browser operations as their commands name them.
func Operations() []string {
return []string{${operations.map(([operation]) => goString(`browser.${operation}`)).join(', ')}}
}`
  return assemble('browser', ['packages/browser-protocol/src/index.ts', 'packages/browser-protocol/src/live.ts'], false, allRoots, generator, extra, ['time', RUNTIME])
}

function machineswire(): GoContractPackage {
  const generator = new GoZodTypes({ runtime: RUNTIME, foreign: foreign('runnerwire', [['ManagedBoot', managedBootSchema]]) })
  generator
    .name(imageStateSchema, 'MachineImageState')
    .name(runtimeStateSchema, 'MachineRuntimeState')
    .variantPrefix(machineRequestSchema, '')
    .variantPrefix(machineResponseSchema, 'Response')
  const ops = Object.entries(machineOps)
  for (const [op, schemas] of ops)
    generator.name(schemas.params, `${goName(op)}Params`)
  const roots: GoContractRoot[] = [
    { name: 'MachineRequest', schema: machineRequestSchema, codecs: ['json'] },
    { name: 'MachineResponse', schema: machineResponseSchema, codecs: ['json'] },
  ]
  for (const root of roots)
    generator.type(root.schema, root.name)
  // A reply's result is validated by its op's schema once the client knows
  // which op it answers.
  const results = ops.map(([op, schemas]) => generator.valueRoot(schemas.result, `${goName(op)}Result`)).join('\n\n')
  return assemble('machineswire', ['packages/machines/src/protocol.ts', 'packages/machines/src/provisioner.ts'], false, roots, generator, results)
}

/** Every contract package, in dependency order. */
export function goContractPackages(): GoContractPackage[] {
  return [cmdservice(), runnerwire(), manifest(), browser(), machineswire()]
}

function goFile(contract: GoContractPackage): string {
  // Standard library paths have no dot in their first element.
  const standard = contract.imports.filter(path => !path.split('/')[0]!.includes('.'))
  const modules = contract.imports.filter(path => !standard.includes(path))
  const imports = [...standard, ...(standard.length > 0 && modules.length > 0 ? [''] : []), ...modules]
  return `// Code generated by scripts/generate-go-contracts.ts from ${contract.sources.join(', ')}. DO NOT EDIT.

package ${contract.name}

import (
${imports.map(path => path ? goString(path) : '').join('\n')}
)

${contract.body}
`
}

/** The table the conformance test runs the recorded corpus against. */
function corpusFile(contract: GoContractPackage): string {
  const entries = contract.roots.map(root => `${goString(root.name)}: {
${root.codecs.map(codec => {
    const label = codec === 'json' ? 'JSON' : 'Msgpack'
    return `${goString(codec)}: contracttest.RoundTrip(Decode${root.name}${label}, Encode${root.name}${label}),`
  }).join('\n')}
},`)
  return `// Code generated by scripts/generate-go-contracts.ts. DO NOT EDIT.

package ${contract.name}

import ${goString(`${MODULE}/contracttest`)}

var corpusRoots = contracttest.Roots{
${entries.join('\n')}
}
`
}

if (import.meta.main) {
  const root = join(import.meta.dir, '..')
  const paths: string[] = []
  for (const contract of goContractPackages()) {
    const directory = join(root, 'internal/contract', contract.name)
    await mkdir(directory, { recursive: true })
    const path = join(directory, 'zz_generated.go')
    await writeFile(path, goFile(contract))
    const corpus = join(directory, 'zz_generated_corpus_test.go')
    await writeFile(corpus, corpusFile(contract))
    paths.push(path, corpus)
  }
  // The generator writes unformatted Go; gofmt is the one formatter of Go.
  const format = Bun.spawnSync(['gofmt', '-w', ...paths], { stderr: 'inherit' })
  if (!format.success)
    throw new Error('gofmt rejected the generated Go')
}
