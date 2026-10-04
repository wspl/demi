// Generated object schema at the browser boundary; no services or timers.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';
import {z} from 'zod';

const source = readFileSync(`${process.argv[2]}/protocol/contracts.ts`, 'utf8');
const code = stripTypeScriptTypes(source)
  .replace(/^import .*$/gm, '')
  .replaceAll('export const ', 'const ');
const schema = runInNewContext(`${code}\n;inputSchema`, {z});
for (const data of [{}, {z: null, a: [true, 42, {nested: 'text'}]}]) {
  assert.equal(schema.safeParse({data}).success, true);
}
for (const data of [[], 'text', null, true, 42]) {
  assert.equal(schema.safeParse({data}).success, false);
}
assert.equal(schema.safeParse({}).success, false);
console.log('ordered object Zod: PASS');
