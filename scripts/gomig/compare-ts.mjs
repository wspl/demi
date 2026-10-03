// Transitional G2 gate: run with bun from any directory. No services or network.
// Compares the complete generated surface; spelling, docs and private factoring
// are intentionally not compared. Fixture probes retain the former six checkers.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, mkdtempSync, rmSync } from 'node:fs';
import { resolve, relative, join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { runInNewContext } from 'node:vm';
import { spawnSync } from 'node:child_process';
import ts from 'typescript';
import { z } from 'zod';
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const reference = resolve(process.env.CONTRACTGEN_REFERENCE ?? '/Users/zan/Projects/demi-worktrees/gomig-ref/generated');
const generated = resolve(process.env.CONTRACTGEN_GENERATED ?? repository);
const json = path => JSON.parse(readFileSync(resolve(repository, path), 'utf8'));
const sourceAt = path => readFileSync(path, 'utf8');
// JSON Schema gives these arrays set semantics; all other array order matters.
function normalize(value, key = '') {
  if (Array.isArray(value)) {
    const result = Array.from(value, item => normalize(item));
    if (['required', 'enum', 'anyOf', 'oneOf', 'allOf', 'type'].includes(key)) {
      result.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
    }
    return result;
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(name => [name, normalize(value[name], name)]));
  }
  return value;
}
function exportsAt(path) {
  const file = ts.createSourceFile(path, sourceAt(path), ts.ScriptTarget.Latest, true);
  const names = [];
  for (const statement of file.statements) {
    if (!statement.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.ExportKeyword)) {
      continue;
    }
    if (ts.isVariableStatement(statement)) {
      for (const declaration of statement.declarationList.declarations) {
        names.push(declaration.name.getText(file));
      }
    }
    else if (statement.name) {
      names.push(statement.name.text);
    }
    else {
      throw new Error(`Unsupported generated export in ${path}: ${statement.getText(file)}`);
    }
  }
  return names.sort();
}
// Execute real Zod on both sides. Registry imports become their package names,
// so comparison observes import identity and order without loading Vue pages.
function load(path, imports = {}) {
  const source = sourceAt(path);
  const module = { exports: {} };
  const code = ts.transpileModule(source, { compilerOptions: {
      module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ESNext,
    } }).outputText;
  runInNewContext(code, { exports: module.exports, module, require(name) {
      if (name === 'zod') {
        return { z };
      }
      if (name === '@demicodes/protocol') {
        return imports;
      }
      if (name.startsWith('@demicodes/plugin-')) {
        return { default: name, __esModule: true };
      }
      throw new Error(`Unexpected generated import ${name} in ${path}`);
    } }, { filename: path });
  return module.exports;
}
function generatedFiles(root) {
  const visit = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    if (entry.name === 'node_modules') {
      return [];
    }
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      return visit(path);
    }
    return path.endsWith('.ts') && path.includes('/generated/') ? [relative(root, path)] : [];
  });
  return visit(join(root, 'packages')).sort();
}
// Collect all export differences so the G2 report does not stop at the first
// schema in a module. Assertions retain their full value/schema diagnostics.
function compareExport(check) {
  try {
    check();
  } catch (error) {
    failures.push(error);
    console.error(error.message);
  }
}

function compareModules(path, left, right) {
  compareExport(() => assert.deepEqual(
    exportsAt(join(generated, path)), exportsAt(join(reference, path)),
    `${path}: exported names`,
  ));
  for (const name of Object.keys(right)) {
    compareExport(() => {
      assert.ok(name in left, `${path}: missing ${name}`);
      if (right[name] instanceof z.ZodType) {
        assert.deepEqual(
          normalize(z.toJSONSchema(left[name], {io: 'input', reused: 'inline'})),
          normalize(z.toJSONSchema(right[name], {io: 'input', reused: 'inline'})),
          `${path}: ${name} validation differs`,
        );
      } else if (typeof right[name] === 'function') {
        assert.ok(['previewMediaType', 'showsInPlace'].includes(name), `Unprobed function ${path}: ${name}`);
      } else {
        assert.deepEqual(
          JSON.parse(JSON.stringify(left[name])), JSON.parse(JSON.stringify(right[name])),
          `${path}: ${name} value differs`,
        );
      }
    });
  }
}
// Type aliases must still infer the corresponding validated value. Comparing
// their checker types in both directions also catches a widened handwritten alias.
function compareTypes(paths) {
  const options = { strict: true, skipLibCheck: true, noEmit: true,
    target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler };
  const host = ts.createCompilerHost(options);
  host.resolveModuleNames = (names, containing) => names.map(name => {
    if (name === '@demicodes/protocol') {
      const root = containing.startsWith(reference + '/') ? reference : generated;
      return { resolvedFileName: join(root, 'packages/protocol/src/generated/contracts.ts'), extension: ts.Extension.Ts };
    }
    return ts.resolveModuleName(name, name.startsWith('.') ? containing : join(repository, 'comparison.ts'), options, host).resolvedModule;
  });
  const filenames = paths.filter(path => !path.endsWith('/pages.ts')).flatMap(path => [join(generated, path), join(reference, path)]);
  const program = ts.createProgram(filenames, options, host);
  const diagnostics = program.getSemanticDiagnostics();
  assert.equal(diagnostics.length, 0, ts.formatDiagnosticsWithColorAndContext(diagnostics, {
    getCurrentDirectory: () => repository,
    getCanonicalFileName: name => name,
    getNewLine: () => '\n',
  }));
  const checker = program.getTypeChecker();
  for (const path of paths.filter(path => !path.endsWith('/pages.ts'))) {
    const symbols = root => {
      const file = program.getSourceFile(join(root, path));
      return new Map(checker.getExportsOfModule(checker.getSymbolAtLocation(file))
        .filter(symbol => symbol.flags & ts.SymbolFlags.TypeAlias)
        .map(symbol => [symbol.name, checker.getDeclaredTypeOfSymbol(symbol)]));
    };
    const left = symbols(generated);
    const right = symbols(reference);
    for (const [name, expected] of right) {
      compareExport(() => {
        const actual = left.get(name);
        assert.ok(actual && checker.isTypeAssignableTo(actual, expected) && checker.isTypeAssignableTo(expected, actual), `${path}: type ${name} differs`);
        assert.equal(actual.flags & ts.TypeFlags.Any, expected.flags & ts.TypeFlags.Any, `${path}: type ${name} became any`);
      });
    }
  }
}
function protocolAt(root) {
  return { ...load(join(root, 'packages/protocol/src/generated/contracts.ts')),
    ...load(join(root, 'packages/protocol/src/generated/tables.ts')) };
}
// Apply the Rust contract corpus's recorded field removals and replacements.
// The protocol test's private helper cannot be imported without running its suite.
function mutateFixture(value, mutation) {
  const parent = pointer => {
    const parts = pointer.slice(1).split('/').map(part => part.replaceAll('~1', '/').replaceAll('~0', '~'));
    const key = parts.pop();
    return [parts.reduce((target, part) => target[part], value), key];
  };
  for (const pointer of mutation.remove ?? []) {
    const [target, key] = parent(pointer);
    delete target[key];
  }
  for (const [pointer, replacement] of Object.entries(mutation.set ?? {})) {
    const [target, key] = parent(pointer);
    target[key] = replacement;
  }
}
function blockFixtures(go, rust) {
  const fixtures = json('internal/core/testdata/blocks.json');
  const cases = json('internal/core/testdata/blocks-mutations.json').refused;
  for (const [label, schema] of [['go', go.blockSchema], ['rust', rust.blockSchema]]) {
    let accepted = 0;
    for (const fixture of fixtures) {
      const parsed = schema.safeParse(fixture);
      if (!parsed.success) {
        throw new Error(`${label} rejects ${fixture.id}: ${parsed.error}`);
      }
      accepted++;
    }
    let refused = 0;
    for (const tc of cases) {
      const value = structuredClone(fixtures.find(fixture => fixture.id === tc.fixture));
      mutateFixture(value, tc);
      const result = schema.safeParse(value);
      if (result.success === tc.webAppRefuses) {
        throw new Error(`${label}: unexpected result for ${tc.why}`);
      }
      if (!result.success) {
        refused++;
      }
    }
    const context = structuredClone(fixtures.find(value => value.type === 'context'));
    context.source = '😀'.repeat(64);
    if (!schema.safeParse(context).success) {
      throw new Error(`${label}: Unicode scalar bound rejects 64 astral scalars`);
    }
    context.source += '😀';
    if (schema.safeParse(context).success) {
      throw new Error(`${label}: Unicode scalar bound accepts 65 astral scalars`);
    }
    console.log(`${label}: fixtures accepted=${accepted}; web mutations refused=${refused}; tolerated=${cases.length - refused}; Unicode 64/65 boundary passed`);
  }
}
function frameFixtures(go, rust) {
  const fixtures = json('internal/framewire/testdata/client-frames.json');
  const server = json('internal/framewire/testdata/server-frames.json');
  const table = json('internal/framewire/testdata/client-frames-mutations.json');
  for (const [label, schemas] of [['go', go], ['rust', rust]]) {
    for (const [frames, schema] of [[fixtures, schemas.clientFrameSchema], [server, schemas.serverFrameSchema]]) {
      for (const frame of frames) {
        if (!schema.safeParse(frame).success) {
          throw new Error(`${label} refuses ${frame.type}`);
        }
      }
    }
    for (const category of ['accepted', 'refused']) {
      for (const tc of table[category]) {
        const value = structuredClone(tc.value ?? fixtures.find(frame => frame.type === tc.fixture));
        mutateFixture(value, tc);
        if (schemas.clientFrameSchema.safeParse(value).success === (category === 'refused' && tc.webAppRefuses)) {
          throw new Error(`${label}: ${tc.why}`);
        }
      }
    }
    for (const variant of schemas.clientFrameSchema.options) {
      if (z.toJSONSchema(variant).additionalProperties !== false) {
        throw new Error(`${label}: client frame not strict`);
      }
    }
    for (const frame of server) {
      if (!schemas.serverFrameSchema.safeParse({ ...frame, future: true }).success) {
        throw new Error(`${label}: server not tolerant`);
      }
    }
    console.log(`${label}: ${fixtures.length} client / ${server.length} server frames; ${table.refused.length} refused and ${table.accepted.length} accepted mutation expectations match`);
  }
}
function viewerFixtures(go, rust) {
  const fixtures = json('internal/cmdpkg/browser/browserop/testdata/live-viewer.json');
  for (const [label, schema] of [['Go', go.liveViewerMessageSchema], ['Rust', rust.liveViewerMessageSchema]]) {
    for (const value of fixtures.accepted) {
      assert.equal(schema.safeParse(value).success, true, `${label} refused ${JSON.stringify(value)}`);
    }
    for (const value of fixtures.refused) {
      assert.equal(schema.safeParse(value).success, false, `${label} accepted ${JSON.stringify(value)}`);
    }
  }
}
function fileFixtures(goTables, rustTables) {
  const fileCases = json('internal/core/testdata/file-types.json');
  for (const name of ['previewMediaType', 'showsInPlace']) {
    for (const [input, want] of fileCases[name]) {
      assert.equal(goTables[name](input), want);
      assert.equal(rustTables[name](input), want);
    }
  }
}
function fixtureGeneration(directory, patterns) {
  const result = spawnSync('go', ['run', './tools/contractgen', '-ts', '-ts-dir', directory, ...patterns], {
    cwd: repository, env: { ...process.env, CGO_ENABLED: '0', GOFLAGS: '-mod=readonly' }, encoding: 'utf8',
  });
  if (result.error) {
    throw result.error;
  }
  assert.equal(result.status, 0, result.stderr);
}
function generatorFixtures(directory, rustTables) {
  fixtureGeneration(directory, ['./tools/contractgen/testdata/blocks', './tools/contractgen/testdata/features']);
  const go = load(`${directory}/protocol/contracts.ts`);
  const rust = protocolAt(reference);
  for (const [name, schema] of Object.entries(go)) {
    assert.ok(name in rust, `Unexpected generator fixture export ${name}`);
    assert.deepEqual(normalize(z.toJSONSchema(schema, { io: 'input', reused: 'inline' })), normalize(z.toJSONSchema(rust[name], { io: 'input', reused: 'inline' })), `Generator fixture ${name} differs from Rust`);
  }
  blockFixtures(go, rust);
  const web = load(`${directory}/web/web-api.ts`, go);
  const plugin = load(`${directory}/plugin-test/plugin.ts`);
  const request = { id: 'id_ok', body: { type: 'text', text: 'hello' }, data: 'Zg==', label: null, extra: null };
  if (!web.requestSchema.safeParse(request).success) {
    throw new Error('valid request refused');
  }
  for (const invalid of [{ ...request, unknown: true }, { ...request, data: 'Zh==' }, { ...request, label: undefined }]) {
    if (web.requestSchema.safeParse(invalid).success) {
      throw new Error('invalid request accepted');
    }
  }
  const tree = { name: 'id_root', children: [{ name: 'id_child', children: [], future: true }] };
  if (!plugin.treeSchema.safeParse(tree).success) {
    throw new Error('recursive tolerant tree refused');
  }
  if (plugin.treeSchema.safeParse({ ...tree, parent: null }).success) {
    throw new Error('optional null accepted');
  }
  console.log('Cross-module imports, strict requests, canonical base64 and recursive tolerant objects passed');
  fixtureGeneration(directory, ['./tools/contractgen/testdata/tables', './tools/contractgen/testdata/text', './tools/contractgen/testdata/unions']);
  const tables = load(`${directory}/protocol/tables.ts`);
  const live = load(join(reference, 'packages/plugin-browser/src/generated/plugin.ts'), protocolAt(reference));
  const expected = normalize({
    preview: rustTables.PREVIEW_TYPES,
    models: [
      ...rustTables.ATTACHMENT_FILE_EXTENSIONS.map(extension => ({ extension, kind: 'attachment' })),
      ...rustTables.VIDEO_FILE_EXTENSIONS.map(extension => ({ extension, kind: 'video' })),
    ],
    frames: ['CONTROL_FRAME', 'VIDEO_FRAME', 'FILE_FRAME', 'MAX_FRAME_BYTES', 'FILE_CHUNK_BYTES', 'HEARTBEAT_MS', 'STALL_MS']
      .map(name => ({ name, value: live['LIVE_' + name] })),
  });
  assert.deepEqual(JSON.parse(JSON.stringify(tables.PREVIEW_TYPES)), expected.preview);
  assert.deepEqual(JSON.parse(JSON.stringify(tables.MODEL_FILE_TYPES)), expected.models);
  assert.deepEqual(JSON.parse(JSON.stringify(tables.LIVE_VIEW_FRAME_CONSTANTS)), expected.frames);
  let paths = 0;
  for (const entry of expected.preview) {
    for (const extension of entry.extensions) {
      for (const path of [`a.${extension}`, `/dir/a.${extension.toUpperCase()}`, `C:\\dir\\a.${extension}`, `a.b.${extension}`, `.${extension}`, `/a.${extension}/file`, `a.${extension}\\file`]) {
        const want = path.endsWith('/file') || path.endsWith('\\file') || path === `.${extension}` ? null : entry.mediaType;
        assert.equal(tables.previewMediaType(path), want, path);
        assert.equal(tables.previewMediaType(path), rustTables.previewMediaType(path), path);
        paths++;
      }
    }
    assert.equal(tables.showsInPlace(entry.mediaType), entry.inPlace);
    assert.equal(tables.showsInPlace(entry.mediaType), rustTables.showsInPlace(entry.mediaType));
  }
  for (const extension of ['txt', 'json', 'ts', 'html', 'csv', 'zip', 'gz', 'exe', 'unknown', 'pñg']) {
    const path = `test.${extension}`;
    assert.equal(tables.previewMediaType(path), null);
    assert.equal(tables.previewMediaType(path), rustTables.previewMediaType(path));
    paths++;
  }
  assert.equal(tables.showsInPlace('IMAGE/PNG'), false);
  assert.equal(tables.showsInPlace('application/unknown'), false);
  {
    for (const [kind, name] of [['attachment', 'ATTACHMENT_FILE_EXTENSIONS'], ['video', 'VIDEO_FILE_EXTENSIONS']]) {
      assert.equal(JSON.stringify(tables[name]), JSON.stringify(rustTables[name]));
      assert.equal(JSON.stringify(tables[name]), JSON.stringify(expected.models.filter(entry => entry.kind === kind).map(entry => entry.extension)));
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
  assert.equal(schemas.receivedSchema.parse({ email: 'ANA@example.test', future: true }).email, 'ANA@example.test');
  assert.equal(schemas.receivedSchema.safeParse({ email: ' ana@example.test ' }).success, false);
  assert.equal(schemas.nameSchema.parse('\ufeff  New name\t\u2003'), 'New name');
  assert.equal(schemas.nameSchema.parse('\u0085name'), '\u0085name');
  assert.equal(schemas.nameSchema.parse(` ${'😀'.repeat(8)} `), '😀'.repeat(8));
  for (const invalid of ['', ' \ufeff\t\u2003', '😀'.repeat(9)]) {
    assert.equal(schemas.nameSchema.safeParse(invalid).success, false);
  }
  // Keep the Rust emitter's z.url form: validation preserves URL input text;
  // the Go boundary stores the canonical serialization.
  for (const input of ['https://api.kimi.com/coding/v1', 'http://127.0.0.1:8080', 'https://example.com:443/v1', 'https://bücher.example/v1']) {
    assert.equal(schemas.endpointUrlSchema.parse(input), input);
  }
  for (const input of ['http:example.com', '', 'api.openai.com/v1', 'ftp://example.test/', 'file:///etc', 'https://', 'https://example.com:65536/']) {
    assert.equal(schemas.endpointUrlSchema.safeParse(input).success, false, input);
  }
  console.log(`Tables: ${paths} paths, model-readable extensions and live frame constants passed; email/trimmed/http-url Zod passed; Rust lookups agree`);
  const replies = load(`${directory}/plugin-unions/plugin.ts`);
  for (const schema of [replies.ensureReplySchema, replies.statusReplySchema]) {
    assert.equal(schema.safeParse({ ok: false, code: 'install_failed', message: 'digest differs' }).success, true);
    assert.equal(schema.safeParse({ ok: 'false', code: 'install_failed', message: 'digest differs' }).success, false);
    assert.equal(schema.safeParse({ code: 'install_failed', message: 'digest differs' }).success, false);
  }
  assert.equal(replies.statusReplySchema.safeParse({ ok: true, platform: 'darwin-arm64', installed: [] }).success, true);
  assert.equal(replies.ensureReplySchema.safeParse({ ok: true, version: '2.1.3', path: '/opt/claude' }).success, true);
  assert.equal('failureSchema' in replies, false);
  assert.equal('bytesSchema' in replies, false);
  assert.equal(replies.emptySchema.safeParse({}).success, true);
  assert.equal(replies.emptySchema.safeParse({ bytes: '/wA=' }).success, true);
  assert.equal(replies.emptySchema.safeParse({ bytes: '/wA' }).success, false);
  console.log('Boolean reply unions, private schemas, and byte-slice base64 passed');
}
const failures = [];
if (!process.argv.includes('--fixtures-only')) {
  try {
    const paths = generatedFiles(reference);
    assert.deepEqual(generatedFiles(generated), paths, 'Generated TypeScript file set differs');
    const go = protocolAt(generated);
    const rust = protocolAt(reference);
    for (const path of paths) {
      try {
        compareModules(path, load(join(generated, path), go), load(join(reference, path), rust));
        console.log(`Compared ${path}`);
      }
      catch (error) {
        failures.push(error);
        console.error(error.message);
      }
    }
    compareTypes(paths);
    blockFixtures(go, rust);
    frameFixtures(go, rust);
    viewerFixtures(load(join(generated, 'packages/plugin-browser/src/generated/plugin.ts'), go), load(join(reference, 'packages/plugin-browser/src/generated/plugin.ts'), rust));
    fileFixtures(go, rust);
  }
  catch (error) {
    failures.push(error);
    console.error(error.stack);
  }
}
const temporary = mkdtempSync(join(tmpdir(), 'demi-compare-ts-'));
try {
  generatorFixtures(temporary, protocolAt(reference));
}
catch (error) {
  failures.push(error);
  console.error(error.stack);
}
finally {
  rmSync(temporary, { recursive: true, force: true });
}
assert.equal(failures.length, 0, `${failures.length} TypeScript comparison failures`);
console.log(process.argv.includes('--fixtures-only')
  ? 'Generator TypeScript fixture probes passed'
  : 'G2: all generated TypeScript files, exports, values, types and fixture probes agree');
