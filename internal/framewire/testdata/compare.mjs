// Compare framewire's executable schemas and exports with the Rust reference.
// Usage: node internal/framewire/testdata/compare.mjs GENERATED REFERENCE
import {readFileSync, readdirSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {stripTypeScriptTypes, createRequire} from 'node:module';
import {isDeepStrictEqual} from 'node:util';
const {z} = createRequire(`${process.cwd()}/internal/framewire/testdata/generated/package.json`)('zod');
const [generated, reference] = process.argv.slice(2);
function schemas(path) {
  const source = readFileSync(path, 'utf8');
  const names = [...source.matchAll(/export const (\w+Schema) = /g)].map(match => match[1]);
  const executable = stripTypeScriptTypes(source).replace(/^import .*$/gm, '').replaceAll('export const ', 'const ');
  return runInNewContext(`${executable}\n;({${names.join(',')}})`, {z});
}
function normalize(value, key = '') {
  if (Array.isArray(value)) {
    const result = value.map(item => normalize(item));
    if (['required', 'enum', 'anyOf', 'oneOf', 'allOf', 'type'].includes(key)) result.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
    return result;
  }
  if (value && typeof value === 'object') return Object.fromEntries(Object.keys(value).sort().map(key => [key, normalize(value[key], key)]));
  return value;
}
const rustSource = new URL('../../../crates/conversation-socket-protocol/src/', import.meta.url);
const owned = new Set(readdirSync(rustSource).filter(name => name.endsWith('.rs')).flatMap(name =>
  [...readFileSync(new URL(name, rustSource), 'utf8').matchAll(/pub (?:struct|enum|type) (\w+)/g)].map(match => match[1])));
const ownedSchemas = new Set([...owned].map(name => name[0].toLowerCase() + name.slice(1) + 'Schema'));
function ownedExports(path) {
  return [...readFileSync(path, 'utf8').matchAll(/^export (?:const|type|function) (\w+)/gm)].map(match => match[1]).filter(name => owned.has(name) || ownedSchemas.has(name)).sort();
}
const goPath = `${generated}/contracts.ts`;
const rustPath = `${reference}/contracts.ts`;
if (!isDeepStrictEqual(ownedExports(goPath), ownedExports(rustPath))) throw new Error('framewire exports differ');
const go = schemas(goPath);
const rust = schemas(rustPath);
const names = Object.keys(rust).filter(name => ownedSchemas.has(name)).sort();
for (const name of names) {
  const left = normalize(z.toJSONSchema(go[name], {io:'input', reused:'inline'}));
  const right = normalize(z.toJSONSchema(rust[name], {io:'input', reused:'inline'}));
  if (!isDeepStrictEqual(left, right)) throw new Error(`schema differs: ${name}\nGo: ${JSON.stringify(left)}\nRust: ${JSON.stringify(right)}`);
}
const fixtures = JSON.parse(readFileSync(new URL('client-frames.json', import.meta.url)));
const server = JSON.parse(readFileSync(new URL('server-frames.json', import.meta.url)));
const table = JSON.parse(readFileSync(new URL('client-frames-mutations.json', import.meta.url)));
for (const [label, schemas] of [['go',go], ['rust',rust]]) {
  for (const [frames,schema] of [[fixtures,schemas.clientFrameSchema], [server,schemas.serverFrameSchema]]) {
    for (const frame of frames) if (!schema.safeParse(frame).success) throw new Error(`${label} refuses ${frame.type}`);
  }
  for (const category of ['accepted','refused']) {
    for (const tc of table[category]) {
      const value = structuredClone(tc.value ?? fixtures.find(frame => frame.type === tc.fixture));
      function parent(pointer) {
        const parts = pointer.slice(1).split('/').map(part => part.replaceAll('~1','/').replaceAll('~0','~'));
        const key = parts.pop();
        return [parts.reduce((target,part) => target[part],value),key];
      }
      for (const pointer of tc.remove ?? []) { const [target,key] = parent(pointer); delete target[key]; }
      for (const [pointer,replacement] of Object.entries(tc.set ?? {})) { const [target,key] = parent(pointer); target[key] = replacement; }
      if (schemas.clientFrameSchema.safeParse(value).success === (category === 'refused' && tc.webAppRefuses)) throw new Error(`${label}: ${tc.why}`);
    }
  }
  for (const variant of schemas.clientFrameSchema.options) {
    if (z.toJSONSchema(variant).additionalProperties !== false) throw new Error(`${label}: client frame not strict`);
  }
  for (const frame of server) {
    if (!schemas.serverFrameSchema.safeParse({...frame,future:true}).success) throw new Error(`${label}: server not tolerant`);
  }
  console.log(`${label}: ${fixtures.length} client / ${server.length} server frames; ${table.refused.length} refused and ${table.accepted.length} accepted mutation expectations match`);
}
console.log(`Framewire: ${names.length} schemas and ${ownedExports(goPath).length} exports match the Rust reference`);
