package declare

import (
	"errors"
	"regexp"
	"regexp/syntax"
	"strings"

	"github.com/dlclark/regexp2"
	"github.com/santhosh-tekuri/jsonschema/v6"
)

// compilePattern uses RE2 for ordinary schema patterns, as fancy-regex does.
// Only lookaround and backreference syntax requires the backtracking engine.
func compilePattern(pattern string) (jsonschema.Regexp, error) {
	re, err := regexp.Compile(pattern)
	if err == nil {
		return re, nil
	}
	var parse *syntax.Error
	if !errors.As(err, &parse) {
		return nil, err
	}
	advanced := false
	switch parse.Code {
	case syntax.ErrInvalidPerlOp:
		advanced = parse.Expr == "(?=" || parse.Expr == "(?!" || parse.Expr == "(?P="
	case syntax.ErrInvalidNamedCapture:
		advanced = strings.HasPrefix(parse.Expr, "(?<=") || strings.HasPrefix(parse.Expr, "(?<!")
	case syntax.ErrInvalidEscape:
		advanced = len(parse.Expr) == 2 && (parse.Expr[1] >= '1' && parse.Expr[1] <= '9' || parse.Expr[1] == 'k')
	}
	if !advanced {
		return nil, err
	}
	// RE2 compatibility retains strict end-of-string anchors and the ordinary
	// engine's character classes while enabling lookaround and backreferences.
	backtracking, err := regexp2.Compile(pattern, regexp2.RE2)
	if err != nil {
		return nil, err
	}
	return schemaRegexp{regexp: backtracking}, nil
}

type schemaRegexp struct {
	regexp *regexp2.Regexp
}

func (r schemaRegexp) String() string { return r.regexp.String() }

func (r schemaRegexp) MatchString(value string) bool {
	// No timeout is installed, so the library documents no possible matching
	// error except an internal engine bug. The schema interface returns bool.
	matched, err := r.regexp.MatchString(value)
	return err == nil && matched
}
