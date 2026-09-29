package wiregen

import (
	"fmt"
	"path/filepath"
	"strings"

	"golang.org/x/tools/go/packages"
)

// foreignRules checks what a field says about the wire structs of other packages
// it holds. Such a struct is decoded by its own package's generated code, which
// checks its structure; its rules are its package's, and only that package can
// run them, so the field names a func= rule that does (a function of that
// package, such as `func=other.Validate`, or one of this package that calls it).
// It also records the packages that func rules name, which the generated code
// imports.
func (p *reader) foreignRules(t *Type, rules []Rule) error {
	funcs, err := p.funcRules(rules)
	if err != nil {
		return err
	}
	if holdsForeign(t) && !funcs {
		return fmt.Errorf("the wire struct of another package is checked by its package: name a func= rule that runs its checks")
	}
	return nil
}

// funcRules reports whether the rules hold a func rule, and records the package
// each qualified func names.
func (p *reader) funcRules(rules []Rule) (bool, error) {
	found := false
	for _, rule := range rules {
		switch rule.Kind {
		case RuleFunc:
			found = true
			if qualifier, _, ok := strings.Cut(rule.Value, "."); ok && !p.use(qualifier) {
				return false, fmt.Errorf("func=%s: the file imports no package %s", rule.Value, qualifier)
			}
		case RuleEach, RuleKeys:
			inner, err := p.funcRules(rule.Inner)
			if err != nil {
				return false, err
			}
			if rule.Kind == RuleEach {
				found = found || inner
			}
		}
	}
	return found, nil
}

// holdsForeign reports whether a value of type t holds a wire struct of another
// package.
func holdsForeign(t *Type) bool {
	switch t.Kind {
	case KindStruct, KindUnion, KindString:
		return t.Qualifier != ""
	case KindPointer, KindSlice, KindMap:
		return holdsForeign(t.Elem)
	}
	return false
}

// foreignLoader resolves module imports through the Go toolchain's package API.
// It reads declarations only: generation must work before generated methods exist.
func foreignLoader(dir string) func(string) (*Package, error) {
	cache := map[string]*Package{}
	return func(path string) (*Package, error) {
		if p := cache[path]; p != nil {
			return p, nil
		}
		found, err := packages.Load(&packages.Config{Dir: dir, Mode: packages.NeedName | packages.NeedFiles}, path)
		if err != nil {
			return nil, err
		}
		if len(found) != 1 || len(found[0].GoFiles) == 0 {
			return nil, fmt.Errorf("cannot locate wire package %s", path)
		}
		p, err := Load(filepath.Dir(found[0].GoFiles[0]))
		if err != nil {
			return nil, err
		}
		cache[path] = p
		return p, nil
	}
}
