// Compare executable web schemas, normalizing unordered sets and equivalent recursive definitions.
// Usage: node compare.mjs <contractgen -ts-dir output> <Rust generated root>.
import {readFileSync, writeFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import {stripTypeScriptTypes, createRequire} from 'node:module';
import {isDeepStrictEqual} from 'node:util';

const packagePath = process.env.WEBAPI_NODE_PACKAGE ?? `${process.cwd()}/package.json`;
const {z} = createRequire(packagePath)('zod');
const [generated, reference] = process.argv.slice(2);

function schemas(path, imports = {}) {
  const source = readFileSync(path, 'utf8');
  const names = [...source.matchAll(/export const (\w+Schema) = /g)].map(match => match[1]);
  const executable = stripTypeScriptTypes(source)
      .replace(/^import .*$/gm, '')
      .replaceAll('export const ', 'const ');
  return runInNewContext(`${executable}\n;({${names.join(',')}})`, {z, ...imports});
}

function normalize(value, key = '') {
  if (Array.isArray(value)) {
    const values = value.map(item => normalize(item));
    if (['required', 'enum', 'anyOf', 'oneOf', 'allOf', 'type'].includes(key)) {
      values.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
    }
    return values;
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, normalize(value[key], key)]));
  }
  return value;
}

// Zod numbers recursive JSON definitions in traversal order. Merge only
// definitions proven identical after replacing their own self-reference;
// references to other definitions are excluded from this equivalence rule.
function canonicalSchema(schema) {
  const value = structuredClone(schema);
  const aliases = new Map();
  const signatures = new Map();
  for (const name of Object.keys(value.$defs ?? {}).sort()) {
    const self = `#/$defs/${name}`;
    let independent = true;
    const signature = JSON.stringify(normalize(JSON.parse(JSON.stringify(value.$defs[name], (key, item) => {
      if (key !== '$ref') return item;
      if (item === self) return '#self';
      independent = false;
      return item;
    }))));
    if (!independent) continue;
    if (signatures.has(signature)) {
      aliases.set(self, `#/$defs/${signatures.get(signature)}`);
      delete value.$defs[name];
    } else {
      signatures.set(signature, name);
    }
  }
  return normalize(JSON.parse(JSON.stringify(value, (key, item) =>
    key === '$ref' && aliases.has(item) ? aliases.get(item) : item)));
}

function exportsAt(path) {
  return [...readFileSync(path, 'utf8').matchAll(/^export (?:const|type|function) (\w+)/gm)]
      .map(match => match[1]).sort();
}

const goPath = `${generated}/web/web-api.ts`;
const rustPath = `${reference}/packages/web/src/api/generated/web-api.ts`;
const go = schemas(goPath, schemas(`${generated}/protocol/contracts.ts`));
const rust = schemas(rustPath, schemas(`${reference}/packages/protocol/src/generated/contracts.ts`));
const actualExports = exportsAt(goPath);
const expectedExports = exportsAt(rustPath);
const exportMatch = isDeepStrictEqual(actualExports, expectedExports);
const differences = [];
for (const name of Object.keys(rust)) {
  if (!go[name]) {
    differences.push({name, missing: true});
    continue;
  }
  const left = canonicalSchema(z.toJSONSchema(go[name], {io: 'input', reused: 'inline'}));
  const right = canonicalSchema(z.toJSONSchema(rust[name], {io: 'input', reused: 'inline'}));
  if (!isDeepStrictEqual(left, right)) {
    differences.push({name, go: left, rust: right});
  }
}
const tableLine = path => readFileSync(path, 'utf8')
    .match(/^export const MAX_PAGE_MESSAGE_BYTES = .+$/m)?.[0];
const goLimit = tableLine(`${generated}/protocol/tables.ts`);
const rustLimit = tableLine(`${reference}/packages/protocol/src/generated/tables.ts`);
const tableMatch = goLimit !== undefined && goLimit === rustLimit;
const result = {
  tableMatch,
  compared: Object.keys(rust).length,
  exports: actualExports.length,
  exportMatch,
  missing: expectedExports.filter(name => !actualExports.includes(name)),
  extra: actualExports.filter(name => !expectedExports.includes(name)),
  differences,
};
writeFileSync(`${generated}/comparison.json`, JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify({...result, differences: differences.map(value => value.name)}));
if (!exportMatch || !tableMatch || differences.length) {
  process.exitCode = 1;
}
