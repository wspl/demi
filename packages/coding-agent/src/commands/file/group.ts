import { pathArg, runtimeModule, type CommandGroup } from '@demicodes/shell'
import { z } from 'zod'
import createModule from './create.command' with { type: 'text' }
import editModule from './edit.command' with { type: 'text' }
import patchModule from './patch.command' with { type: 'text' }
import readModule from './read.command' with { type: 'text' }

/** The `demi file` group: every leaf is a `runtime` module over `ctx.fs`. */
export function createFileGroup(): CommandGroup {
  return {
    name: 'file',
    summary: 'Read, create, edit, and patch workspace files (text, images, and video).',
    subcommands: [
      {
        name: 'read',
        kind: 'runtime',
        module: runtimeModule(readModule),
        summary:
          'Read a file. Text files print as text; image and video files are shown to you as viewable media. Output is the raw file bytes, so it also pipes cleanly into other commands (e.g. ffmpeg).',
        successOutput:
          'writes the raw file bytes to stdout; an image or video result is presented to you as viewable media',
        failureOutput: 'writes the reason to stderr and exits non-zero if the path is missing or unreadable',
        input: {
          path: pathArg(z.string().describe('File path to read')),
        },
        positionals: ['path'],
      },
      {
        name: 'create',
        kind: 'runtime',
        module: runtimeModule(createModule),
        summary: 'Create a new file. Fails if the file exists.',
        successOutput: 'writes "Created <path>" to stdout',
        failureOutput: 'writes the reason to stderr and exits non-zero without overwriting existing files',
        input: {
          path: pathArg(z.string().describe('Target file path')),
          content: z.string().describe('File content, passed via stdin/heredoc'),
        },
        positionals: ['path'],
        stdinField: 'content',
      },
      {
        name: 'edit',
        kind: 'runtime',
        module: runtimeModule(editModule),
        summary: 'Replace exact text in an existing file.',
        successOutput: 'writes "Edited <path>" to stdout',
        failureOutput: 'writes no-match, ambiguous-match, or write errors to stderr and exits non-zero without partial writes',
        input: {
          path: pathArg(z.string().describe('Target file path')),
          old: z.string().describe('Exact text to replace'),
          new: z.string().describe('Replacement text'),
          occurrence: z.number().int().positive().optional().describe('1-based occurrence to replace'),
          context: z.number().int().positive().optional().describe('Line number used to choose nearest occurrence'),
        },
        positionals: ['path'],
      },
      {
        name: 'patch',
        kind: 'runtime',
        module: runtimeModule(patchModule),
        summary: 'Apply a unified diff patch to one or more files.',
        successOutput: 'writes "Patched <n> file(s)" to stdout',
        failureOutput: 'writes parse, validation, or write errors to stderr and exits non-zero after rolling back partial writes when possible',
        input: {
          patch: z.string().describe('Unified diff content, passed via stdin/heredoc'),
        },
        stdinField: 'patch',
      },
    ],
  }
}
