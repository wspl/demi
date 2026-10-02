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
assert.deepEqual(JSON.parse(JSON.stringify(tables.ModelFileTypes)), expected.models);
assert.deepEqual(JSON.parse(JSON.stringify(tables.LiveViewFrameConstants)), expected.frames);
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
    assert.equal(tables.PREVIEW_TYPESByExtensions(extension)?.mediaType, entry.mediaType);
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
for (const entry of expected.models) {
  assert.equal(tables.ModelFileTypesByExtension(entry.extension)?.kind, entry.kind);
}
for (const entry of expected.frames) {
  assert.equal(tables.LiveViewFrameConstantsByName(entry.name)?.value, entry.value);
}
if (reference) {
  for (const [kind, name] of [['attachment', 'ATTACHMENT_FILE_EXTENSIONS'], ['video', 'VIDEO_FILE_EXTENSIONS']]) {
    assert.equal(JSON.stringify(tables.ModelFileTypes.filter(entry => entry.kind === kind).map(entry => entry.extension)), JSON.stringify(reference[name]));
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
console.log(`Tables: ${paths} paths, model-readable extensions and live frame constants passed; email/trimmed Zod passed${reference ? '; Rust lookups agree' : ''}`);
