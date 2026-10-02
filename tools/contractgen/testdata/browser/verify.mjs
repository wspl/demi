// Generated Zod at the browser boundary; no services or timers.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';
import {z} from 'zod';

const source = readFileSync(`${process.argv[2]}/plugin-browser/plugin.ts`, 'utf8');
const names = [...source.matchAll(/export const (\w+)/g)].map(match => match[1]);
const code = stripTypeScriptTypes(source)
  .replace(/^import .*$/gm, '')
  .replaceAll('export const ', 'const ');
const schemas = runInNewContext(`${code}\n;({${names.join(',')}})`, {z});
for (const [name, accepted, refused] of [
  ['nodeValue', ['hello', 0, -1.5], [true, null, [], {}, Infinity]],
  ['scalar', [-1, -11, 1.5, true, {value: 'x'}], ['hello', null, [], {}]],
  ['wheel', [{deltaX: -10000, deltaY: 10000}], [{deltaX: -10000.1, deltaY: 0}, {deltaX: 0, deltaY: 10000.1}]],
  ['dialogInspectResult', [{dialog: null}, {dialog: {type: 'prompt', message: 'x'}}], [{}, {dialog: {}}, {dialog: false}]],
  ['optional', [{before: 'b', after: 'a'}, {before: 'b', type: 'alert', message: 'm', after: 'a'}], [{}, {before: 'b', type: 1, after: 'a'}]],
]) {
  const schema = schemas[`${name}Schema`];
  for (const value of accepted) assert.equal(schema.safeParse(value).success, true, `${name}: ${JSON.stringify(value)}`);
  for (const value of refused) assert.equal(schema.safeParse(value).success, false, `${name}: ${JSON.stringify(value)}`);
}
console.log('browser Zod: PASS');
