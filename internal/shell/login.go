package shell

import (
	"fmt"
	"path/filepath"
	"strings"

	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

// userProfiles are the user login profiles; a login shell reads the first
// readable one, as Bash does.
var userProfiles = []string{".bash_profile", ".bash_login", ".profile"}

// login makes the job a fresh login shell: it reads the system profile and
// the first readable user login profile, then restores the job's own
// context, which the profiles cannot replace: the runner-owned variables, the
// command aliases first in PATH, and the working directory. Shell variables
// and functions from the profiles stay for the script. login reports the
// exit status and true when a profile exited, or restoring failed, and the
// script must not run.
func (j *Job) login(runner *interp.Runner) (int, bool) {
	var profiles strings.Builder
	if profile := j.shell.config.SystemProfile; profile != "" {
		fmt.Fprintf(&profiles, "if [ -r %s ]; then . %[1]s; fi\n", quote(profile))
	}
	profiles.WriteString("if [ -n \"${HOME-}\" ]; then\n")
	for i, name := range userProfiles {
		keyword := "elif"
		if i == 0 {
			keyword = "if"
		}
		path := "\"$HOME\"/" + name
		fmt.Fprintf(&profiles, "\t%s [ -r %s ]; then . %[2]s\n", keyword, path)
	}
	profiles.WriteString("\tfi\nfi\n")
	if code, exited := j.runStatements(runner, profiles.String()); exited {
		return code, true
	}
	if code, exited := j.runStatements(runner, j.restoreScript(runner)); exited {
		return code, true
	}
	if runner.Dir != filepath.Clean(j.spec.Dir) {
		// cd reported why on standard error. The script must not run
		// somewhere else than where it was asked to.
		return 1, true
	}
	return 0, false
}

// restoreScript returns the statements that restore the job's own context
// after the profiles.
func (j *Job) restoreScript(runner *interp.Runner) string {
	var script strings.Builder
	for _, entry := range newEnviron(j.spec.Env) {
		name, value, _ := strings.Cut(entry, "=")
		if !runnerOwned(name) {
			continue
		}
		fmt.Fprintf(&script, "export %s=%s\n", name, quote(value))
	}
	if commands := j.spec.Commands; commands != nil && commands.AliasDir != "" {
		fmt.Fprintf(&script, "export PATH=%s\n", quote(aliasPath(commands.AliasDir, runner.Vars["PATH"].String())))
	}
	fmt.Fprintf(&script, "builtin cd -- %s\n", quote(j.spec.Dir))
	return script.String()
}

// runnerOwned reports whether the runner owns the variable name, so that the
// login profiles cannot replace it.
func runnerOwned(name string) bool {
	return strings.HasPrefix(name, "DEMI_") || name == "TMPDIR" || name == "TEMP"
}

// aliasPath returns path with the alias directory first and nowhere else.
func aliasPath(aliasDir, path string) string {
	entries := []string{aliasDir}
	if path != "" {
		for _, entry := range filepath.SplitList(path) {
			if entry != aliasDir {
				entries = append(entries, entry)
			}
		}
	}
	return strings.Join(entries, string(filepath.ListSeparator))
}

// runStatements runs generated statements one at a time, so that finishing
// them is not the shell's exit and fires no exit trap. It reports the exit
// status and true when a statement made the shell exit, or failed while
// restoring the context.
func (j *Job) runStatements(runner *interp.Runner, source string) (int, bool) {
	file, err := syntax.NewParser().Parse(strings.NewReader(source), "")
	if err != nil {
		// The statements are generated with quoted values; they always parse.
		panic(fmt.Sprintf("generated login statements do not parse: %v", err))
	}
	for _, stmt := range file.Stmts {
		err := runner.Run(j.ctx, stmt)
		if runner.Exited() {
			result := exitResult(err, runner.Dir)
			return result.Code, true
		}
	}
	return 0, false
}

// quote quotes s as one shell word.
func quote(s string) string {
	quoted, err := syntax.Quote(s, syntax.LangBash)
	if err != nil {
		// Only a string with a null byte cannot be quoted, and neither
		// paths nor environment values can hold one.
		panic(fmt.Sprintf("cannot quote %q: %v", s, err))
	}
	return quoted
}
