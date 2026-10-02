// Compare executable schemas; normalize only unordered JSON Schema sets.
import {readFileSync, writeFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {stripTypeScriptTypes} from 'node:module';
import {isDeepStrictEqual} from 'node:util';
import {z} from 'zod';

const referencePath = process.env.CONTRACTGEN_REFERENCE;
if (!referencePath) throw new Error('Set CONTRACTGEN_REFERENCE to the Rust contracts.ts');
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
const go = schemas('generated/protocol/contracts.ts');
const rust = schemas(referencePath);
const names = Object.keys(go).filter(name => name in rust).sort();
const differences = [];
for (const name of names) {
  const left = normalize(z.toJSONSchema(go[name], {io: 'input', reused: 'inline'}));
  const right = normalize(z.toJSONSchema(rust[name], {io: 'input', reused: 'inline'}));
  if (!isDeepStrictEqual(left, right)) differences.push(name);
  if (name === 'blockSchema') {
    writeFileSync('go-block-schema.json', JSON.stringify(left, null, 2) + '\n');
    writeFileSync('rust-block-schema.json', JSON.stringify(right, null, 2) + '\n');
  }
}
const fixtures = JSON.parse(readFileSync('../../../crates/shared-types/tests/shared-types/fixtures/blocks.json'));
const cases = JSON.parse(readFileSync('../../../crates/shared-types/tests/shared-types/fixtures/blocks-mutations.json')).refused;
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
console.log(`Additional Go exports=${JSON.stringify(Object.keys(go).filter(name => !(name in rust)).sort())}`);
writeFileSync('schema-comparison.json', JSON.stringify({shared: names, differences, additional: Object.keys(go).filter(name => !(name in rust)).sort()}, null, 2)+'\n');
if (differences.length || Object.keys(go).some(name => !(name in rust))) process.exitCode = 1;
// Keep a literal source diff alongside the normalized semantic comparison.
for (const [path, output] of [[referencePath, 'rust-corresponding.ts'], ['generated/protocol/contracts.ts', 'go-corresponding.ts']]) {
  const source = readFileSync(path, 'utf8');
  const declarations = [...source.matchAll(/export const (\w+Schema) = [\s\S]*?\nexport type [^\n]+/g)];
  writeFileSync(output, declarations.filter(match => names.includes(match[1])).map(match => match[0]).join('\n\n')+'\n');
}

const web = schemas('generated/web/web-api.ts', go);
const plugin = schemas('generated/plugin-test/plugin.ts');
const request = {id: 'id_ok', body: {type: 'text', text: 'hello'}, data: 'Zg==', label: null, extra: null};
if (!web.requestSchema.safeParse(request).success) throw new Error('valid request refused');
for (const invalid of [{...request, unknown: true}, {...request, data: 'Zh=='}, {...request, label: undefined}]) {
  if (web.requestSchema.safeParse(invalid).success) throw new Error('invalid request accepted');
}
const tree = {name: 'id_root', children: [{name: 'id_child', children: [], future: true}]};
if (!plugin.treeSchema.safeParse(tree).success) throw new Error('recursive tolerant tree refused');
if (plugin.treeSchema.safeParse({...tree, parent: null}).success) throw new Error('optional null accepted');
console.log('Cross-module imports, strict requests, canonical base64 and recursive tolerant objects passed');
