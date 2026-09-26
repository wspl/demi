// Package tools lists the standard utilities that shell jobs run in the
// runner process (docs/demi-next/runner.md § Shell jobs).
//
// Each utility family is a package under internal/tools that exports
//
//	var Utilities = map[string]toolctx.Utility{...}
//
// and is added to families below.
package tools

import (
	"fmt"
	"maps"

	"github.com/wspl/demi/internal/toolctx"
)

// families are the utility families' Utilities maps.
var families = []map[string]toolctx.Utility{}

// Utilities are all standard utilities by name.
var Utilities = merge(families)

// merge joins the families; two families must not define the same name.
func merge(families []map[string]toolctx.Utility) map[string]toolctx.Utility {
	all := make(map[string]toolctx.Utility)
	for _, family := range families {
		for name := range maps.Keys(family) {
			if _, ok := all[name]; ok {
				panic(fmt.Sprintf("utility %q is defined by two families", name))
			}
		}
		maps.Copy(all, family)
	}
	return all
}
