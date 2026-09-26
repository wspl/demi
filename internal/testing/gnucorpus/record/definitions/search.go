package definitions

import "github.com/wspl/demi/internal/testing/gnucorpus"

// grepFixture is shared between grep and rg: both search the same kind of
// text, so the same input tree exercises both.
func grepFixture() gnucorpus.Tree {
	return files(map[string]string{
		"a.txt":       "apple\nBanana\ncherry apple\n",
		"b.txt":       "date\napple pie\n",
		"nomatch.txt": "xyz\n",
		"sub/c.txt":   "nested apple\n",
	})
}

func grepCases() []gnucorpus.Case {
	tree := grepFixture()
	return []gnucorpus.Case{
		core("plain-match", []string{"apple", "a.txt"}, "", tree),
		core("no-match-exit-1", []string{"zzz", "a.txt"}, "", tree),
		core("ignore-case", []string{"-i", "banana", "a.txt"}, "", tree),
		core("line-numbers", []string{"-n", "apple", "a.txt"}, "", tree),
		core("invert-match", []string{"-v", "apple", "a.txt"}, "", tree),
		core("count", []string{"-c", "apple", "a.txt"}, "", tree),
		core("files-with-matches", []string{"-l", "apple", "a.txt", "b.txt", "nomatch.txt"}, "", tree),
		core("files-without-matches", []string{"-L", "apple", "a.txt", "b.txt", "nomatch.txt"}, "", tree),
		core("multiple-files-prefix", []string{"apple", "a.txt", "b.txt"}, "", tree),
		core("stdin", []string{"apple"}, "apple\nbanana\n", gnucorpus.Tree{}),
		core("recursive", []string{"-r", "apple", "."}, "", tree),
		core("extended-regex", []string{"-E", "^(apple|date)$", "a.txt"}, "", tree),
		core("fixed-strings", []string{"-F", "apple", "a.txt"}, "", tree),
		core("missing-file-error", []string{"apple", "missing.txt"}, "", tree),
		extended("word-match", []string{"-w", "apple", "a.txt"}, "", tree),
		extended("line-match", []string{"-x", "apple", "a.txt"}, "", tree),
		extended("only-matching", []string{"-o", "apple", "a.txt"}, "", tree),
		extended("context", []string{"-C", "1", "apple", "a.txt"}, "", tree),
		extended("before-context", []string{"-B", "1", "apple", "a.txt"}, "", tree),
		extended("after-context", []string{"-A", "1", "apple", "a.txt"}, "", tree),
	}
}

func rgCases() []gnucorpus.Case {
	tree := grepFixture()
	return []gnucorpus.Case{
		core("plain-match", []string{"apple", "a.txt"}, "", tree),
		core("no-match-exit-1", []string{"zzz", "a.txt"}, "", tree),
		core("ignore-case", []string{"-i", "banana", "a.txt"}, "", tree),
		core("line-numbers", []string{"-n", "apple", "a.txt"}, "", tree),
		core("invert-match", []string{"-v", "apple", "a.txt"}, "", tree),
		core("count", []string{"-c", "apple", "a.txt"}, "", tree),
		// --sort path pins the order across files; without it ripgrep may
		// search multiple files in parallel and report them out of order.
		core("files-with-matches", []string{"--sort", "path", "-l", "apple", "a.txt", "b.txt", "nomatch.txt"}, "", tree),
		core("recursive-by-default", []string{"--sort", "path", "apple", "."}, "", tree),
		core("stdin", []string{"apple"}, "apple\nbanana\n", gnucorpus.Tree{}),
		core("fixed-strings", []string{"-F", "apple", "a.txt"}, "", tree),
		core("missing-file-error", []string{"apple", "missing.txt"}, "", tree),
		extended("word-match", []string{"-w", "apple", "a.txt"}, "", tree),
		extended("line-match", []string{"-x", "apple", "a.txt"}, "", tree),
		extended("only-matching", []string{"-o", "apple", "a.txt"}, "", tree),
		extended("context", []string{"-C", "1", "apple", "a.txt"}, "", tree),
		extended("glob-restrict", []string{"--sort", "path", "-g", "*.txt", "--", "apple", "."}, "", tree),
	}
}

func findFixture() gnucorpus.Tree {
	return gnucorpus.Tree{
		Files: map[string][]byte{
			"a.txt":        []byte("a\n"),
			"sub/b.txt":    []byte("b\n"),
			"sub/deep/c.c": []byte("c\n"),
		},
		Dirs: []string{"empty-dir"},
	}
}

func findCases() []gnucorpus.Case {
	tree := findFixture()
	return []gnucorpus.Case{
		core("list-all", []string{"."}, "", tree),
		core("name-glob", []string{".", "-name", "*.txt"}, "", tree),
		core("iname-case-insensitive", []string{".", "-iname", "A.TXT"}, "", tree),
		core("type-file", []string{".", "-type", "f"}, "", tree),
		core("type-dir", []string{".", "-type", "d"}, "", tree),
		core("maxdepth", []string{".", "-maxdepth", "1"}, "", tree),
		core("mindepth", []string{".", "-mindepth", "2"}, "", tree),
		core("empty-directories", []string{".", "-type", "d", "-empty"}, "", tree),
		core("print0-then-count", []string{".", "-name", "*.txt", "-print0"}, "", tree),
		core("nonexistent-root", []string{"missing-dir"}, "", tree),
		extended("exec-echo-per-file", []string{".", "-name", "*.txt", "-exec", "echo", "found", "{}", ";"}, "", tree),
		extended("exec-plus-batches", []string{".", "-name", "*.txt", "-exec", "cat", "{}", "+"}, "", tree),
		extended("not-name", []string{".", "-not", "-name", "*.txt"}, "", tree),
		extended("or-name", []string{".", "(", "-name", "*.txt", "-o", "-name", "*.c", ")"}, "", tree),
	}
}

func xargsFixture() gnucorpus.Tree {
	return files(map[string]string{"a.txt": "a\n", "b.txt": "b\n"})
}

func xargsCases() []gnucorpus.Case {
	tree := xargsFixture()
	return []gnucorpus.Case{
		core("default-echo", nil, "a b c\n", gnucorpus.Tree{}),
		core("dash-n-one-per-invocation", []string{"-n", "1", "echo"}, "a b c\n", gnucorpus.Tree{}),
		core("explicit-command", []string{"cat"}, "a.txt\nb.txt\n", tree),
		core("dash-capital-i-placeholder", []string{"-I", "{}", "echo", "got", "{}"}, "x\ny\n", gnucorpus.Tree{}),
		core("dash-zero-null-delimited", []string{"-0", "echo"}, "a\x00b\x00", gnucorpus.Tree{}),
		core("no-input-no-run", []string{"echo", "fallback"}, "", gnucorpus.Tree{}),
		extended("dash-capital-l-lines-per-invocation", []string{"-L", "2", "echo"}, "a\nb\nc\nd\n", gnucorpus.Tree{}),
		extended("exit-nonzero-on-command-failure", []string{"false"}, "x\n", gnucorpus.Tree{}),
	}
}
