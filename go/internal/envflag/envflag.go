// Package envflag reads a program's configuration from its command line and
// its environment, as clap reads an argument that names an environment
// variable: each setting from its flag, else its variable, else its default.
// An error names the flag or the variable whose value it refuses, never the
// value, and a UsageError tells what the command line or a value got wrong
// from the program's own refusals.
package envflag

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"strings"
)

// A Setting is one flag and the environment variable that also sets it.
type Setting struct {
	// Flag is the flag's name, which the command line gives as --<Flag>.
	Flag     string
	Variable string
	// Fallback is the value without the flag and the variable; "" for none.
	Fallback string
	// Required means there is no default and the program cannot start
	// without a value.
	Required bool
	// List means the flag may repeat and its value is a comma-separated
	// list.
	List bool
	// Parse reads one value; its error says what is wrong without the
	// value.
	Parse func(value string) error
	// explicit is whether the flag or the variable set the setting.
	explicit bool
}

// Explicit says whether the flag or the variable set the setting, rather
// than its default.
func (s *Setting) Explicit() bool { return s.explicit }

// A Switch is a flag that takes no value, given at most once.
type Switch struct {
	Flag   string
	Target *bool
}

// A Command is what a program reads from its command line and environment.
type Command struct {
	Name     string
	Settings []*Setting
	Switches []Switch
	// Usage writes the text -h and --help print.
	Usage func(io.Writer)
}

// A UsageError means the command line or a setting's value is not one the
// program reads, as clap says of its own refusals; the program's other
// refusals are its own. The Rust programs exit with status 2 for the first
// and 1 for the second.
type UsageError struct {
	Err error
}

func (e *UsageError) Error() string { return e.Err.Error() }

func (e *UsageError) Unwrap() error { return e.Err }

// IsUsage reports whether err is a [UsageError].
func IsUsage(err error) bool {
	var usage *UsageError
	return errors.As(err, &usage)
}

// Parse reads args, the program's arguments after its name, and the
// variables lookup finds into the command's settings and switches. -h and
// --help write the usage to help and answer a UsageError that wraps
// flag.ErrHelp.
func (c *Command) Parse(args []string, lookup func(string) (string, bool), help io.Writer) error {
	commandLine := flag.NewFlagSet(c.Name, flag.ContinueOnError)
	commandLine.SetOutput(io.Discard)
	// A flag given twice is refused, as clap refuses it, except a list.
	repeated := ""
	for _, sw := range c.Switches {
		commandLine.BoolFunc(sw.Flag, "", func(string) error {
			if *sw.Target {
				repeated = sw.Flag
			}
			*sw.Target = true
			return nil
		})
	}
	given := map[string][]string{}
	for _, s := range c.Settings {
		commandLine.Func(s.Flag, s.Variable, func(value string) error {
			if len(given[s.Flag]) > 0 && !s.List {
				repeated = s.Flag
			}
			given[s.Flag] = append(given[s.Flag], value)
			return nil
		})
	}
	if err := commandLine.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			c.Usage(help)
		}
		return &UsageError{Err: err}
	}
	if repeated != "" {
		return &UsageError{Err: fmt.Errorf("--%s cannot be used multiple times", repeated)}
	}
	if commandLine.NArg() > 0 {
		return &UsageError{Err: fmt.Errorf("unexpected argument %q", commandLine.Arg(0))}
	}
	for _, s := range c.Settings {
		if err := s.read(given[s.Flag], lookup); err != nil {
			return &UsageError{Err: err}
		}
	}
	return nil
}

// read sets the setting from its flag values, else its variable, else its
// default.
func (s *Setting) read(values []string, lookup func(string) (string, bool)) error {
	source := "--" + s.Flag
	if len(values) == 0 {
		if value, ok := lookup(s.Variable); ok {
			values = []string{value}
			source = s.Variable
		}
	}
	if len(values) == 0 {
		if s.Required {
			return fmt.Errorf("%s is required (--%s)", s.Variable, s.Flag)
		}
		if s.Fallback == "" {
			return nil
		}
		values = []string{s.Fallback}
	} else {
		s.explicit = true
	}
	if s.List {
		var items []string
		for _, value := range values {
			items = append(items, strings.Split(value, ",")...)
		}
		values = items
	}
	for _, value := range values {
		if err := s.Parse(value); err != nil {
			return fmt.Errorf("invalid value for %s: %w", source, err)
		}
	}
	return nil
}

// Environment returns the lookup of a list of NAME=VALUE entries, such as
// os.Environ's.
func Environment(environ []string) func(string) (string, bool) {
	values := make(map[string]string, len(environ))
	for _, entry := range environ {
		if name, value, ok := strings.Cut(entry, "="); ok {
			values[name] = value
		}
	}
	return func(name string) (string, bool) {
		value, ok := values[name]
		return value, ok
	}
}
