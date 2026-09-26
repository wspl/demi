package cli

import (
	"fmt"
	"strings"
)

// Quote quotes an argument in a diagnostic as coreutils does in the C
// locale: 'VALUE', with backslash escapes for quotes, backslashes and bytes
// that are not printable.
func Quote(value string) string {
	var quoted strings.Builder
	quoted.WriteByte('\'')
	for index := 0; index < len(value); index++ {
		c := value[index]
		switch {
		case c == '\'' || c == '\\':
			quoted.WriteByte('\\')
			quoted.WriteByte(c)
		case isPrintable(c):
			quoted.WriteByte(c)
		default:
			quoted.WriteString(escape(c))
		}
	}
	quoted.WriteByte('\'')
	return quoted.String()
}

// QuoteName quotes a file name in a diagnostic as coreutils does: unchanged
// when a shell would read it back as is, otherwise in shell quotes, with
// $'...' for bytes that are not printable.
func QuoteName(name string) string {
	if name == "" {
		return "''"
	}
	plain := true
	printable := true
	compatible := true
	hasQuote := false
	for index := 0; index < len(name); index++ {
		c := name[index]
		if !isPrintable(c) {
			printable = false
		}
		if needsShellQuote(c, index) {
			plain = false
		}
		if c == '\'' {
			hasQuote = true
		}
		if !doubleQuoteCompatible(c, index) {
			compatible = false
		}
	}
	if plain {
		return name
	}
	if hasQuote && printable && compatible {
		return `"` + name + `"`
	}
	var quoted strings.Builder
	quoted.WriteByte('\'')
	for index := 0; index < len(name); index++ {
		c := name[index]
		switch {
		case c == '\'':
			quoted.WriteString(`'\''`)
		case isPrintable(c):
			quoted.WriteByte(c)
		default:
			quoted.WriteString(`'$'`)
			for index < len(name) && !isPrintable(name[index]) {
				quoted.WriteString(escape(name[index]))
				index++
			}
			index--
			quoted.WriteString(`'`)
			if index == len(name)-1 {
				// The name ends with the $'...' part; no empty quotes follow.
				return quoted.String()
			}
			quoted.WriteString(`'`)
		}
	}
	quoted.WriteByte('\'')
	return quoted.String()
}

func isPrintable(c byte) bool {
	return c >= 0x20 && c < 0x7f
}

// needsShellQuote reports whether byte c at position index of a name keeps
// a shell from reading the name back unquoted.
func needsShellQuote(c byte, index int) bool {
	if !isPrintable(c) {
		return true
	}
	if strings.IndexByte(" !\"$&'()*:;<=>?[\\^`|", c) >= 0 {
		return true
	}
	return index == 0 && (c == '#' || c == '~')
}

// doubleQuoteCompatible reports whether byte c at position index means the
// same inside double quotes as coreutils expects when it prefers double
// quotes for a name that contains a single quote.
func doubleQuoteCompatible(c byte, index int) bool {
	switch {
	case c >= 'a' && c <= 'z', c >= 'A' && c <= 'Z', c >= '0' && c <= '9':
		return true
	case strings.IndexByte("%+,-./:@_ '", c) >= 0:
		return true
	}
	return index == 0 && (c == '#' || c == '~')
}

// escape returns the C escape of a byte that is not printable.
func escape(c byte) string {
	switch c {
	case '\a':
		return `\a`
	case '\b':
		return `\b`
	case '\t':
		return `\t`
	case '\n':
		return `\n`
	case '\v':
		return `\v`
	case '\f':
		return `\f`
	case '\r':
		return `\r`
	}
	return fmt.Sprintf(`\%03o`, c)
}
