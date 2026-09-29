package wiregen

import (
	"fmt"
	"strings"
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
	case KindStruct:
		return t.Qualifier != ""
	case KindPointer, KindSlice, KindMap:
		return holdsForeign(t.Elem)
	}
	return false
}
