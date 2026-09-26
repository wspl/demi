// Package text holds the text utilities of shell jobs: cat, head, tail, wc,
// tee, sort, uniq, cut and tr. Each behaves as its GNU coreutils 9.4
// counterpart does in the C locale and reaches its job only through its
// toolctx.Invocation.
package text

import "github.com/wspl/demi/internal/toolctx"

// Utilities are the text utilities by name.
var Utilities = map[string]toolctx.Utility{
	"cat":  cat,
	"head": head,
	"tail": tail,
	"wc":   wc,
	"tee":  tee,
	"uniq": uniq,
	"cut":  cut,
	"tr":   tr,
	"sort": sortMain,
}
