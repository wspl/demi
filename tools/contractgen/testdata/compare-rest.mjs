// Execute generated modules at the web boundary. No network, services or timers.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';
import {z} from 'zod';

const [directory, expectedPath, referencePath] = process.argv.slice(2);
function load(path) {
  const source = readFileSync(path, 'utf8');
  const names = [...source.matchAll(/export (?:const|function) (\w+)/g)].map(match => match[1]);
  const code = stripTypeScriptTypes(source)
    .replace(/^import .*$/gm, '')
    .replaceAll('export const ', 'const ')
    .replaceAll('export function ', 'function ');
  return runInNewContext(`${code}\n;({${names.join(',')}})`, {z});
}
const tables = load(`${directory}/protocol/tables.ts`);
const expected = JSON.parse(readFileSync(expectedPath, 'utf8'));
assert.deepEqual(JSON.parse(JSON.stringify(tables.PREVIEW_TYPES)), expected.preview);
assert.deepEqual(JSON.parse(JSON.stringify(tables.MODEL_FILE_TYPES)), expected.models);
assert.deepEqual(JSON.parse(JSON.stringify(tables.LIVE_VIEW_FRAME_CONSTANTS)), expected.frames);
const reference = referencePath ? load(referencePath) : undefined;
let paths = 0;
for (const entry of expected.preview) {
  for (const extension of entry.extensions) {
    for (const path of [`a.${extension}`, `/dir/a.${extension.toUpperCase()}`, `C:\\dir\\a.${extension}`, `a.b.${extension}`, `.${extension}`, `/a.${extension}/file`, `a.${extension}\\file`]) {
      const want = path.endsWith('/file') || path.endsWith('\\file') || path === `.${extension}` ? null : entry.mediaType;
      assert.equal(tables.previewMediaType(path), want, path);
      if (reference) assert.equal(tables.previewMediaType(path), reference.previewMediaType(path), path);
      paths++;
    }
  }
  assert.equal(tables.showsInPlace(entry.mediaType), entry.inPlace);
  if (reference) assert.equal(tables.showsInPlace(entry.mediaType), reference.showsInPlace(entry.mediaType));
}
for (const extension of ['txt', 'json', 'ts', 'html', 'csv', 'zip', 'gz', 'exe', 'unknown', 'pñg']) {
  const path = `test.${extension}`;
  assert.equal(tables.previewMediaType(path), null);
  if (reference) assert.equal(tables.previewMediaType(path), reference.previewMediaType(path));
  paths++;
}
assert.equal(tables.showsInPlace('IMAGE/PNG'), false);
assert.equal(tables.showsInPlace('application/unknown'), false);
if (reference) {
  for (const [kind, name] of [['attachment', 'ATTACHMENT_FILE_EXTENSIONS'], ['video', 'VIDEO_FILE_EXTENSIONS']]) {
    assert.equal(JSON.stringify(tables[name]), JSON.stringify(reference[name]));
    assert.equal(JSON.stringify(tables[name]), JSON.stringify(expected.models.filter(entry => entry.kind === kind).map(entry => entry.extension)));
  }
}

const schemas = load(`${directory}/web/web-api.ts`);
for (const name of ['emailSchema', 'receivedEmailSchema']) {
  const schema = schemas[name];
  assert.equal(schema.parse('ANA@example.test'), 'ANA@example.test');
  for (const invalid of [' a@example.test ', 'a..b@example.test', '.a@example.test', 'a.@example.test', 'a@host', 'a@-host.test', 'a@host.t', 'anä@example.test', '', `${'a'.repeat(242)}@example.test`]) {
    assert.equal(schema.safeParse(invalid).success, false, `${name}: ${invalid}`);
  }
}
assert.equal(schemas.receivedSchema.parse({email: 'ANA@example.test', future: true}).email, 'ANA@example.test');
assert.equal(schemas.receivedSchema.safeParse({email: ' ana@example.test '}).success, false);
assert.equal(schemas.nameSchema.parse('\ufeff  New name\t\u2003'), 'New name');
assert.equal(schemas.nameSchema.parse('\u0085name'), '\u0085name');
assert.equal(schemas.nameSchema.parse(` ${'😀'.repeat(8)} `), '😀'.repeat(8));
for (const invalid of ['', ' \ufeff\t\u2003', '😀'.repeat(9)]) {
  assert.equal(schemas.nameSchema.safeParse(invalid).success, false);
}
// Keep the Rust emitter's z.url form: validation preserves URL input text;
// the Go boundary stores the canonical serialization.
for (const input of ['https://api.kimi.com/coding/v1', 'http://127.0.0.1:8080', 'https://example.com:443/v1', 'https://bücher.example/v1']) {
  assert.equal(schemas.endpointUrlSchema.parse(input), input);
}
for (const input of ['http:example.com', '', 'api.openai.com/v1', 'ftp://example.test/', 'file:///etc', 'https://', 'https://example.com:65536/']) {
  assert.equal(schemas.endpointUrlSchema.safeParse(input).success, false, input);
}
console.log(`Tables: ${paths} paths, model-readable extensions and live frame constants passed; email/trimmed/http-url Zod passed${reference ? '; Rust lookups agree' : ''}`);

const replies = load(`${directory}/plugin-unions/plugin.ts`);
for (const schema of [replies.ensureReplySchema, replies.statusReplySchema]) {
  assert.equal(schema.safeParse({ok: false, code: 'install_failed', message: 'digest differs'}).success, true);
  assert.equal(schema.safeParse({ok: 'false', code: 'install_failed', message: 'digest differs'}).success, false);
  assert.equal(schema.safeParse({code: 'install_failed', message: 'digest differs'}).success, false);
}
assert.equal(replies.statusReplySchema.safeParse({ok: true, platform: 'darwin-arm64', installed: []}).success, true);
assert.equal(replies.ensureReplySchema.safeParse({ok: true, version: '2.1.3', path: '/opt/claude'}).success, true);
assert.equal('failureSchema' in replies, false);
assert.equal('bytesSchema' in replies, false);
assert.equal(replies.emptySchema.safeParse({}).success, true);
assert.equal(replies.emptySchema.safeParse({bytes: '/wA='}).success, true);
assert.equal(replies.emptySchema.safeParse({bytes: '/wA'}).success, false);
console.log('Boolean reply unions, private schemas, and byte-slice base64 passed');
