package definitions

import "github.com/wspl/demi/internal/testing/gnucorpus"

func basenameCases() []gnucorpus.Case {
	return []gnucorpus.Case{
		core("simple-path", []string{"/a/b/c.txt"}, "", gnucorpus.Tree{}),
		core("trailing-slash", []string{"/a/b/"}, "", gnucorpus.Tree{}),
		core("strip-suffix", []string{"-s", ".txt", "/a/b/c.txt"}, "", gnucorpus.Tree{}),
		core("no-directory-component", []string{"c.txt"}, "", gnucorpus.Tree{}),
		extended("multiple-with-dash-a", []string{"-a", "/a/b.txt", "/c/d.txt"}, "", gnucorpus.Tree{}),
	}
}

func dirnameCases() []gnucorpus.Case {
	return []gnucorpus.Case{
		core("simple-path", []string{"/a/b/c.txt"}, "", gnucorpus.Tree{}),
		core("no-directory-component", []string{"c.txt"}, "", gnucorpus.Tree{}),
		core("root", []string{"/"}, "", gnucorpus.Tree{}),
		extended("multiple-operands", []string{"/a/b.txt", "/c/d.txt"}, "", gnucorpus.Tree{}),
	}
}

func envCases() []gnucorpus.Case {
	// HOME is normally the case's temp directory, whose name is unique to
	// each run; a case that prints the whole environment fixes HOME to a
	// literal value instead, since env never reads that directory.
	fixedHome := map[string]string{"HOME": "/home/fixture"}

	printsEnvironment := core("prints-environment", nil, "", gnucorpus.Tree{})
	printsEnvironment.Env = fixedHome

	setsAVariable := core("sets-a-variable-for-the-command", []string{"FOO=bar", "env"}, "", gnucorpus.Tree{})
	setsAVariable.Env = fixedHome

	nullSeparated := extended("null-separated-output", []string{"-0"}, "", gnucorpus.Tree{})
	nullSeparated.Env = fixedHome

	return []gnucorpus.Case{
		printsEnvironment,
		setsAVariable,
		core("dash-i-clears-inherited-environment", []string{"-i", "FOO=bar", "env"}, "", gnucorpus.Tree{}),
		core("dash-u-unsets-a-variable", []string{"-u", "HOME", "env"}, "", gnucorpus.Tree{}),
		nullSeparated,
	}
}

func seqCases() []gnucorpus.Case {
	return []gnucorpus.Case{
		core("last-only", []string{"5"}, "", gnucorpus.Tree{}),
		core("first-and-last", []string{"2", "6"}, "", gnucorpus.Tree{}),
		core("first-increment-last", []string{"2", "2", "10"}, "", gnucorpus.Tree{}),
		core("empty-range", []string{"5", "1"}, "", gnucorpus.Tree{}),
		core("negative-increment", []string{"5", "-1", "1"}, "", gnucorpus.Tree{}),
		core("custom-separator", []string{"-s", ",", "1", "3"}, "", gnucorpus.Tree{}),
		extended("format-string", []string{"-f", "%03g", "1", "3"}, "", gnucorpus.Tree{}),
		extended("equal-width", []string{"-w", "8", "10"}, "", gnucorpus.Tree{}),
	}
}

func dateCases() []gnucorpus.Case {
	// Every case fixes the moment with -d so the result never depends on
	// "now"; TZ=UTC in the base environment makes it depend only on that
	// fixed moment and the format string.
	return []gnucorpus.Case{
		core("epoch-to-iso-date", []string{"-u", "-d", "@0", "+%Y-%m-%d"}, "", gnucorpus.Tree{}),
		core("epoch-seconds-round-trip", []string{"-u", "-d", "@1700000000", "+%s"}, "", gnucorpus.Tree{}),
		core("day-of-week-name", []string{"-u", "-d", "2024-01-02", "+%A"}, "", gnucorpus.Tree{}),
		core("iso-8601", []string{"-u", "-d", "@0", "--iso-8601=seconds"}, "", gnucorpus.Tree{}),
		core("rfc-3339", []string{"-u", "-d", "@0", "--rfc-3339=seconds"}, "", gnucorpus.Tree{}),
		core("invalid-date-errors", []string{"-d", "not a date"}, "", gnucorpus.Tree{}),
		extended("custom-format-fields", []string{"-u", "-d", "2024-06-15", "+%j day of %Y"}, "", gnucorpus.Tree{}),
	}
}

func sleepCases() []gnucorpus.Case {
	// Only zero-length durations, so recording and checking stay fast; the
	// interesting behavior is argument parsing, not timing.
	return []gnucorpus.Case{
		core("zero-seconds", []string{"0"}, "", gnucorpus.Tree{}),
		core("fractional-zero", []string{"0.0"}, "", gnucorpus.Tree{}),
		core("multiple-operands-sum", []string{"0", "0"}, "", gnucorpus.Tree{}),
		core("invalid-duration-errors", []string{"not-a-number"}, "", gnucorpus.Tree{}),
		core("missing-operand-errors", nil, "", gnucorpus.Tree{}),
	}
}

func pasteCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.txt": "1\n2\n3\n", "b.txt": "x\ny\nz\n"})
	return []gnucorpus.Case{
		core("two-files-tab-joined", []string{"a.txt", "b.txt"}, "", tree),
		core("custom-delimiter", []string{"-d", ",", "a.txt", "b.txt"}, "", tree),
		core("serial-single-file", []string{"-s", "a.txt"}, "", tree),
		core("stdin-with-dash", []string{"-", "b.txt"}, "1\n2\n3\n", files(map[string]string{"b.txt": "x\ny\nz\n"})),
		extended("mismatched-line-counts", []string{"a.txt", "short.txt"}, "", files(map[string]string{
			"a.txt": "1\n2\n3\n", "short.txt": "x\n",
		})),
	}
}

func nlCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.txt": "one\n\ntwo\nthree\n"})
	return []gnucorpus.Case{
		core("default-skips-blank-lines", []string{"a.txt"}, "", tree),
		core("number-all-lines", []string{"-ba", "a.txt"}, "", tree),
		core("custom-separator", []string{"-s", ": ", "a.txt"}, "", tree),
		core("custom-width", []string{"-w", "3", "a.txt"}, "", tree),
		core("stdin", nil, "a\nb\n", gnucorpus.Tree{}),
		extended("right-justified-no-leading-zeros", []string{"-n", "rn", "a.txt"}, "", tree),
		extended("right-justified-leading-zeros", []string{"-n", "rz", "a.txt"}, "", tree),
	}
}

func tacCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.txt": "one\ntwo\nthree\n"})
	return []gnucorpus.Case{
		core("reverses-line-order", []string{"a.txt"}, "", tree),
		core("stdin", nil, "one\ntwo\nthree\n", gnucorpus.Tree{}),
		core("multiple-files", []string{"a.txt", "a.txt"}, "", tree),
		core("missing-file-errors", []string{"missing.txt"}, "", tree),
		extended("custom-separator", []string{"-s", ","}, "one,two,three", gnucorpus.Tree{}),
	}
}

func odCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.bin": "hello\x00\x01\x02world"})
	return []gnucorpus.Case{
		core("hex-bytes-no-address", []string{"-An", "-tx1", "a.bin"}, "", tree),
		core("printable-characters", []string{"-c", "a.bin"}, "", tree),
		core("octal-default", []string{"a.bin"}, "", tree),
		core("skip-bytes", []string{"-j", "5", "-An", "-tx1", "a.bin"}, "", tree),
		core("limit-bytes", []string{"-N", "3", "-An", "-tx1", "a.bin"}, "", tree),
		core("stdin", []string{"-An", "-tx1"}, "hi", gnucorpus.Tree{}),
		extended("decimal-bytes", []string{"-An", "-tu1", "a.bin"}, "", tree),
	}
}
