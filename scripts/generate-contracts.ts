import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { z } from 'zod'
import {
  commandArgsSchema, commandErrorSchema, completionSchema, invocationSchema,
  serviceInfoSchema, nativePackageSchema, NATIVE_TARGETS, NATIVE_PROTOCOL_VERSION,
  MAX_METADATA_BYTES, MAX_RECORD_BYTES, MAX_INVOCATIONS, INFO_PATH, INVOKE_PATH, CONVERSATION_PATH, SHUTDOWN_PATH,
  conversationRequestSchema, conversationStatusSchema,
  artifactLocationSchema,
  editContextSchema, editCopiesSchema, editFileSchema, editJournalSchema,
  EDIT_FILE_BYTES, EDIT_JOB_BYTES, EDIT_JOB_FILES, EDIT_JOB_SEGMENTS,
} from '../packages/command-protocol/src/index'
import { manifestSchema, manifestNodeSchema } from '../packages/command-loader/src/manifest/schema'
import { RustZodTypes, rustField, rustPascal, rustString } from './rust-zod'
import { BROWSER_CLIPBOARD_PNG_BYTES, BROWSER_CLIPBOARD_PNG_PIXELS, BROWSER_STDIN_BYTES, BROWSER_FETCH_URLS, BROWSER_CONSOLE_ENTRIES, BROWSER_CONSOLE_BYTES, BROWSER_CDP_EVENTS, BROWSER_CDP_BYTES, browserQuerySchema, browserErrorSchema, browserOperations, browserTargetSchema, browserNodeSchema, browserCreatedBySchema, browserReleaseSchema, browserInstallationSchema, browserRuntimeConfigSchema, browserDefaultTimeout, BROWSER_DEFAULT_NODES, BROWSER_MAX_NODES, BROWSER_INLINE_BYTES } from '../packages/browser-protocol/src/index'

function flatten(schema: z.core.$ZodType): z.ZodObject[] {
  const def = (schema as z.core.$ZodTypes)._zod.def
  if (def.type === 'union')
    return def.options.flatMap(flatten)
  if (def.type !== 'object')
    throw new Error('Wire variants must be objects')
  return [schema as z.ZodObject]
}

async function wire(): Promise<string> {
  const { backendToRunnerMessageSchema, runnerToBackendMessageSchema, bytesSchema } = await import('../packages/runner-protocol/src/schemas')
  const { JOB_VIEW_BYTES, RUNNER_PROTOCOL_VERSION } = await import('../packages/runner-protocol/src/messages')
  const generator = new RustZodTypes({
    bytes: bytesSchema,
    dateType: 'super::Timestamp',
    overrides: new Map<z.core.$ZodType, string>([
      [editCopiesSchema, 'demi_command_service::protocol::EditCopies'],
      [nativePackageSchema, 'demi_command_service::protocol::PackageDescriptor'],
      [artifactLocationSchema, 'demi_command_service::protocol::ArtifactLocation'],
    ]),
  })
  const schemas = flatten(backendToRunnerMessageSchema)
  const variants = schemas.map(schema => {
    const tag = (schema.shape.type as z.ZodLiteral<string>).value
    const fields = generator.fields(schema, rustPascal(tag), new Set(['type'])).replaceAll('pub ', '')
    return `#[serde(rename = ${rustString(tag)})]\n${rustPascal(tag)} {\n${fields}\n},`
  })
  const requestIds = (prefix: string) => schemas.flatMap(schema => {
    const tag = (schema.shape.type as z.ZodLiteral<string>).value
    return tag.startsWith(prefix) ? [`Self::${rustPascal(tag)} { id, .. } => Some(id),`] : []
  })
  const fsIds = requestIds('fs_')
  const gitIds = requestIds('git_')
  const outgoing = flatten(runnerToBackendMessageSchema).map(schema => {
    const literals = Object.entries(schema.shape).filter(([, child]) => child._zod.def.type === 'literal')
    const tag = (schema.shape.type as z.ZodLiteral<string>).value
    const op = schema.shape.op ? (schema.shape.op as z.ZodLiteral<string>).value : ''
    const name = `${rustPascal(tag)}${rustPascal(op)}`
    const constants = new Set(literals.map(([key]) => key))
    const fields = Object.entries(schema.shape).filter(([key]) => !constants.has(key))
    const args = fields.map(([key, child]) => `${rustField(key)}: ${generator.type(child, `${name}${rustPascal(key)}`)}`)
    const body = generator.fields(schema, name, constants, false).replaceAll('pub ', '')
    const checks = fields.map(([key, child]) => generator.validate(child, `&${rustField(key)}`)).join('\n')
    return `#[allow(clippy::too_many_arguments)]
      pub fn ${rustField(name)}(${args.join(', ')}) -> Result<super::Outbound, super::WireError> {
        ${checks ? `let checked: Result<(), String> = (|| { ${checks}\nOk(()) })();
        checked.map_err(super::WireError::Invalid)?;` : ''}
        #[derive(serde::Serialize)]
        struct Message {
          ${literals.map(([key]) => `#[serde(rename = ${rustString(key)})]\n${rustField(key)}: &'static str,`).join('\n')}
          ${body}
        }
        super::encode(&Message {
          ${literals.map(([key, child]) => `${rustField(key)}: ${rustString((child as z.ZodLiteral<string>).value)},`).join('\n')}
          ${fields.map(([key]) => `${rustField(key)},`).join('\n')}
        })
      }`
  })
  return `pub const VERSION: u64 = ${RUNNER_PROTOCOL_VERSION};
    pub const JOB_VIEW_BYTES: usize = ${JOB_VIEW_BYTES};
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    #[serde(tag = "type", deny_unknown_fields)]
    pub enum Inbound { ${variants.join('\n')} }
    impl Inbound {
      pub fn fs_request_id(&self) -> Option<&str> {
        match self { ${fsIds.join('\n')} _ => None }
      }
      pub fn git_request_id(&self) -> Option<&str> {
        match self { ${gitIds.join('\n')} _ => None }
      }
    }
    ${generator.finish()}
    ${outgoing.join('\n')}`
}

function manifest(): string {
  const generator = new RustZodTypes({
    boxedUnions: true,
    overrides: new Map([[nativePackageSchema, 'demi_command_service::protocol::PackageDescriptor']]),
  })
  generator.type(manifestNodeSchema, 'Node')
  generator.type(manifestSchema, 'Manifest')
  // Recursive validation is bounded by the runner's manifest admission limit.
  return generator.finish()
}

function commandProtocol(): string {
  const generator = new RustZodTypes({
    overrides: new Map([[commandArgsSchema, 'serde_json::Value']]),
    jsonObjects: new Set([commandArgsSchema]),
  })
  const types = { EditContext: editContextSchema, EditCopies: editCopiesSchema,
    EditFile: editFileSchema, EditJournal: editJournalSchema,
    ConversationRequest: conversationRequestSchema,
    ConversationStatus: conversationStatusSchema,
    ArtifactLocation: artifactLocationSchema,
    PackageDescriptor: nativePackageSchema, ServiceInfo: serviceInfoSchema,
    CommandError: commandErrorSchema, Completion: completionSchema, Invocation: invocationSchema }
  for (const [name, schema] of Object.entries(types))
    generator.type(schema, name)
  const checks = Object.entries(types).filter(([name]) => ['PackageDescriptor', 'Invocation', 'ConversationRequest', 'ConversationStatus', 'EditContext', 'EditJournal'].includes(name)).map(([name, schema]) =>
    `pub fn ${rustField(name)}_validate(value: &${name}) -> Result<(), String> {
      ${generator.validate(schema, 'value')}\nOk(())
    }`)
  return `${generator.finish()}
    ${checks.join('\n')}
    pub const VERSION: u64 = ${NATIVE_PROTOCOL_VERSION};
    pub const TARGETS: &[&str] = &[${NATIVE_TARGETS.map(rustString).join(', ')}];
    pub const MAX_METADATA_BYTES: usize = ${MAX_METADATA_BYTES};
    pub const MAX_RECORD_BYTES: usize = ${MAX_RECORD_BYTES};
    pub const MAX_INVOCATIONS: usize = ${MAX_INVOCATIONS};
    pub const EDIT_FILE_BYTES: usize = ${EDIT_FILE_BYTES};
    pub const EDIT_JOB_BYTES: u64 = ${EDIT_JOB_BYTES};
    pub const EDIT_JOB_FILES: usize = ${EDIT_JOB_FILES};
    pub const EDIT_JOB_SEGMENTS: u64 = ${EDIT_JOB_SEGMENTS};
    pub const INFO_PATH: &str = ${rustString(INFO_PATH)};
    pub const INVOKE_PATH: &str = ${rustString(INVOKE_PATH)};
    pub const CONVERSATION_PATH: &str = ${rustString(CONVERSATION_PATH)};
    pub const SHUTDOWN_PATH: &str = ${rustString(SHUTDOWN_PATH)};`
}

function browserProtocol(): string {
  const generator = new RustZodTypes()
  generator.type(browserErrorSchema, 'BrowserFailure')
  generator.type(browserTargetSchema, 'BrowserTarget')
  generator.type(browserQuerySchema, 'BrowserQuery')
  generator.type(browserNodeSchema, 'BrowserNode')
  generator.type(browserCreatedBySchema, 'BrowserCreatedBy')
  generator.type(browserReleaseSchema, 'BrowserRelease')
  generator.type(browserInstallationSchema, 'BrowserInstallation')
  generator.type(browserRuntimeConfigSchema, 'BrowserRuntimeConfig')
  const operations = Object.entries(browserOperations)
  const variants = operations.map(([name, schemas]) => {
    const kind = rustPascal(name.replaceAll('.', '_'))
    generator.type(schemas.input, `${kind}Input`)
    generator.type(schemas.result, `${kind}Result`)
    return `${kind}(${generator.type(schemas.input, `${kind}Input`)}),`
  })
  return `${generator.finish()}
    pub enum BrowserCommand { ${variants.join('\n')} }
    impl BrowserCommand {
      pub fn parse(operation: &str, args: serde_json::Value) -> Result<Self, serde_json::Error> {
        match operation {
          ${operations.map(([name]) => `${rustString(name)} => serde_json::from_value(args).map(Self::${rustPascal(name.replaceAll('.', '_'))}),`).join('\n')}
          _ => Err(serde::de::Error::custom("unknown browser operation")),
        }
      }
      pub fn timeout(&self) -> std::time::Duration {
        let value = match self {
          ${operations.map(([name]) => `Self::${rustPascal(name.replaceAll('.', '_'))}(input) => input.timeout.unwrap_or(${browserDefaultTimeout(name)}),`).join('\n')}
        };
        std::time::Duration::from_millis(value)
      }
      pub fn tab(&self) -> Option<&str> {
        match self {
          ${operations.map(([name, schemas]) => `Self::${rustPascal(name.replaceAll('.', '_'))}(input) => ${'tab' in schemas.input.shape ? 'Some(&input.tab)' : 'None'},`).join('\n')}
        }
      }
      pub fn wait_url(&self) -> Option<&str> {
        match self {
          ${operations.filter(([, schemas]) => 'wait-url' in schemas.input.shape).map(([name]) => `Self::${rustPascal(name.replaceAll('.', '_'))}(input) => input.wait_url.as_deref(),`).join('\n')}
          _ => None,
        }
      }
      pub fn target(&self) -> Option<BrowserTarget> {
        match self {
          ${operations.filter(([, schemas]) => 'ref' in schemas.input.shape).map(([name]) => `Self::${rustPascal(name.replaceAll('.', '_'))}(input) => Some(BrowserTarget { ${Object.keys(browserTargetSchema.shape).map(key => `${rustField(key)}: input.${rustField(key)}.clone()`).join(', ')} }),`).join('\n')}
          _ => None,
        }
      }
    }
    pub fn validate_result(operation: &str, value: serde_json::Value) -> Result<serde_json::Value, serde_json::Error> {
      match operation {
        ${operations.map(([name, schemas]) => `${rustString(name)} => serde_json::to_value(serde_json::from_value::<${generator.type(schemas.result, `${rustPascal(name.replaceAll('.', '_'))}Result`)}>(value)?),`).join('\n')}
        _ => Err(serde::de::Error::custom("unknown browser operation")),
      }
    }
    pub const DEFAULT_NODES: usize = ${BROWSER_DEFAULT_NODES};
    pub const MAX_NODES: usize = ${BROWSER_MAX_NODES};
    pub const INLINE_BYTES: usize = ${BROWSER_INLINE_BYTES};
    pub const STDIN_BYTES: usize = ${BROWSER_STDIN_BYTES};
    pub const CLIPBOARD_PNG_BYTES: usize = ${BROWSER_CLIPBOARD_PNG_BYTES};
    pub const CLIPBOARD_PNG_PIXELS: usize = ${BROWSER_CLIPBOARD_PNG_PIXELS};
    pub const FETCH_URLS: usize = ${BROWSER_FETCH_URLS};
    pub const CONSOLE_ENTRIES: usize = ${BROWSER_CONSOLE_ENTRIES};
    pub const CONSOLE_BYTES: usize = ${BROWSER_CONSOLE_BYTES};
    pub const CDP_EVENTS: usize = ${BROWSER_CDP_EVENTS};
    pub const CDP_BYTES: usize = ${BROWSER_CDP_BYTES};
    pub const OPERATIONS: &[&str] = &[${operations.map(([name]) => rustString(`browser.${name}`)).join(', ')}];`
}

const [target, output] = process.argv.slice(2)
if (!output || !['runner', 'command-service', 'demi-commands'].includes(target ?? ''))
  throw new Error('Usage: generate-contracts.ts <runner|command-service|demi-commands> <OUT_DIR>')
const sources = target === 'runner' ? { wire: await wire(), manifest: manifest() }
  : target === 'demi-commands' ? { browser: browserProtocol() } : { protocol: commandProtocol() }
for (const [name, source] of Object.entries(sources))
  await writeFile(join(output, `${name}.rs`), `// Generated from authoritative Zod schemas.\n${source}\n`)
