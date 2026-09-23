import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import {
  LIVE_FILE_CHUNK_BYTES, LIVE_FILE_HEADER_BYTES, LIVE_FRAME_KIND, LIVE_HEARTBEAT_MS, LIVE_MAX_FRAME_BYTES,
  LIVE_STALL_MS, LIVE_VIDEO_HEADER_BYTES, liveModuleMessageSchema, liveViewerMessageSchema,
} from '../packages/browser-protocol/src/live'
import { RustZodTypes, rustField, rustPascal, rustString } from './rust-zod'
import { BROWSER_CLIPBOARD_PNG_BYTES, BROWSER_CLIPBOARD_PNG_PIXELS, BROWSER_STDIN_BYTES, BROWSER_FETCH_URLS, BROWSER_CONSOLE_ENTRIES, BROWSER_CONSOLE_BYTES, BROWSER_CDP_EVENTS, BROWSER_CDP_BYTES, browserQuerySchema, browserErrorSchema, browserOperations, browserTargetSchema, browserNodeSchema, browserCreatedBySchema, browserViewportSchema, browserReleaseSchema, browserInstallationSchema, browserRuntimeConfigSchema, browserDefaultTimeout, BROWSER_DEFAULT_NODES, BROWSER_MAX_NODES, BROWSER_INLINE_BYTES } from '../packages/browser-protocol/src/index'


function browserProtocol(): string {
  const generator = new RustZodTypes()
  generator.type(browserErrorSchema, 'BrowserFailure')
  generator.type(browserTargetSchema, 'BrowserTarget')
  generator.type(browserQuerySchema, 'BrowserQuery')
  generator.type(browserNodeSchema, 'BrowserNode')
  generator.type(browserCreatedBySchema, 'BrowserCreatedBy')
  generator.type(browserViewportSchema, 'BrowserViewport')
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
  // The live view's protocol (`browser-live-view.md` § The stream).
  const derive = 'Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize'
  const liveInbound = generator.taggedEnum(liveViewerMessageSchema, 'LiveInbound', { prefix: 'LiveInbound', derive })
  const liveOutbound = generator.taggedEnum(liveModuleMessageSchema, 'LiveOutbound', { prefix: 'LiveOutbound', derive })
  return `${generator.finish()}
    ${liveInbound}
    ${liveOutbound}
    pub const LIVE_CONTROL_FRAME: u8 = ${LIVE_FRAME_KIND.control};
    pub const LIVE_VIDEO_FRAME: u8 = ${LIVE_FRAME_KIND.video};
    pub const LIVE_FILE_FRAME: u8 = ${LIVE_FRAME_KIND.file};
    pub const LIVE_MAX_FRAME_BYTES: usize = ${LIVE_MAX_FRAME_BYTES};
    pub const LIVE_FILE_CHUNK_BYTES: usize = ${LIVE_FILE_CHUNK_BYTES};
    pub const LIVE_VIDEO_HEADER_BYTES: usize = ${LIVE_VIDEO_HEADER_BYTES};
    pub const LIVE_FILE_HEADER_BYTES: usize = ${LIVE_FILE_HEADER_BYTES};
    pub const LIVE_HEARTBEAT_MS: u64 = ${LIVE_HEARTBEAT_MS};
    pub const LIVE_STALL_MS: u64 = ${LIVE_STALL_MS};
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
if (!output || target !== 'demi-commands')
  throw new Error('Usage: generate-contracts.ts demi-commands <OUT_DIR>')
const sources = { browser: browserProtocol() }
for (const [name, source] of Object.entries(sources))
  await writeFile(join(output, `${name}.rs`), `// Generated from authoritative Zod schemas.\n${source}\n`)
