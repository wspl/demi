package shell

import (
	"slices"
	"strings"

	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"

	"github.com/wspl/demi/internal/toolctx"
)

// environ is an environment as "name=value" entries sorted by name, with
// each name once. It is what a utility sees as toolctx.Env and what an
// external program receives.
type environ []string

var _ toolctx.Env = environ(nil)

// exported returns the variables of a shell environment that commands
// receive.
func exported(env expand.Environ) environ {
	return newEnviron(interp.ExecEnv(env))
}

// newEnviron sorts entries by name and keeps the last value of each name.
// Entries without "=" are dropped.
func newEnviron(entries []string) environ {
	byName := make(map[string]string, len(entries))
	for _, entry := range entries {
		name, value, ok := strings.Cut(entry, "=")
		if !ok || name == "" {
			continue
		}
		byName[name] = value
	}
	env := make(environ, 0, len(byName))
	for name, value := range byName {
		env = append(env, name+"="+value)
	}
	slices.SortFunc(env, func(a, b string) int {
		return strings.Compare(entryName(a), entryName(b))
	})
	return env
}

// fromToolEnv copies a utility's environment.
func fromToolEnv(env toolctx.Env) environ {
	var entries []string
	env.Each(func(name, value string) bool {
		entries = append(entries, name+"="+value)
		return true
	})
	return newEnviron(entries)
}

func entryName(entry string) string {
	name, _, _ := strings.Cut(entry, "=")
	return name
}

// Get implements toolctx.Env.
func (e environ) Get(name string) (string, bool) {
	i, found := slices.BinarySearchFunc(e, name, func(entry, name string) int {
		return strings.Compare(entryName(entry), name)
	})
	if !found {
		return "", false
	}
	_, value, _ := strings.Cut(e[i], "=")
	return value, true
}

// Each implements toolctx.Env in name order.
func (e environ) Each(fn func(name, value string) bool) {
	for _, entry := range e {
		name, value, _ := strings.Cut(entry, "=")
		if !fn(name, value) {
			return
		}
	}
}
