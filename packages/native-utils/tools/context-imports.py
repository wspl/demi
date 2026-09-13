"""Route selected upstream standard-library imports through invocation context."""

from pathlib import Path
import argparse
import re


def flatten_use(text):
    tokens = re.findall(r'::|[A-Za-z_][A-Za-z_0-9]*|[{},*]', text)
    cursor = 0
    paths = []

    def tree(prefix):
        nonlocal cursor
        if tokens[cursor] == '{':
            cursor += 1
            while tokens[cursor] != '}':
                tree(prefix)
                if tokens[cursor] == ',':
                    cursor += 1
            cursor += 1
            return
        name = tokens[cursor]
        cursor += 1
        path = prefix + [name]
        if cursor < len(tokens) and tokens[cursor] == '::':
            cursor += 1
            tree(path)
            return
        alias = ''
        if cursor < len(tokens) and tokens[cursor] == 'as':
            alias = ' as ' + tokens[cursor + 1]
            cursor += 2
        if path[-1] == 'self':
            path.pop()
        paths.append('::'.join(path) + alias)

    tree([])
    if cursor != len(tokens):
        raise ValueError(f'Unparsed use statement: {text}')
    return paths


root = Path(__file__).resolve().parents[3] / 'vendor'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('packages', nargs='+', help='Newly imported upstream package names')
for name in parser.parse_args().packages:
    package = root / name
    if not package.is_dir():
        raise ValueError(f'Unknown upstream package: {name}')
    for source in (package / 'src').rglob('*.rs'):
        if source.name == 'context.rs' or 'context' in source.parts:
            continue
        namespace = 'crate' if package.name == 'uucore' else 'uucore'
        text = source.read_text()
        # Expand grouped std imports without changing surrounding cfg attributes.
        def grouped(match):
            paths = flatten_use(match.group(1))
            return 'use {' + ', '.join(paths) + '};'
        text = re.sub(r'\buse\s+(std::\{[^;]+\});', grouped, text)
        for module in ('io', 'fs', 'env', 'process', 'thread'):
            text = re.sub(r'\bstd::' + module + r'\b', namespace + '::context::' + module, text)
        for macro in ('print', 'println', 'eprint', 'eprintln'):
            text = re.sub(r'(?<![\w:])' + macro + r'!', namespace + '::context_' + macro + '!', text)
        source.write_text(text)
