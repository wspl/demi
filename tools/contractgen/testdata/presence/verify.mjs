import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';
import {z} from 'zod';

const source = readFileSync(`${process.argv[2]}/plugin-presence/plugin.ts`, 'utf8');
const code = stripTypeScriptTypes(source)
  .replace(/^import .*$/gm, '')
  .replaceAll('export const ', 'const ');
const {patchSchema, installEnvelopeSchema, defaultsSchema} = runInNewContext(
  `${code}\n;({patchSchema, installEnvelopeSchema, defaultsSchema})`, {z});
for (const input of [{}, {option: null, double: null, items: null},
  {option: '', double: '', items: []}, {option: 'ok', double: 'yes', items: ['a']}]) {
  assert.equal(patchSchema.safeParse(input).success, true, JSON.stringify(input));
}
for (const input of [{option: 'longer'}, {double: 'longer'}, {items: 1}]) {
  assert.equal(patchSchema.safeParse(input).success, false, JSON.stringify(input));
}
const install = {package: 'p', name: 'n', version: 'v', phase: 'download', done: 0, total: 1};
for (const [field, maximum] of [['package', 200], ['name', 100], ['version', 100]]) {
  assert.equal(installEnvelopeSchema.safeParse({install: {...install, [field]: 'a'.repeat(maximum)}}).success, true);
  assert.equal(installEnvelopeSchema.safeParse({install: {...install, [field]: 'a'.repeat(maximum + 1)}}).success, false);
}
console.log('nullable optional states and install bounds: PASS');

for (const input of [{}, {items: [], labels: {}, enabled: false, count: 0, text: ''},
  {items: ['a'], labels: {a: 'b'}, enabled: true, count: 2, text: 'yes'}]) {
  assert.equal(defaultsSchema.safeParse(input).success, true, JSON.stringify(input));
}
assert.deepEqual(Object.keys(defaultsSchema.parse({})), []);
for (const input of ['items', 'labels', 'enabled', 'count', 'text'].map(key => ({[key]: null}))) {
  assert.equal(defaultsSchema.safeParse(input).success, false, JSON.stringify(input));
}
console.log('default fields: PASS');
