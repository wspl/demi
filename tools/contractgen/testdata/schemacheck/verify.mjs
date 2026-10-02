// Go-only checks must not narrow generated Zod schemas.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';
import {z} from 'zod';

const source = readFileSync(`${process.argv[2]}/plugin-schemacheck/plugin.ts`, 'utf8');
const code = stripTypeScriptTypes(source)
  .replace(/^import .*$/gm, '')
  .replaceAll('export const ', 'const ');
const {checkedTextSchema, envelopeSchema, codecTextSchema, codecEnvelopeSchema} = runInNewContext(
  `${code}\n;({checkedTextSchema, envelopeSchema, codecTextSchema, codecEnvelopeSchema})`, {z});
for (const value of ['allowed', 'reserved']) {
  assert.equal(checkedTextSchema.safeParse(value).success, true);
  assert.equal(envelopeSchema.safeParse({value}).success, true);
}
assert.equal(checkedTextSchema.safeParse('').success, false);
assert.equal(checkedTextSchema.safeParse(1).success, false);
assert.equal(envelopeSchema.safeParse({value: 1}).success, false);
assert.equal(envelopeSchema.safeParse({}).success, false);
console.log('Go-only checks omitted from Zod: PASS');

assert.equal(codecTextSchema.safeParse('a@b').success, false);
assert.equal(codecTextSchema.safeParse('x').success, false);
assert.equal(codecEnvelopeSchema.safeParse({value: 'x'}).success, false);
assert.equal(codecTextSchema.safeParse('a@b.com').success, true);
assert.equal(codecEnvelopeSchema.safeParse({value: 'a@b.com'}).success, true);
assert.equal(codecTextSchema.safeParse('A@b.com').success, false);
assert.equal(codecTextSchema.safeParse('a'.repeat(25) + '@b.com').success, false);
assert.equal(codecTextSchema.safeParse('reserved').success, false);
