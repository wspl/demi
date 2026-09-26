package definitions

import (
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/testing/gnucorpus"
)

// twentyLines is "line1\n".."line20\n", long enough to exercise head/tail's
// default window and their -n/-c variants on both ends.
func twentyLines() string {
	var b strings.Builder
	for i := 1; i <= 20; i++ {
		fmt.Fprintf(&b, "line%d\n", i)
	}
	return b.String()
}

func catCases() []gnucorpus.Case {
	tree := files(map[string]string{
		"a.txt": "one\ntwo\n",
		"b.txt": "three\n",
		"empty": "",
	})
	blanks := files(map[string]string{"blanks.txt": "a\n\n\nb\n\n\n\nc\n"})
	tabs := files(map[string]string{"tabs.txt": "a\tb\nno-newline-at-end"})
	return []gnucorpus.Case{
		core("one-file", []string{"a.txt"}, "", tree),
		core("two-files", []string{"a.txt", "b.txt"}, "", tree),
		core("dash-reads-stdin", []string{"a.txt", "-", "b.txt"}, "STDIN\n", tree),
		core("number-all-lines", []string{"-n", "a.txt"}, "", tree),
		core("number-nonblank", []string{"-b", "blanks.txt"}, "", blanks),
		core("squeeze-blank", []string{"-s", "blanks.txt"}, "", blanks),
		core("missing-file", []string{"missing.txt"}, "", tree),
		extended("show-ends", []string{"-E", "a.txt"}, "", tree),
		extended("show-tabs", []string{"-T", "tabs.txt"}, "", tabs),
		extended("show-all", []string{"-A", "tabs.txt"}, "", tabs),
		extended("empty-file", []string{"empty"}, "", tree),
		extended("double-dash", []string{"--", "a.txt"}, "", tree),
	}
}

func headCases() []gnucorpus.Case {
	tree := files(map[string]string{"twenty.txt": twentyLines(), "short.txt": "a\nb\n"})
	return []gnucorpus.Case{
		core("default-ten-lines", []string{"twenty.txt"}, "", tree),
		core("dash-n-count", []string{"-n", "3", "twenty.txt"}, "", tree),
		core("dash-n-space-count", []string{"-3", "twenty.txt"}, "", tree),
		core("dash-c-count", []string{"-c", "7", "twenty.txt"}, "", tree),
		core("stdin", nil, "x\ny\nz\n", gnucorpus.Tree{}),
		core("two-files-headers", []string{"-n", "1", "twenty.txt", "short.txt"}, "", tree),
		core("missing-file", []string{"missing.txt"}, "", tree),
		extended("negative-n-all-but-last", []string{"-n", "-5", "twenty.txt"}, "", tree),
		extended("negative-c-all-but-last", []string{"-c", "-5", "twenty.txt"}, "", tree),
		extended("quiet-with-two-files", []string{"-q", "twenty.txt", "short.txt"}, "", tree),
		extended("verbose-single-file", []string{"-v", "short.txt"}, "", tree),
	}
}

func tailCases() []gnucorpus.Case {
	tree := files(map[string]string{"twenty.txt": twentyLines(), "short.txt": "a\nb\n"})
	return []gnucorpus.Case{
		core("default-ten-lines", []string{"twenty.txt"}, "", tree),
		core("dash-n-count", []string{"-n", "3", "twenty.txt"}, "", tree),
		core("dash-n-plus-from-start", []string{"-n", "+18", "twenty.txt"}, "", tree),
		core("dash-c-count", []string{"-c", "7", "twenty.txt"}, "", tree),
		core("stdin", nil, "x\ny\nz\n", gnucorpus.Tree{}),
		core("two-files-headers", []string{"-n", "1", "twenty.txt", "short.txt"}, "", tree),
		core("missing-file", []string{"missing.txt"}, "", tree),
		extended("dash-c-plus-from-start", []string{"-c", "+16"}, "", gnucorpus.Tree{}),
		extended("quiet-with-two-files", []string{"-q", "twenty.txt", "short.txt"}, "", tree),
		extended("verbose-single-file", []string{"-v", "short.txt"}, "", tree),
	}
}

func wcCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.txt": "one two\nthree\n", "b.txt": "four\n"})
	return []gnucorpus.Case{
		core("default-lines-words-bytes", []string{"a.txt"}, "", tree),
		core("dash-l", []string{"-l", "a.txt"}, "", tree),
		core("dash-w", []string{"-w", "a.txt"}, "", tree),
		core("dash-c-bytes", []string{"-c", "a.txt"}, "", tree),
		core("stdin", nil, "one two\nthree\n", gnucorpus.Tree{}),
		core("multiple-files-total", []string{"a.txt", "b.txt"}, "", tree),
		core("missing-file", []string{"missing.txt"}, "", tree),
		extended("dash-m-chars", []string{"-m", "a.txt"}, "", tree),
		extended("dash-capital-l-longest-line", []string{"-L", "a.txt"}, "", tree),
		extended("combined-l-w", []string{"-lw", "a.txt"}, "", tree),
	}
}

func teeCases() []gnucorpus.Case {
	tree := files(map[string]string{"existing.txt": "old\n"})
	return []gnucorpus.Case{
		core("copies-stdin-to-file-and-stdout", []string{"out.txt"}, "hello\n", gnucorpus.Tree{}),
		core("two-files", []string{"a.txt", "b.txt"}, "hello\n", gnucorpus.Tree{}),
		core("append", []string{"-a", "existing.txt"}, "new\n", tree),
		core("truncates-by-default", []string{"existing.txt"}, "new\n", tree),
		extended("dash-i-ignore-interrupts", []string{"-i", "out.txt"}, "x\n", gnucorpus.Tree{}),
		extended("no-file-operand", nil, "just-stdout\n", gnucorpus.Tree{}),
	}
}

func sortCases() []gnucorpus.Case {
	lines := files(map[string]string{"a.txt": "banana\napple\ncherry\napple\n"})
	nums := files(map[string]string{"nums.txt": "10\n2\n33\n4\n"})
	fields := files(map[string]string{"fields.txt": "b:2\na:10\nc:1\n"})
	sorted := files(map[string]string{"sorted.txt": "a\nb\nc\n"})
	unsorted := files(map[string]string{"unsorted.txt": "b\na\nc\n"})
	return []gnucorpus.Case{
		core("default-lexical", []string{"a.txt"}, "", lines),
		core("reverse", []string{"-r", "a.txt"}, "", lines),
		core("unique", []string{"-u", "a.txt"}, "", lines),
		core("numeric", []string{"-n", "nums.txt"}, "", nums),
		core("field-key", []string{"-t", ":", "-k", "2,2n", "fields.txt"}, "", fields),
		core("stdin", nil, "banana\napple\n", gnucorpus.Tree{}),
		core("check-sorted-succeeds", []string{"-c", "sorted.txt"}, "", sorted),
		core("check-unsorted-fails", []string{"-c", "unsorted.txt"}, "", unsorted),
		extended("fold-case", []string{"-f", "a.txt"}, "", lines),
		extended("human-numeric", []string{"-h"}, "10K\n2K\n1M\n", gnucorpus.Tree{}),
		extended("merge-two-files", []string{"-m", "a.txt", "sorted.txt"}, "", gnucorpus.Tree{Files: mergeFiles(lines, sorted)}),
		extended("output-to-file", []string{"-o", "out.txt", "a.txt"}, "", lines),
	}
}

// mergeFiles combines two Trees' Files maps for cases that read from more
// than one input tree.
func mergeFiles(trees ...gnucorpus.Tree) map[string][]byte {
	m := map[string][]byte{}
	for _, tree := range trees {
		for path, content := range tree.Files {
			m[path] = content
		}
	}
	return m
}

func uniqCases() []gnucorpus.Case {
	stdinLines := "a\na\nb\nb\nb\nc\n"
	return []gnucorpus.Case{
		core("default-adjacent-dedup", nil, stdinLines, gnucorpus.Tree{}),
		core("count", []string{"-c"}, stdinLines, gnucorpus.Tree{}),
		core("only-duplicated", []string{"-d"}, stdinLines, gnucorpus.Tree{}),
		core("only-unique", []string{"-u"}, stdinLines, gnucorpus.Tree{}),
		core("ignore-case", []string{"-i"}, "A\na\nB\n", gnucorpus.Tree{}),
		core("file-argument", []string{"in.txt"}, "", files(map[string]string{"in.txt": stdinLines})),
		extended("skip-fields", []string{"-f", "1"}, "x a\ny a\nz b\n", gnucorpus.Tree{}),
		extended("skip-chars", []string{"-s", "2"}, "xaa\nyaa\nzbb\n", gnucorpus.Tree{}),
	}
}

func cutCases() []gnucorpus.Case {
	csv := "a,b,c\nd,e,f\n"
	return []gnucorpus.Case{
		core("fields-with-delimiter", []string{"-d", ",", "-f", "2"}, csv, gnucorpus.Tree{}),
		core("fields-range", []string{"-d", ",", "-f", "1-2"}, csv, gnucorpus.Tree{}),
		core("characters", []string{"-c", "1-3"}, "abcdef\n", gnucorpus.Tree{}),
		core("bytes", []string{"-b", "1-3"}, "abcdef\n", gnucorpus.Tree{}),
		core("file-argument", []string{"-d", ",", "-f", "1", "in.csv"}, "", files(map[string]string{"in.csv": csv})),
		extended("complement", []string{"-d", ",", "-f", "1", "--complement"}, csv, gnucorpus.Tree{}),
		extended("only-delimited", []string{"-d", ",", "-f", "2", "-s"}, "a,b\nno-delimiter\n", gnucorpus.Tree{}),
		extended("output-delimiter", []string{"-d", ",", "-f", "1-2", "--output-delimiter=-"}, csv, gnucorpus.Tree{}),
	}
}

func trCases() []gnucorpus.Case {
	return []gnucorpus.Case{
		core("translate-set", []string{"abc", "xyz"}, "aabbcc\n", gnucorpus.Tree{}),
		core("to-upper-classes", []string{"[:lower:]", "[:upper:]"}, "Hello World\n", gnucorpus.Tree{}),
		core("delete-set", []string{"-d", "aeiou"}, "hello world\n", gnucorpus.Tree{}),
		core("squeeze-repeats", []string{"-s", "l"}, "hellllo\n", gnucorpus.Tree{}),
		extended("complement-delete", []string{"-c", "-d", "a-z\n"}, "Hello, World! 123\n", gnucorpus.Tree{}),
		extended("truncate-set1", []string{"-t", "abc", "xy"}, "abcabc\n", gnucorpus.Tree{}),
		extended("delete-and-squeeze", []string{"-ds", "aeiou"}, "aabbccddeeffgg\n", gnucorpus.Tree{}),
	}
}
