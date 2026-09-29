package wire

import (
	"errors"
	"fmt"
	"regexp"
	"strings"
	"unicode/utf8"
)

// A Report collects the rules a value breaks. The zero Report is empty; a
// value that breaks none is a nil error.
type Report struct {
	errs []error
}

// Err returns what the report holds: nil, or the one refusal, or all of them.
func (r *Report) Err() error {
	switch len(r.errs) {
	case 0:
		return nil
	case 1:
		return r.errs[0]
	default:
		return errors.Join(r.errs...)
	}
}

// Add records that the field at path breaks a rule.
func (r *Report) Add(path, rule string) {
	r.errs = append(r.errs, &InvalidError{Path: path, Rule: rule})
}

// Nest records the refusals in err, which the checks of the field elem of a
// value returned, with elem in front of their paths. A nil err records nothing.
func (r *Report) Nest(elem string, err error) {
	if err == nil {
		return
	}
	for _, leaf := range leaves(err) {
		var invalid *InvalidError
		if errors.As(leaf, &invalid) {
			r.errs = append(r.errs, &InvalidError{Path: Prefix(elem, invalid.Path), Rule: invalid.Rule})
			continue
		}
		r.Add(elem, leaf.Error())
	}
}

// Check records the error of a rule that a function of the type checks, if
// there is one: the error's text is the rule.
func (r *Report) Check(path string, err error) {
	if err == nil {
		return
	}
	var invalid *InvalidError
	if errors.As(err, &invalid) {
		r.Nest(path, err)
		return
	}
	r.Add(path, err.Error())
}

// leaves returns the errors that err joins, or err itself.
func leaves(err error) []error {
	if joined, ok := err.(interface{ Unwrap() []error }); ok {
		var all []error
		for _, inner := range joined.Unwrap() {
			all = append(all, leaves(inner)...)
		}
		return all
	}
	return []error{err}
}

// Bounds are the limits of a length or a number; a limit that is not set is
// open.
type Bounds struct {
	Min, Max       int64
	HasMin, HasMax bool
}

// AtLeast returns bounds with a lower limit only.
func AtLeast(min int64) Bounds { return Bounds{Min: min, HasMin: true} }

// AtMost returns bounds with an upper limit only.
func AtMost(max int64) Bounds { return Bounds{Max: max, HasMax: true} }

// Between returns bounds with both limits.
func Between(min, max int64) Bounds { return Bounds{Min: min, Max: max, HasMin: true, HasMax: true} }

// holds reports whether n is within the bounds.
func (b Bounds) holds(n int64) bool {
	return (!b.HasMin || n >= b.Min) && (!b.HasMax || n <= b.Max)
}

// rule words what the bounds ask of a quantity, in the words of unit (a plural
// such as "characters"; its singular is the plural without the final s).
func (b Bounds) rule(unit string) string {
	switch {
	case b.HasMin && b.HasMax:
		return fmt.Sprintf("must have %d to %d %s", b.Min, b.Max, unit)
	case b.HasMin:
		return fmt.Sprintf("must have at least %d %s", b.Min, plural(b.Min, unit))
	default:
		return fmt.Sprintf("must have at most %d %s", b.Max, plural(b.Max, unit))
	}
}

func plural(n int64, unit string) string {
	if n == 1 {
		return strings.TrimSuffix(unit, "s")
	}
	return unit
}

// Chars checks the length of s in Unicode scalar values, the one unit of a
// length in the contracts.
func (r *Report) Chars(path, s string, b Bounds) {
	if !b.holds(int64(utf8.RuneCountInString(s))) {
		r.Add(path, b.rule("characters"))
	}
}

// Bytes checks the length of s in bytes.
func (r *Report) Bytes(path, s string, b Bounds) {
	if !b.holds(int64(len(s))) {
		r.Add(path, b.rule("bytes"))
	}
}

// Items checks the length of a slice or a map.
func (r *Report) Items(path string, n int, b Bounds) {
	if !b.holds(int64(n)) {
		r.Add(path, b.rule("items"))
	}
}

// Int checks a number against its bounds.
func (r *Report) Int(path string, n int64, b Bounds) {
	if !b.holds(n) {
		r.Add(path, b.numberRule())
	}
}

// Uint checks a number of an unsigned type against its bounds.
func (r *Report) Uint(path string, n uint64, b Bounds) {
	if n > 1<<63-1 {
		// Larger than any bound the rules can state.
		if b.HasMax {
			r.Add(path, b.numberRule())
		}
		return
	}
	r.Int(path, int64(n), b)
}

func (b Bounds) numberRule() string {
	switch {
	case b.HasMin && b.HasMax:
		return fmt.Sprintf("must be from %d to %d", b.Min, b.Max)
	case b.HasMin:
		return fmt.Sprintf("must be at least %d", b.Min)
	default:
		return fmt.Sprintf("must be at most %d", b.Max)
	}
}

// FloatBounds are the limits of a float; a limit that is not set is open.
type FloatBounds struct {
	Min, Max       float64
	HasMin, HasMax bool
}

// FloatAtLeast returns bounds with a lower limit only.
func FloatAtLeast(min float64) FloatBounds { return FloatBounds{Min: min, HasMin: true} }

// FloatAtMost returns bounds with an upper limit only.
func FloatAtMost(max float64) FloatBounds { return FloatBounds{Max: max, HasMax: true} }

// FloatBetween returns bounds with both limits.
func FloatBetween(min, max float64) FloatBounds {
	return FloatBounds{Min: min, Max: max, HasMin: true, HasMax: true}
}

// Float checks a float against its bounds.
func (r *Report) Float(path string, n float64, b FloatBounds) {
	if b.HasMin && n < b.Min || b.HasMax && n > b.Max {
		r.Add(path, b.rule())
	}
}

func (b FloatBounds) rule() string {
	switch {
	case b.HasMin && b.HasMax:
		return fmt.Sprintf("must be from %g to %g", b.Min, b.Max)
	case b.HasMin:
		return fmt.Sprintf("must be at least %g", b.Min)
	default:
		return fmt.Sprintf("must be at most %g", b.Max)
	}
}

// EqInt checks that a number is the one allowed value.
func (r *Report) EqInt(path string, n, want int64) {
	if n != want {
		r.Add(path, fmt.Sprintf("must be %d", want))
	}
}

// EqUint checks that a number of an unsigned type is the one allowed value.
func (r *Report) EqUint(path string, n, want uint64) {
	if n != want {
		r.Add(path, fmt.Sprintf("must be %d", want))
	}
}

// EqString checks that a string is the one allowed value.
func (r *Report) EqString(path, s, want string) {
	if s != want {
		r.Add(path, "must be "+want)
	}
}

// OneOf checks that s is one of a closed set of strings.
func (r *Report) OneOf(path, s string, allowed ...string) {
	for _, one := range allowed {
		if s == one {
			return
		}
	}
	r.Add(path, "must be one of: "+join(allowed))
}

// Pattern checks that s matches re, a pattern that the contract names.
func (r *Report) Pattern(path, s string, re *regexp.Regexp) {
	if !re.MatchString(s) {
		r.Add(path, "must match "+re.String())
	}
}

// NoNUL checks that s holds no NUL.
func (r *Report) NoNUL(path, s string) {
	if strings.IndexByte(s, 0) >= 0 {
		r.Add(path, "contains NUL")
	}
}

// Unique checks that the elements of a slice appear once.
func Unique[T comparable](r *Report, path string, elements []T) {
	seen := make(map[T]struct{}, len(elements))
	for _, element := range elements {
		if _, ok := seen[element]; ok {
			r.Add(path, "has a duplicate element")
			return
		}
		seen[element] = struct{}{}
	}
}
