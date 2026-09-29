package commandtree

import (
	"cmp"
	"fmt"
	"math"
	"math/big"
	"slices"
	"strconv"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6/kind"
)

// joinFailures joins the failures of one check into the one text the model
// reads.
func joinFailures(messages []string) string {
	return strings.Join(messages, "; ")
}

// subject names the value a failure is about: the path to it, in quotes, and
// "value" for the instance itself. It never repeats the value, which may be a
// whole stdin body.
func (f failure) subject() string {
	if f.literal != "" {
		return f.literal
	}
	if len(f.location) == 0 {
		return "value"
	}
	return `"` + strings.Join(f.location, ".") + `"`
}

// message words a failure from the Rust implementation's templates: the wording
// is what the model reads, and it is pinned by a table of the Rust's messages
// (testdata/rust/wording.json).
func (f failure) message(document Object) string {
	subject := f.subject()
	switch k := f.kind.(type) {
	case *kind.Type:
		return typeMessage(subject, k.Want)
	case *kind.Enum:
		return enumMessage(subject, enumOptions(f, document, k))
	case *kind.Const:
		return constant(f, document, k) + " was expected"
	case *kind.Minimum:
		return subject + " is less than the minimum of " + limit(k.Want)
	case *kind.Maximum:
		return subject + " is greater than the maximum of " + limit(k.Want)
	case *kind.ExclusiveMinimum:
		return subject + " is less than or equal to the minimum of " + limit(k.Want)
	case *kind.ExclusiveMaximum:
		return subject + " is greater than or equal to the maximum of " + limit(k.Want)
	case *kind.MultipleOf:
		want, _ := k.Want.Float64()
		return subject + " is not a multiple of " + strconv.FormatFloat(want, 'f', -1, 64)
	case *kind.MinLength:
		return fmt.Sprintf("%s is shorter than %d character%s", subject, k.Want, plural(k.Want, "", "s"))
	case *kind.MaxLength:
		return fmt.Sprintf("%s is longer than %d character%s", subject, k.Want, plural(k.Want, "", "s"))
	case *kind.MinItems:
		return fmt.Sprintf("%s has less than %d item%s", subject, k.Want, plural(k.Want, "", "s"))
	case *kind.MaxItems:
		return fmt.Sprintf("%s has more than %d item%s", subject, k.Want, plural(k.Want, "", "s"))
	case *kind.MinProperties:
		return fmt.Sprintf("%s has less than %d propert%s", subject, k.Want, plural(k.Want, "y", "ies"))
	case *kind.MaxProperties:
		return fmt.Sprintf("%s has more than %d propert%s", subject, k.Want, plural(k.Want, "y", "ies"))
	case *kind.Required, *kind.DependentRequired, *kind.Dependency:
		return encodeValue(f.name) + " is a required property"
	case *kind.AdditionalProperties:
		return additionalMessage(f.unexpected)
	case *kind.Pattern:
		return subject + " does not match \"" + k.Want + "\""
	case *kind.UniqueItems:
		return subject + " has non-unique elements"
	case *kind.Contains, *kind.MinContains:
		return "None of " + subject + " are valid under the given schema"
	case *kind.Format:
		return subject + ` is not a "` + k.Want + `"`
	case *kind.AnyOf:
		return subject + " is not valid under any of the schemas listed in the 'anyOf' keyword"
	case *kind.OneOf:
		if len(k.Subschemas) == 0 {
			return subject + " is not valid under any of the schemas listed in the 'oneOf' keyword"
		}
		return subject + " is valid under more than one of the schemas listed in the 'oneOf' keyword"
	case *kind.Not:
		return encodeValue(lookup(document, f.pointer)) + " is not allowed for " + subject
	case *kind.FalseSchema:
		return "False schema does not allow " + subject
	case *kind.PropertyNames:
		// The Rust words the failure of the name as it words any, with the
		// name for the value.
		if f.inner != nil {
			return f.inner.message(document)
		}
	}
	keyword := "schema"
	if len(f.pointer) > 0 {
		keyword = f.pointer[len(f.pointer)-1]
	}
	return subject + " does not satisfy the " + strconv.Quote(keyword) + " keyword"
}

// plural returns one when n is 1, and many otherwise.
func plural(n int, one, many string) string {
	if n == 1 {
		return one
	}
	return many
}

// typeRank is the order in which the Rust lists the types a schema allows.
var typeRank = map[string]int{"null": 0, "boolean": 1, "integer": 2, "number": 3, "string": 4, "array": 5, "object": 6}

// typeMessage words a value of a type that a schema does not list. The types
// come in the order of typeRank.
func typeMessage(subject string, want []string) string {
	types := slices.Clone(want)
	slices.SortFunc(types, func(a, b string) int { return cmp.Compare(typeRank[a], typeRank[b]) })
	types = slices.Compact(types)
	if len(types) == 1 {
		return subject + ` is not of type "` + types[0] + `"`
	}
	quoted := make([]string, len(types))
	for i, name := range types {
		quoted[i] = `"` + name + `"`
	}
	return subject + " is not of types " + strings.Join(quoted, ", ")
}

// enumOptions returns the values of an enum as the schema wrote them.
func enumOptions(f failure, document Object, k *kind.Enum) []string {
	if written, ok := lookup(document, f.pointer).([]any); ok {
		options := make([]string, len(written))
		for i, option := range written {
			options[i] = encodeValue(option)
		}
		return options
	}
	options := make([]string, len(k.Want))
	for i, option := range k.Want {
		options[i] = encodeValue(option)
	}
	return options
}

// maxEnumOptions is how many of an enum's options a message shows.
const maxEnumOptions = 3

// enumMessage words a value outside an enum: all of its options up to three,
// and the first two and the count of the others beyond that.
func enumMessage(subject string, options []string) string {
	message := subject + " is not one of "
	if len(options) <= maxEnumOptions {
		for i, option := range options {
			switch {
			case i == 0:
				message += option
			case i == len(options)-1:
				message += " or " + option
			default:
				message += ", " + option
			}
		}
		return message
	}
	shown := maxEnumOptions - 1
	message += strings.Join(options[:shown], ", ")
	return fmt.Sprintf("%s or %d other candidates", message, len(options)-shown)
}

// constant returns the value that a const keyword allows, as the schema wrote
// it.
func constant(f failure, document Object, k *kind.Const) string {
	if written := lookup(document, f.pointer); written != nil {
		return encodeValue(written)
	}
	return encodeValue(k.Want)
}

// additionalMessage words the properties that a schema refuses.
func additionalMessage(unexpected []string) string {
	quoted := make([]string, len(unexpected))
	for i, name := range unexpected {
		quoted[i] = "'" + name + "'"
	}
	suffix := " were unexpected)"
	if len(unexpected) == 1 {
		suffix = " was unexpected)"
	}
	return "Additional properties are not allowed (" + strings.Join(quoted, ", ") + suffix
}

// limit words a number that a schema bounds a value by, as JSON writes it.
func limit(number *big.Rat) string {
	if number.IsInt() {
		return number.Num().String()
	}
	value, _ := number.Float64()
	return shortest(value)
}

// shortest writes a finite float as serde_json does: the shortest digits that
// read back as the same value, in scientific notation only when the exponent
// is below -5 or above 15.
func shortest(value float64) string {
	if value == math.Trunc(value) && math.Abs(value) < 1e16 {
		return strconv.FormatFloat(value, 'f', 1, 64)
	}
	scientific := strconv.FormatFloat(value, 'e', -1, 64)
	mantissa, exponentText, _ := strings.Cut(scientific, "e")
	exponent, _ := strconv.Atoi(exponentText)
	if exponent >= -5 && exponent <= 15 {
		return strconv.FormatFloat(value, 'f', -1, 64)
	}
	return mantissa + "e" + strconv.Itoa(exponent)
}
