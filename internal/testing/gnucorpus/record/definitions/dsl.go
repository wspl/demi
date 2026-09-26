package definitions

import "github.com/wspl/demi/internal/testing/gnucorpus"

// files builds a gnucorpus.Tree of plain files from path/content pairs.
func files(pairs map[string]string) gnucorpus.Tree {
	m := make(map[string][]byte, len(pairs))
	for path, content := range pairs {
		m[path] = []byte(content)
	}
	return gnucorpus.Tree{Files: m}
}

// core is a case definition whose option is in the core set.
func core(name string, argv []string, stdin string, tree gnucorpus.Tree) gnucorpus.Case {
	return gnucorpus.Case{Name: name, Core: true, Argv: argv, Stdin: []byte(stdin), Tree: tree}
}

// extended is a case definition whose option is in the extended set.
func extended(name string, argv []string, stdin string, tree gnucorpus.Tree) gnucorpus.Case {
	return gnucorpus.Case{Name: name, Core: false, Argv: argv, Stdin: []byte(stdin), Tree: tree}
}
