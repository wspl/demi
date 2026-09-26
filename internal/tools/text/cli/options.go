// Package cli holds what the GNU-compatible utilities share: command-line
// parsing as glibc's getopt_long does it, diagnostics in the GNU format, and
// the number syntax of the coreutils options.
package cli

import (
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/toolctx"
)

// ArgKind says whether an option takes an argument.
type ArgKind int

// The argument kinds of getopt_long.
const (
	NoArg ArgKind = iota
	RequiredArg
	// OptionalArg is attached only: "-xVALUE" or "--name=VALUE".
	OptionalArg
)

// Option is one option a utility accepts.
type Option struct {
	// Short is the one-letter form, or 0 when there is none.
	Short rune
	// Long is the long form without "--", or "" when there is none.
	Long string
	Arg  ArgKind
	// Key identifies the option in the parse result. It defaults to Long,
	// or to the short letter when there is no long form. Options that share
	// a key are the same option under different names.
	Key string
}

// key returns the option's identity in the parse result.
func (o Option) key() string {
	if o.Key != "" {
		return o.Key
	}
	if o.Long != "" {
		return o.Long
	}
	return string(o.Short)
}

// Parsed is one option or operand found on the command line.
type Parsed struct {
	Key string
	// Value is the option's argument; HasValue tells an empty argument from
	// an absent optional one.
	Value    string
	HasValue bool
	// Name is the option as written without its value: "-n" or "--lines".
	Name string
	// Index is the position in args of the argument the option starts in.
	Index int
	// Literal marks an operand after the end of the options: after "--",
	// or from the first operand on when options must come first.
	Literal bool
}

// Operand is the Key of a Parsed entry that is an operand; Value holds it.
const Operand = ""

// UsageError is an invalid command line, reported as getopt_long reports it.
type UsageError struct {
	Message string
	// Status is the exit status when it differs from the utility's usual
	// status for a bad command line; coreutils exits 1 for a bad option
	// argument even where a bad option exits 2.
	Status int
}

func (e *UsageError) Error() string {
	return e.Message
}

// Parse parses args (without the program name) as getopt_long does, and
// returns options and operands in command-line order: options may follow
// operands unless POSIXLY_CORRECT is set, "--" ends the options, "-" is an
// operand, short options cluster, and a long option may be abbreviated to
// any unambiguous prefix. With optionsFirst, as with POSIXLY_CORRECT, the
// first operand ends the options. On an error it returns what it parsed
// before.
func Parse(args []string, options []Option, optionsFirst bool, env toolctx.Env) ([]Parsed, error) {
	_, posix := env.Get("POSIXLY_CORRECT")
	posix = posix || optionsFirst
	var parsed []Parsed
	literal := false
	for index := 0; index < len(args); index++ {
		arg := args[index]
		if !literal && arg == "--" {
			literal = true
			continue
		}
		if literal || len(arg) < 2 || arg[0] != '-' {
			literal = literal || posix
			parsed = append(parsed, Parsed{Key: Operand, Value: arg, HasValue: true, Index: index, Literal: literal})
			continue
		}
		var found []Parsed
		var err error
		if strings.HasPrefix(arg, "--") {
			found, index, err = parseLong(args, index, options)
		} else {
			found, index, err = parseShort(args, index, options)
		}
		if err != nil {
			return parsed, err
		}
		parsed = append(parsed, found...)
	}
	return parsed, nil
}

// parseLong parses the long option args[index] and returns the index of the
// last argument it consumed.
func parseLong(args []string, index int, options []Option) ([]Parsed, int, error) {
	start := index
	body := args[index][2:]
	name, value, hasValue := strings.Cut(body, "=")
	option, err := matchLong(name, body, options)
	if err != nil {
		return nil, index, err
	}
	written := "--" + option.Long
	switch option.Arg {
	case NoArg:
		if hasValue {
			return nil, index, &UsageError{Message: fmt.Sprintf("option '%s' doesn't allow an argument", written)}
		}
	case RequiredArg:
		if !hasValue {
			if index+1 >= len(args) {
				return nil, index, &UsageError{Message: fmt.Sprintf("option '%s' requires an argument", written)}
			}
			index++
			value = args[index]
			hasValue = true
		}
	case OptionalArg:
	}
	return []Parsed{{Key: option.key(), Value: value, HasValue: hasValue, Name: written, Index: start}}, index, nil
}

// matchLong finds the option that name names exactly or abbreviates. body
// is the argument after "--", which the messages quote.
func matchLong(name, body string, options []Option) (Option, error) {
	var matches []Option
	for _, option := range options {
		if option.Long == "" {
			continue
		}
		if option.Long == name {
			return option, nil
		}
		if strings.HasPrefix(option.Long, name) {
			matches = append(matches, option)
		}
	}
	if len(matches) == 0 {
		return Option{}, &UsageError{Message: fmt.Sprintf("unrecognized option '--%s'", body)}
	}
	// Abbreviations of names for one and the same option are not ambiguous.
	for _, match := range matches[1:] {
		if match.key() != matches[0].key() || match.Arg != matches[0].Arg {
			var message strings.Builder
			fmt.Fprintf(&message, "option '--%s' is ambiguous; possibilities:", name)
			for _, candidate := range matches {
				fmt.Fprintf(&message, " '--%s'", candidate.Long)
			}
			return Option{}, &UsageError{Message: message.String()}
		}
	}
	return matches[0], nil
}

// parseShort parses the short option cluster args[index] and returns the
// index of the last argument it consumed.
func parseShort(args []string, index int, options []Option) ([]Parsed, int, error) {
	start := index
	cluster := []rune(args[index][1:])
	var parsed []Parsed
	for position := 0; position < len(cluster); position++ {
		letter := cluster[position]
		option, ok := findShort(letter, options)
		if !ok {
			return nil, index, &UsageError{Message: fmt.Sprintf("invalid option -- '%c'", letter)}
		}
		result := Parsed{Key: option.key(), Name: "-" + string(letter), Index: start}
		rest := string(cluster[position+1:])
		switch option.Arg {
		case NoArg:
			parsed = append(parsed, result)
			continue
		case RequiredArg:
			switch {
			case rest != "":
				result.Value = rest
			case index+1 < len(args):
				index++
				result.Value = args[index]
			default:
				return nil, index, &UsageError{Message: fmt.Sprintf("option requires an argument -- '%c'", letter)}
			}
			result.HasValue = true
		case OptionalArg:
			result.Value = rest
			result.HasValue = rest != ""
		}
		return append(parsed, result), index, nil
	}
	return parsed, index, nil
}

func findShort(letter rune, options []Option) (Option, bool) {
	for _, option := range options {
		if option.Short == letter {
			return option, true
		}
	}
	return Option{}, false
}

// Choice is one group of equivalent words an option argument may take.
type Choice struct {
	Key   string
	Words []string
}

// Argmatch resolves the argument value of option (its long name with "--")
// to a choice as coreutils' argmatch does: an exact word, or a prefix of the
// words of exactly one choice. A failure lists the valid choices.
func Argmatch(option, value string, choices []Choice) (string, error) {
	var found []string
	for _, choice := range choices {
		for _, word := range choice.Words {
			if word == value {
				return choice.Key, nil
			}
		}
	}
	for _, choice := range choices {
		for _, word := range choice.Words {
			if strings.HasPrefix(word, value) {
				found = append(found, choice.Key)
				break
			}
		}
	}
	problem := "invalid"
	if len(found) > 0 {
		unique := true
		for _, key := range found[1:] {
			if key != found[0] {
				unique = false
			}
		}
		if unique {
			return found[0], nil
		}
		problem = "ambiguous"
	}
	var message strings.Builder
	fmt.Fprintf(&message, "%s argument %s for %s\nValid arguments are:", problem, Quote(value), Quote(option))
	for _, choice := range choices {
		quoted := make([]string, len(choice.Words))
		for index, word := range choice.Words {
			quoted[index] = Quote(word)
		}
		fmt.Fprintf(&message, "\n  - %s", strings.Join(quoted, ", "))
	}
	return "", &UsageError{Message: message.String(), Status: 1}
}
