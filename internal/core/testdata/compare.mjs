// Compare executable schemas; normalize only unordered JSON Schema sets.
import {readFileSync, readdirSync, writeFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {stripTypeScriptTypes, createRequire} from 'node:module';
import {isDeepStrictEqual} from 'node:util';
const {z} = createRequire(`${process.cwd()}/package.json`)('zod');
const [generated, reference, fixturesPath] = process.argv.slice(2);

const referencePath = `${reference}/contracts.ts`;
function schemas(path, imports = {}) {
  const source = readFileSync(path, 'utf8');
  const names = [...source.matchAll(/export const (\w+Schema) = /g)].map(match => match[1]);
  const executable = stripTypeScriptTypes(source).replace(/^import .*$/gm, '').replaceAll('export const ', 'const ');
  return runInNewContext(`${executable}\n;({${names.join(',')}})`, {z, ...imports});
}
function normalize(value, key = '') {
  if (Array.isArray(value)) {
    const result = value.map(item => normalize(item));
    if (['required', 'enum', 'anyOf', 'oneOf', 'allOf', 'type'].includes(key)) {
      result.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
    }
    return result;
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, normalize(value[key], key)]));
  }
  return value;
}
const go = schemas(`${generated}/contracts.ts`);
const rust = schemas(referencePath);
// The reference also contains framewire's types. Select core's exports by
// Rust ownership, independently of what the Go generator produced.
const rustSource = new URL('../../../crates/shared-types/src/', import.meta.url);
const ownedTypes = new Set(readdirSync(rustSource).filter(name => name.endsWith('.rs')).flatMap(name =>
  [...readFileSync(new URL(name, rustSource), 'utf8').matchAll(/pub (?:struct|enum) (\w+)/g)].map(match => match[1])));
const ownedSchemas = new Set([...ownedTypes].map(name => name[0].toLowerCase() + name.slice(1) + 'Schema'));
function exportsAt(path) {
  return [...readFileSync(path, 'utf8').matchAll(/^export (?:const|type|function) (\w+)/gm)].map(match => match[1]).sort();
}
const expectedExports = exportsAt(referencePath).filter(name => ownedTypes.has(name) || ownedSchemas.has(name));
const actualExports = exportsAt(`${generated}/contracts.ts`);
if (!isDeepStrictEqual(actualExports, expectedExports)) {
  throw new Error(`Core exports differ: missing=${JSON.stringify(expectedExports.filter(name => !actualExports.includes(name)))} extra=${JSON.stringify(actualExports.filter(name => !expectedExports.includes(name)))}`);
}
console.log(`Core contract exports exactly match reference: ${actualExports.length}`);
const names = Object.keys(go).sort();
const differences = [];
for (const name of names) {
  const left = normalize(z.toJSONSchema(go[name], {io: 'input', reused: 'inline'}));
  const right = normalize(z.toJSONSchema(rust[name], {io: 'input', reused: 'inline'}));
  if (!isDeepStrictEqual(left, right)) differences.push(name);
  if (name === 'blockSchema') {
    writeFileSync(`${generated}/go-block-schema.json`, JSON.stringify(left, null, 2) + '\n');
    writeFileSync(`${generated}/rust-block-schema.json`, JSON.stringify(right, null, 2) + '\n');
  }
}
const fixtures = JSON.parse(readFileSync(`${fixturesPath}/blocks.json`));
const cases = JSON.parse(readFileSync(`${fixturesPath}/blocks-mutations.json`)).refused;
for (const [label, schema] of [['go', go.blockSchema], ['rust', rust.blockSchema]]) {
  let accepted = 0;
  for (const fixture of fixtures) {
    const parsed = schema.safeParse(fixture);
    if (!parsed.success) throw new Error(`${label} rejects ${fixture.id}: ${parsed.error}`);
    accepted++;
  }
  let refused = 0;
  for (const tc of cases) {
    const value = structuredClone(fixtures.find(fixture => fixture.id === tc.fixture));
    function parent(pointer) {
      const parts = pointer.slice(1).split('/').map(part => part.replaceAll('~1', '/').replaceAll('~0', '~'));
      const key = parts.pop();
      return [parts.reduce((target, part) => target[part], value), key];
    }
    for (const pointer of tc.remove ?? []) { const [target, key] = parent(pointer); delete target[key]; }
    for (const [pointer, replacement] of Object.entries(tc.set ?? {})) { const [target, key] = parent(pointer); target[key] = replacement; }
    const result = schema.safeParse(value);
    if (result.success === tc.webAppRefuses) throw new Error(`${label}: unexpected result for ${tc.why}`);
    if (!result.success) refused++;
  }
  const context = structuredClone(fixtures.find(value => value.type === 'context'));
  context.source = '😀'.repeat(64);
  if (!schema.safeParse(context).success) throw new Error(`${label}: Unicode scalar bound rejects 64 astral scalars`);
  context.source += '😀';
  if (schema.safeParse(context).success) throw new Error(`${label}: Unicode scalar bound accepts 65 astral scalars`);
  console.log(`${label}: fixtures accepted=${accepted}; web mutations refused=${refused}; tolerated=${cases.length-refused}; Unicode 64/65 boundary passed`);
}
console.log(`Shared named schemas compared=${names.length}; differences=${JSON.stringify(differences)}`);

writeFileSync(`${generated}/schema-comparison.json`, JSON.stringify({shared: names, differences, exports: actualExports}, null, 2)+'\n');
if (differences.length) process.exitCode = 1;
// Keep a literal source diff alongside the normalized semantic comparison.
for (const [path, output] of [[referencePath, `${generated}/rust-corresponding.ts`], [`${generated}/contracts.ts`, `${generated}/go-corresponding.ts`]]) {
  const source = readFileSync(path, 'utf8');
  const declarations = [...source.matchAll(/export const (\w+Schema) = [\s\S]*?\nexport type [^\n]+/g)];
  writeFileSync(output, declarations.filter(match => names.includes(match[1])).map(match => match[0]).join('\n\n')+'\n');
}


function tableModule(path) {
 const source = readFileSync(path,'utf8');
 const names = [...source.matchAll(/export (?:const|function) (\w+)/g)].map(match => match[1]);
 const executable = stripTypeScriptTypes(source).replace(/^import .*$/gm,'').replaceAll('export const ','const ').replaceAll('export function ','function ');
 return runInNewContext(`${executable}\n;({${names.join(',')}})`);
}
const goTables = tableModule(`${generated}/tables.ts`);
const rustTables = tableModule(`${reference}/tables.ts`);
if (!isDeepStrictEqual(normalize(JSON.parse(JSON.stringify(goTables.PREVIEW_TYPES))), normalize(JSON.parse(JSON.stringify(rustTables.PREVIEW_TYPES))))) throw new Error('preview table differs');
const fileCases = JSON.parse(readFileSync(`${fixturesPath}/file-types.json`));
for (const name of ['previewMediaType','showsInPlace']) {
 for (const [input,want] of fileCases[name]) {
  if (goTables[name](input) !== want || rustTables[name](input) !== want) throw new Error(`${name}: ${input}`);
 }
}
// MAX_PAGE_MESSAGE_BYTES belongs to webapi, not shared-types.
const expectedTableExports = exportsAt(`${reference}/tables.ts`).filter(name => name !== 'MAX_PAGE_MESSAGE_BYTES');
const actualTableExports = exportsAt(`${generated}/tables.ts`);
if (!isDeepStrictEqual(actualTableExports, expectedTableExports)) {
  throw new Error(`Core table exports differ: ${JSON.stringify(actualTableExports)}`);
}
for (const name of ['ATTACHMENT_FILE_EXTENSIONS', 'VIDEO_FILE_EXTENSIONS']) {
  if (JSON.stringify(goTables[name]) !== JSON.stringify(rustTables[name])) {
    throw new Error(`${name} differs from reference`);
  }
}
console.log('Core table exports, scalar values, preview table and lookup fixtures exactly match reference');
