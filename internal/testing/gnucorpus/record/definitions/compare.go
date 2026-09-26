package definitions

import "github.com/wspl/demi/internal/testing/gnucorpus"

func diffCases() []gnucorpus.Case {
	identical := files(map[string]string{"a.txt": "same\n", "b.txt": "same\n"})
	// The unified and context formats print each file's modification time,
	// so both files get a fixed mtime to keep that text deterministic.
	differing := gnucorpus.Tree{
		Files: map[string][]byte{"a.txt": []byte("one\ntwo\nthree\n"), "b.txt": []byte("one\nTWO\nthree\nfour\n")},
		Times: map[string]int64{"a.txt": 1_700_000_000, "b.txt": 1_700_000_100},
	}
	dirs := gnucorpus.Tree{Files: map[string][]byte{
		"da/common.txt": []byte("same\n"),
		"da/only-a.txt": []byte("a\n"),
		"db/common.txt": []byte("same\n"),
		"db/only-b.txt": []byte("b\n"),
	}}
	return []gnucorpus.Case{
		core("identical-files-exit-zero", []string{"a.txt", "b.txt"}, "", identical),
		core("default-format", []string{"a.txt", "b.txt"}, "", differing),
		core("unified-format", []string{"-u", "a.txt", "b.txt"}, "", differing),
		core("context-format", []string{"-c", "a.txt", "b.txt"}, "", differing),
		core("brief", []string{"-q", "a.txt", "b.txt"}, "", differing),
		core("report-identical-files", []string{"-s", "a.txt", "b.txt"}, "", identical),
		core("missing-file-errors", []string{"missing.txt", "b.txt"}, "", differing),
		extended("ed-script", []string{"-e", "a.txt", "b.txt"}, "", differing),
		extended("recursive-directories", []string{"-r", "da", "db"}, "", dirs),
	}
}

func cmpCases() []gnucorpus.Case {
	identical := files(map[string]string{"a.bin": "abcdef", "b.bin": "abcdef"})
	differing := files(map[string]string{"a.bin": "abcXef", "b.bin": "abcdef"})
	shorter := files(map[string]string{"a.bin": "abc", "b.bin": "abcdef"})
	return []gnucorpus.Case{
		core("identical-files-exit-zero", []string{"a.bin", "b.bin"}, "", identical),
		core("first-difference-reported", []string{"a.bin", "b.bin"}, "", differing),
		core("silent-exit-code-only", []string{"-s", "a.bin", "b.bin"}, "", differing),
		core("list-all-differences", []string{"-l", "a.bin", "b.bin"}, "", differing),
		core("shorter-file-reported", []string{"a.bin", "b.bin"}, "", shorter),
		core("missing-file-errors", []string{"missing.bin", "b.bin"}, "", differing),
		extended("skip-first-bytes", []string{"-i", "3", "a.bin", "b.bin"}, "", differing),
		extended("limit-bytes-compared", []string{"-n", "3", "a.bin", "b.bin"}, "", differing),
	}
}
