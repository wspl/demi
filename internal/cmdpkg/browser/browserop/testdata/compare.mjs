// Compare browser-owned validators, independently of generator source spelling.
import assert from 'node:assert/strict';
import {readFileSync, readdirSync, writeFileSync} from 'node:fs';
import {createRequire, stripTypeScriptTypes} from 'node:module';
import {join, resolve} from 'node:path';
import {runInNewContext} from 'node:vm';

const [generatedPath, referencePath, dependencyPackage = resolve('package.json')] = process.argv.slice(2);
const {z} = createRequire(resolve(dependencyPackage))('zod');

// Evaluate generated schemas with the same installed Zod on both sides.
function schemas(path) {
  const source = readFileSync(path, 'utf8');
  const names = [...source.matchAll(/export const (\w+Schema) = /g)].map(match => match[1]);
  const code = stripTypeScriptTypes(source)
    .replace(/^import .*$/gm, '')
    .replaceAll('export const ', 'const ');
  return runInNewContext(`${code}\n;({${names.join(',')}})`, {z});
}

// These JSON Schema arrays are sets; their order has no validation meaning.
function canonical(value, key = '') {
  if (Array.isArray(value)) {
    const result = value.map(item => canonical(item));
    if (['required', 'enum', 'anyOf', 'oneOf', 'allOf', 'type'].includes(key)) {
      result.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
    }
    return result;
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(name => [name, canonical(value[name], name)]));
  }
  return value;
}

// Select ownership from the Rust crate, never from the generated export list:
// otherwise a missing Go root would silently disappear from this comparison.
const rustRoot = new URL('../../../../../crates/command-package-browser-protocol/src/', import.meta.url);
function rustTypes(directory) {
  return readdirSync(directory, {withFileTypes: true}).flatMap(entry => {
    const path = new URL(entry.name + (entry.isDirectory() ? '/' : ''), directory);
    if (entry.isDirectory()) return rustTypes(path);
    if (!entry.name.endsWith('.rs')) return [];
    return [...readFileSync(path, 'utf8').matchAll(/pub (?:struct|enum) (\w+)/g)].map(match => match[1]);
  });
}
const owned = new Set([...rustTypes(rustRoot), 'TabId', 'NodeRef']);
const reference = schemas(referencePath);
const generated = schemas(generatedPath);
const expectedNames = Object.keys(reference).filter(name => owned.has(name[0].toUpperCase() + name.slice(1, -6))).sort();
assert.deepEqual(Object.keys(generated).sort(), expectedNames, 'browser-owned schema exports');

for (const name of expectedNames) {
  const goSchema = canonical(z.toJSONSchema(generated[name], {io: 'input', reused: 'inline'}));
  const rustSchema = canonical(z.toJSONSchema(reference[name], {io: 'input', reused: 'inline'}));
  assert.deepEqual(goSchema, rustSchema, `${name}: input validation differs`);
}

// Also run the viewer fixtures through the actual schemas: this exercises
// strict object handling, discriminants, required nullables and negative bounds.
const fixtures = JSON.parse(readFileSync(new URL('live-viewer.json', import.meta.url), 'utf8'));
for (const [label, schema] of [['Go', generated.liveViewerMessageSchema], ['Rust', reference.liveViewerMessageSchema]]) {
  for (const value of fixtures.accepted) {
    assert.equal(schema.safeParse(value).success, true, `${label} refused ${JSON.stringify(value)}`);
  }
  for (const value of fixtures.refused) {
    assert.equal(schema.safeParse(value).success, false, `${label} accepted ${JSON.stringify(value)}`);
  }
}
console.log(`Browser TypeScript: ${expectedNames.length} exported schemas equal; ${fixtures.accepted.length} accepted and ${fixtures.refused.length} refused viewer fixtures agree.`);
writeFileSync(join(resolve(generatedPath, '..'), 'comparison.json'), JSON.stringify({schemas: expectedNames, differences: []}, null, 2) + '\n');
