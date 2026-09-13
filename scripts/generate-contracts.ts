import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { z } from 'zod'
import {
  commandArgsSchema, commandErrorSchema, completionSchema, invocationSchema,
  serviceInfoSchema, nativePackageSchema, NATIVE_TARGETS, NATIVE_PROTOCOL_VERSION,
  MAX_METADATA_BYTES, MAX_RECORD_BYTES, MAX_INVOCATIONS, INFO_PATH, INVOKE_PATH, SHUTDOWN_PATH,
} from '../packages/command-protocol/src/index'
import { manifestSchema, manifestNodeSchema } from '../packages/command-loader/src/manifest/schema'
import {
  backendToRunnerMessageSchema, runnerToBackendMessageSchema, bytesSchema,
} from '../packages/runner-protocol/src/schemas'
import { JOB_VIEW_BYTES, RUNNER_PROTOCOL_VERSION } from '../packages/runner-protocol/src/messages'
import { RustZodTypes, rustField, rustPascal, rustString } from './rust-zod'

function flatten(schema: z.core.$ZodType): z.ZodObject[] {
  const def = (schema as z.core.$ZodTypes)._zod.def
  if (def.type === 'union')
    return def.options.flatMap(flatten)
  if (def.type !== 'object')
    throw new Error('Wire variants must be objects')
  return [schema as z.ZodObject]
}

function wire(): string {
  const generator = new RustZodTypes({ bytes: bytesSchema, dateType: 'super::Timestamp' })
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
  const types = { PackageDescriptor: nativePackageSchema, ServiceInfo: serviceInfoSchema,
    CommandError: commandErrorSchema, Completion: completionSchema, Invocation: invocationSchema }
  for (const [name, schema] of Object.entries(types))
    generator.type(schema, name)
  const checks = Object.entries(types).filter(([name]) => ['PackageDescriptor', 'Invocation'].includes(name)).map(([name, schema]) =>
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
    pub const INFO_PATH: &str = ${rustString(INFO_PATH)};
    pub const INVOKE_PATH: &str = ${rustString(INVOKE_PATH)};
    pub const SHUTDOWN_PATH: &str = ${rustString(SHUTDOWN_PATH)};`
}

const [target, output] = process.argv.slice(2)
if (!output || !['runner', 'command-service'].includes(target ?? ''))
  throw new Error('Usage: generate-contracts.ts <runner|command-service> <OUT_DIR>')
const sources = target === 'runner' ? { wire: wire(), manifest: manifest() } : { protocol: commandProtocol() }
for (const [name, source] of Object.entries(sources))
  await writeFile(join(output, `${name}.rs`), `// Generated from authoritative Zod schemas.\n${source}\n`)
