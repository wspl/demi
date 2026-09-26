package definitions

import "github.com/wspl/demi/internal/testing/gnucorpus"

func sedCases() []gnucorpus.Case {
	tree := files(map[string]string{
		"a.txt":  "hello world\nhello there\ngoodbye\n",
		"nl.txt": "one\ntwo\nthree\nfour\n",
	})
	return []gnucorpus.Case{
		core("substitute-first-match", []string{"s/hello/hi/", "a.txt"}, "", tree),
		core("substitute-global", []string{"s/hello/hi/g", "a.txt"}, "", tree),
		core("delete-line-by-address", []string{"2d", "nl.txt"}, "", tree),
		core("print-with-quiet", []string{"-n", "2p", "nl.txt"}, "", tree),
		core("range-address", []string{"-n", "2,3p", "nl.txt"}, "", tree),
		core("multiple-expressions", []string{"-e", "s/one/1/", "-e", "s/two/2/", "nl.txt"}, "", tree),
		core("stdin", []string{"s/a/b/"}, "cat\nbat\n", gnucorpus.Tree{}),
		core("in-place-edit", []string{"-i", "s/hello/hi/g", "a.txt"}, "", tree),
		extended("extended-regex", []string{"-E", "s/(hello) (world)/\\2 \\1/", "a.txt"}, "", tree),
		extended("in-place-with-backup-suffix", []string{"-i.bak", "s/hello/hi/g", "a.txt"}, "", tree),
		extended("last-line-address", []string{"-n", "$p", "nl.txt"}, "", tree),
		extended("case-insensitive-flag", []string{"s/HELLO/hi/gi", "a.txt"}, "", tree),
	}
}

func jqCases() []gnucorpus.Case {
	obj := `{"name":"demi","tags":["go","rust"],"count":2,"nested":{"a":1,"b":2}}` + "\n"
	arr := "[1,2,3,4,5]\n"
	lines := `{"n":1}` + "\n" + `{"n":2}` + "\n"
	return []gnucorpus.Case{
		core("identity", []string{"."}, obj, gnucorpus.Tree{}),
		core("field-access", []string{".name"}, obj, gnucorpus.Tree{}),
		core("raw-output", []string{"-r", ".name"}, obj, gnucorpus.Tree{}),
		core("compact-output", []string{"-c", "."}, obj, gnucorpus.Tree{}),
		core("array-index", []string{".tags[0]"}, obj, gnucorpus.Tree{}),
		core("array-length", []string{"length"}, arr, gnucorpus.Tree{}),
		core("map-expression", []string{"map(. * 2)"}, arr, gnucorpus.Tree{}),
		core("select-filter", []string{"map(select(. > 2))"}, arr, gnucorpus.Tree{}),
		core("keys", []string{"-c", "keys"}, obj, gnucorpus.Tree{}),
		core("null-input-with-arg", []string{"-n", "--arg", "x", "hi", "$x"}, "", gnucorpus.Tree{}),
		core("slurp-lines-to-array", []string{"-c", "-s", "."}, lines, gnucorpus.Tree{}),
		core("invalid-json-error", []string{"."}, "not json\n", gnucorpus.Tree{}),
		extended("nested-field", []string{".nested.a"}, obj, gnucorpus.Tree{}),
		extended("add", []string{"add"}, arr, gnucorpus.Tree{}),
		extended("sort-by", []string{"-c", "sort_by(-.)"}, arr, gnucorpus.Tree{}),
		extended("tostring-then-fromjson", []string{"tostring"}, arr, gnucorpus.Tree{}),
	}
}
