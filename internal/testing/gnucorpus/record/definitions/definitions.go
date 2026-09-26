// Package definitions is the source of truth for what the differential
// corpus records: one case list per utility (argv, stdin, the tree to create
// first, and whether the case is core or extended). It holds no expected
// output; internal/testing/gnucorpus/record fills that in by running each
// utility's Reference binary and writes the result to
// testdata/gnu/<utility>/cases.json.
package definitions

import "github.com/wspl/demi/internal/testing/gnucorpus"

// Utility is one of the 41 standard utilities as the recorder sees it.
type Utility struct {
	// Reference is the reference binary's name, resolved through PATH.
	Reference string
	// Cases returns fresh case definitions (Want is always zero; the
	// recorder fills it in). A function, not a slice, so every recording
	// run starts from an unshared copy.
	Cases func() []gnucorpus.Case
}

// Order lists the 41 utilities in the brief's recording order: the text
// family first, since T1a starts from it.
var Order = []string{
	"cat", "head", "tail", "wc", "tee", "sort", "uniq", "cut", "tr",
	"grep", "rg", "find", "xargs",
	"sed", "jq",
	"ls", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "stat", "du", "df",
	"chmod", "chown", "realpath", "mktemp",
	"basename", "dirname", "env", "seq", "date", "sleep", "paste", "nl", "tac", "od",
	"diff", "cmp",
}

// Registry maps each utility to its reference binary and case definitions.
var Registry = map[string]Utility{
	"cat":  {Reference: "cat", Cases: catCases},
	"head": {Reference: "head", Cases: headCases},
	"tail": {Reference: "tail", Cases: tailCases},
	"wc":   {Reference: "wc", Cases: wcCases},
	"tee":  {Reference: "tee", Cases: teeCases},
	"sort": {Reference: "sort", Cases: sortCases},
	"uniq": {Reference: "uniq", Cases: uniqCases},
	"cut":  {Reference: "cut", Cases: cutCases},
	"tr":   {Reference: "tr", Cases: trCases},

	"grep":  {Reference: "grep", Cases: grepCases},
	"rg":    {Reference: "rg", Cases: rgCases},
	"find":  {Reference: "find", Cases: findCases},
	"xargs": {Reference: "xargs", Cases: xargsCases},

	"sed": {Reference: "sed", Cases: sedCases},
	"jq":  {Reference: "jq", Cases: jqCases},

	"ls":       {Reference: "ls", Cases: lsCases},
	"cp":       {Reference: "cp", Cases: cpCases},
	"mv":       {Reference: "mv", Cases: mvCases},
	"rm":       {Reference: "rm", Cases: rmCases},
	"mkdir":    {Reference: "mkdir", Cases: mkdirCases},
	"rmdir":    {Reference: "rmdir", Cases: rmdirCases},
	"touch":    {Reference: "touch", Cases: touchCases},
	"stat":     {Reference: "stat", Cases: statCases},
	"du":       {Reference: "du", Cases: duCases},
	"df":       {Reference: "df", Cases: dfCases},
	"chmod":    {Reference: "chmod", Cases: chmodCases},
	"chown":    {Reference: "chown", Cases: chownCases},
	"realpath": {Reference: "realpath", Cases: realpathCases},
	"mktemp":   {Reference: "mktemp", Cases: mktempCases},

	"basename": {Reference: "basename", Cases: basenameCases},
	"dirname":  {Reference: "dirname", Cases: dirnameCases},
	"env":      {Reference: "env", Cases: envCases},
	"seq":      {Reference: "seq", Cases: seqCases},
	"date":     {Reference: "date", Cases: dateCases},
	"sleep":    {Reference: "sleep", Cases: sleepCases},
	"paste":    {Reference: "paste", Cases: pasteCases},
	"nl":       {Reference: "nl", Cases: nlCases},
	"tac":      {Reference: "tac", Cases: tacCases},
	"od":       {Reference: "od", Cases: odCases},

	"diff": {Reference: "diff", Cases: diffCases},
	"cmp":  {Reference: "cmp", Cases: cmpCases},
}
