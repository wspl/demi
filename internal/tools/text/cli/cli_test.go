package cli_test

import (
	"errors"
	"testing"

	"github.com/wspl/demi/internal/toolctx/toolctxtest"
	"github.com/wspl/demi/internal/tools/text/cli"
)

// File names are quoted in diagnostics as GNU coreutils 9.4 quotes them
// (recorded from cat's "No such file or directory" messages, LC_ALL=C).
func TestQuoteNameMatchesCoreutils(t *testing.T) {
	for name, want := range map[string]string{
		"x":           "x",
		"":            "''",
		"a b":         "'a b'",
		"it's":        `"it's"`,
		"a$b":         "'a$b'",
		"tab\tx":      `'tab'$'\t''x'`,
		"~a":          "'~a'",
		"a~":          "a~",
		"#a":          "'#a'",
		"a:b":         "'a:b'",
		"{a}":         "{a}",
		"\xc3\xa9":    `''$'\303\251'`,
		"a'b\"c":      `'a'\''b"c'`,
		"a\nb":        `'a'$'\n''b'`,
		"a'\x01b":     `'a'\'''$'\001''b'`,
		"a'b:c":       `"a'b:c"`,
		"a'b=c":       `'a'\''b=c'`,
		"a\t":         `'a'$'\t'`,
		"a\r\a\x7fz":  `'a'$'\r\a\177''z'`,
		"#a'b":        `"#a'b"`,
		"a'b,%+@-_./": `"a'b,%+@-_./"`,
	} {
		if got := cli.QuoteName(name); got != want {
			t.Errorf("QuoteName(%q) = %s, want %s", name, got, want)
		}
	}
}

// With POSIXLY_CORRECT set, the first operand ends the options.
func TestPosixlyCorrectStopsAtTheFirstOperand(t *testing.T) {
	options := []cli.Option{{Short: 'n', Long: "number"}}

	parsed, err := cli.Parse([]string{"a", "-n"}, options, false, toolctxtest.Env{"POSIXLY_CORRECT": "1"})

	if err != nil || len(parsed) != 2 || parsed[1].Key != cli.Operand || parsed[1].Value != "-n" {
		t.Fatalf("parsed = %+v, %v", parsed, err)
	}
}

// Size suffixes follow xstrtoumax: K and KiB are 1024, KB is 1000, a bare
// suffix counts one, and overflow saturates with an error.
func TestParseUintSuffixes(t *testing.T) {
	for value, want := range map[string]uint64{
		"10": 10, " +3": 3, "010": 10, "1K": 1024, "1KiB": 1024, "1kB": 1000,
		"2b": 1024, "K": 1024, "1M": 1 << 20, "1E": 1 << 60,
	} {
		got, err := cli.ParseUint(value, cli.SizeSuffixes)
		if err != nil || got != want {
			t.Errorf("ParseUint(%q) = %d, %v; want %d", value, got, err, want)
		}
	}
	for _, value := range []string{"", "-1", "1x", "1KIB", "0x10", "1.5", "1 K"} {
		if _, err := cli.ParseUint(value, cli.SizeSuffixes); !errors.Is(err, cli.ErrInvalidNumber) {
			t.Errorf("ParseUint(%q) error = %v", value, err)
		}
	}
	if _, err := cli.ParseUint("1Z", cli.SizeSuffixes); !errors.Is(err, cli.ErrOverflow) {
		t.Errorf("ParseUint(1Z) error = %v", err)
	}
}
