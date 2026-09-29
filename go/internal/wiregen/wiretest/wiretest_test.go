package wiretest

import (
	"encoding/json/v2"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/internal/wire"
)

// secret stands for a value that must never appear in an error.
const secret = "s3cr3t-token"

// failure decodes document as a T and returns its error's message, which is
// empty when the document is accepted.
func failure[T any](document string) string {
	if _, err := decode[T]([]byte(document)); err != nil {
		return err.Error()
	}
	return ""
}

func TestAStructureThatBreaksItsTypeIsRefusedAtTheFirstFailureNamingTheField(t *testing.T) {
	// The base of the documents: a Shapes that decodes, one member changed.
	base := map[string]string{
		"text": `"t"`, "flag": `true`, "small": `1`, "signed": `-1`, "native": `1`,
		"names": `["a"]`, "nested": `[[1],[2]]`, "leaf": `{"name":"n"}`, "leaves": `[{"name":"n"},{"name":"m"}]`,
		"byName": `{"k":{"name":"n"}}`, "shape": `{"kind":"circle","radius":1}`, "shapes": `[{"kind":"circle","radius":1},{"kind":"square","side":2}]`,
		"loose": `{"number":1}`, "span": `{"low":1,"high":2}`,
	}
	order := []string{"text", "flag", "small", "signed", "native", "names", "nested", "leaf", "leaves", "byName", "shape", "shapes", "loose", "span"}
	shapes := func(member, value string) string {
		var out []string
		for _, name := range order {
			v := base[name]
			if name == member {
				v = value
			}
			if v != "" {
				out = append(out, `"`+name+`":`+v)
			}
		}
		if _, known := base[member]; !known {
			out = append(out, `"`+member+`":`+value)
		}
		return "{" + strings.Join(out, ",") + "}"
	}
	if got := failure[Shapes](shapes("text", `"t"`)); got != "" {
		t.Fatalf("the base document was refused: %s", got)
	}
	for name, test := range map[string]struct{ document, want string }{
		"a member the type lacks":            {shapes("extra", `1`), "invalid: extra: unknown member"},
		"a member of a nested value":         {shapes("leaves", `[{"name":"n"},{"name":"m"},{"name":"o","extra":1}]`), "invalid: leaves[2].extra: unknown member"},
		"a required member missing":          {shapes("text", ""), "invalid: text: required"},
		"a required member of a nested one":  {shapes("leaf", `{}`), "invalid: leaf.name: required"},
		"a string of another kind":           {shapes("text", `1`), "invalid: text: must be a string"},
		"a null for a required member":       {shapes("text", `null`), "invalid: text: must be a string"},
		"a boolean of another kind":          {shapes("flag", `"true"`), "invalid: flag: must be a boolean"},
		"an integer over its type":           {shapes("small", `256`), "invalid: small: must be an unsigned integer of 8 bits"},
		"a negative for an unsigned integer": {shapes("small", `-1`), "invalid: small: must be an unsigned integer of 8 bits"},
		"a fraction for an integer":          {shapes("small", `1.5`), "invalid: small: must be an unsigned integer of 8 bits"},
		"a string for an integer":            {shapes("small", `"1"`), "invalid: small: must be a number"},
		"a signed integer over its type":     {shapes("signed", `9223372036854775808`), "invalid: signed: must be an integer of 64 bits"},
		"an element of another kind":         {shapes("names", `["a",1]`), "invalid: names[1]: must be a string"},
		"a nested element of another kind":   {shapes("nested", `[[1],[2,"x"]]`), "invalid: nested[1][1]: must be a number"},
		"an array for a string":              {shapes("text", `[]`), "invalid: text: must be a string"},
		"an object for an array":             {shapes("names", `{}`), "invalid: names: must be an array"},
		"an array for an object":             {shapes("leaf", `[]`), "invalid: leaf: must be an object"},
		"a member of a map's value":          {shapes("byName", `{"k":{"name":"n"},"other key":{}}`), `invalid: byName["other key"].name: required`},
		"an optional member that is null":    {shapes("maybe", `null`), "invalid: maybe: must not be null"},
		"an optional number of another kind": {shapes("maybeNum", `"x"`), "invalid: maybeNum: must be a number"},
		"an optional number that is null":    {shapes("maybeNum", `null`), "invalid: maybeNum: must not be null"},
		"a variant with an unknown tag":      {shapes("shape", `{"kind":"triangle"}`), "invalid: shape.kind: must be one of: circle, square"},
		"a variant without its tag":          {shapes("shape", `{"radius":1}`), "invalid: shape.kind: required"},
		"a tag that is not a string":         {shapes("shape", `{"kind":1}`), "invalid: shape.kind: must be a string"},
		"a member of another variant":        {shapes("shape", `{"kind":"circle","radius":1,"side":2}`), "invalid: shape.side: unknown member"},
		"a variant's member missing":         {shapes("shape", `{"kind":"square"}`), "invalid: shape.side: required"},
		"a union that is not an object":      {shapes("shape", `"circle"`), "invalid: shape: must be an object"},
		"a variant among several":            {shapes("shapes", `[{"kind":"circle","radius":1},{"kind":"oval"}]`), "invalid: shapes[1].kind: must be one of: circle, square"},
		"an object that fits no variant":     {shapes("loose", `{"number":1,"name":"n"}`), "invalid: loose: matches none of: Numbered, Named"},
		"a member of no variant":             {shapes("loose", `{}`), "invalid: loose: matches none of: Numbered, Named"},
		"a rule across fields":               {shapes("span", `{"low":2,"high":1}`), "invalid: span: low is above high"},
	} {
		if got := failure[Shapes](test.document); got != test.want {
			t.Errorf("%s: %q, want %q", name, got, test.want)
		}
	}
}

func TestADocumentThatIsNotOneValueOfItsTypeIsRefused(t *testing.T) {
	for name, test := range map[string]struct{ document, want string }{
		"a duplicate member":     {`{"name":"a","name":"b"}`, "invalid: name: has a duplicate member"},
		"an array":               {`[]`, "invalid: must be an object"},
		"a null":                 {`null`, "invalid: must be an object"},
		"a string":               {`"a"`, "invalid: must be an object"},
		"nothing":                {``, "invalid: is not valid JSON"},
		"a document cut short":   {`{"name":`, "invalid: name: is not valid JSON"},
		"text after the value":   {`{"name":"a"} x`, "invalid: is not valid JSON"},
		"a trailing comma":       {`{"name":"a",}`, "invalid: is not valid JSON"},
		"a second value":         {`{"name":"a"}{"name":"b"}`, "invalid: is not valid JSON"},
		"a member of the object": {`{"name":"a","extra":1}`, "invalid: extra: unknown member"},
		"no member":              {`{}`, "invalid: name: required"},
	} {
		if got := failure[Leaf](test.document); got != test.want {
			t.Errorf("%s: %q, want %q", name, got, test.want)
		}
	}
	if got := failure[Empty](`{"x":1}`); got != "invalid: x: unknown member" {
		t.Errorf("a member of a struct with none: %q", got)
	}
	if got := failure[Empty](`{}`); got != "" {
		t.Errorf("an empty struct: %q", got)
	}
}

func TestAStringThatIsNotUTF8IsRefusedNamingItsFieldOnBothSides(t *testing.T) {
	if got := failure[Leaf]("{\"name\":\"a\xff\"}"); got != "invalid: name: is not valid JSON" {
		t.Errorf("decoding: %q", got)
	}
	_, err := json.Marshal(Leaf{Name: "a\xff"}, wireOptions)
	if got := wire.Refusal(err).Error(); got != "invalid: name: is not valid JSON" {
		t.Errorf("encoding: %q", got)
	}
}

func TestAUnionAtTheTopOfADocumentIsDecodedByItsOwnDecoder(t *testing.T) {
	shape, err := decode[Shape]([]byte(`{"radius":2,"kind":"circle"}`))
	if err != nil || !reflect.DeepEqual(shape, Circle{Radius: 2}) {
		t.Errorf("shape = %#v, %v; the tag may come after the members", shape, err)
	}
	for name, test := range map[string]struct{ document, want string }{
		"a variant": {`{"kind":"square","side":1,"label":""}`, "invalid: label: must have at least 1 character"},
		"no tag":    {`{"side":1}`, "invalid: kind: required"},
	} {
		if got := failure[Shape](test.document); got != test.want {
			t.Errorf("%s: %q, want %q", name, got, test.want)
		}
	}
	if got := failure[Loose](`{"name":""}`); got != "invalid: name: must have at least 1 character" {
		t.Errorf("the rule of the variant that fits: %q", got)
	}
	if got := failure[Vague](`{}`); got != "invalid: matches more than one of: Yes, No" {
		t.Errorf("an object that fits several variants: %q", got)
	}
	if got := failure[Vague](`{"ok":true}`); got == "" {
		t.Error("an object that fits several variants was accepted")
	}
}

// rules returns the document of a Rules that breaks no rule, with the members
// of the changes replaced.
func rules(changes map[string]string) string {
	members := map[string]string{
		"chars": `"ab"`, "atLeast": `"ab"`, "atMost": `"abc"`, "bytes": `"a"`, "items": `["a"]`, "number": `1`, "big": `3`, "version": `7`,
		"word": `"hello"`, "kind": `"a"`, "mode": `"fast"`, "token": `"abc"`, "plain": `"a"`, "unique": `["a","b"]`, "checked": `"ab"`,
		"each": `["a","ab"]`, "table": `{"k":"v"}`, "modes": `{"fast":1}`, "object": `{}`,
	}
	for name, value := range changes {
		members[name] = value
	}
	var parts []string
	for _, name := range []string{"chars", "atLeast", "atMost", "bytes", "items", "number", "big", "version", "word", "kind", "mode", "token", "plain", "unique", "checked", "each", "table", "modes", "optional", "object"} {
		if value, ok := members[name]; ok && value != "" {
			parts = append(parts, `"`+name+`":`+value)
		}
	}
	return "{" + strings.Join(parts, ",") + "}"
}

func TestEveryRuleRefusesWhatItForbidsNamingTheFieldAndNeverTheValue(t *testing.T) {
	if got := failure[Rules](rules(nil)); got != "" {
		t.Fatalf("the base document was refused: %s", got)
	}
	long := `"` + secret + `"`
	for name, test := range map[string]struct {
		changes map[string]string
		want    string
	}{
		"chars, too few":                {map[string]string{"chars": `"s"`}, "invalid: chars: must have 2 to 4 characters"},
		"chars, too many":               {map[string]string{"chars": long}, "invalid: chars: must have 2 to 4 characters"},
		"chars, a lower bound":          {map[string]string{"atLeast": `"s"`}, "invalid: atLeast: must have at least 2 characters"},
		"chars, an upper bound":         {map[string]string{"atMost": long}, "invalid: atMost: must have at most 3 characters"},
		"bytes, too many":               {map[string]string{"bytes": `"éé"`}, "invalid: bytes: must have 1 to 3 bytes"},
		"items, none":                   {map[string]string{"items": `[]`}, "invalid: items: must have 1 to 2 items"},
		"items, too many":               {map[string]string{"items": `["a","b","c"]`}, "invalid: items: must have 1 to 2 items"},
		"range, below":                  {map[string]string{"number": `0`}, "invalid: number: must be from 1 to 10"},
		"range, above":                  {map[string]string{"number": `11`}, "invalid: number: must be from 1 to 10"},
		"range, over the type":          {map[string]string{"number": `200`}, "invalid: number: must be an integer of 8 bits"},
		"range, under the type":         {map[string]string{"number": `-129`}, "invalid: number: must be an integer of 8 bits"},
		"range, an upper bound":         {map[string]string{"big": `4`}, "invalid: big: must be at most 3"},
		"range, beyond every bound":     {map[string]string{"big": `18446744073709551615`}, "invalid: big: must be at most 3"},
		"eq, a number":                  {map[string]string{"version": `8`}, "invalid: version: must be 7"},
		"eq, a string":                  {map[string]string{"word": long}, "invalid: word: must be hello"},
		"oneof":                         {map[string]string{"kind": long}, "invalid: kind: must be one of: a, b"},
		"oneof, a named type":           {map[string]string{"mode": long}, "invalid: mode: must be one of: fast, slow"},
		"pattern":                       {map[string]string{"token": `"` + secret + `!"`}, "invalid: token: must match ^[a-z]+$"},
		"nonul":                         {map[string]string{"plain": `"` + secret + `\u0000"`}, "invalid: plain: contains NUL"},
		"unique":                        {map[string]string{"unique": `["` + secret + `","` + secret + `"]`}, "invalid: unique: has a duplicate element"},
		"func":                          {map[string]string{"checked": `"` + secret + ` "`}, "invalid: checked: contains a space"},
		"each, of a slice":              {map[string]string{"each": `["a","` + secret + `"]`}, "invalid: each[1]: must have 1 to 2 characters"},
		"each, of a map":                {map[string]string{"table": `{"k":"` + secret + `\u0000"}`}, `invalid: table["k"]: contains NUL`},
		"keys":                          {map[string]string{"table": `{"long-key":"v"}`}, `invalid: table["long-key"]: must have 1 to 3 characters`},
		"keys, a closed set":            {map[string]string{"modes": `{"medium":1}`}, `invalid: modes["medium"]: must be one of: fast, slow`},
		"an optional member with rules": {map[string]string{"optional": `"` + secret + `"`}, "invalid: optional: must have 1 to 2 characters"},
		"a function on raw JSON":        {map[string]string{"object": long}, "invalid: object: must be an object"},
	} {
		got := failure[Rules](rules(test.changes))
		if got != test.want {
			t.Errorf("%s: %q, want %q", name, got, test.want)
		}
		if strings.Contains(got, secret) {
			t.Errorf("%s: %q quotes the value", name, got)
		}
	}
}

func TestADecodedValueKeepsNothingOfItsInput(t *testing.T) {
	input := []byte(rules(map[string]string{"object": `{"a":1}`, "chars": `"ab"`}))
	value, err := decode[Rules](input)
	if err != nil {
		t.Fatal(err)
	}
	// The raw JSON of a field is copied out of the buffer that a decoder reads.
	var raw Rules
	if err := json.Unmarshal(input, &raw, wireOptions); err != nil {
		t.Fatal(err)
	}
	for i := range input {
		input[i] = 'x'
	}
	if string(raw.Object) != `{"a":1}` || raw.Chars != "ab" || string(value.Object) != `{"a":1}` {
		t.Errorf("the decoded value changed with its input: %s, %q", raw.Object, raw.Chars)
	}
}

func TestEveryBrokenRuleIsReportedInDeclarationOrder(t *testing.T) {
	got := failure[Rules](rules(map[string]string{"chars": `"s"`, "table": `{"a":"b","toolong":"c\u0000"}`, "kind": `"z"`}))
	// The keys of a map are checked before its values.
	want := strings.Join([]string{
		"invalid: chars: must have 2 to 4 characters",
		"invalid: kind: must be one of: a, b",
		`invalid: table["toolong"]: must have 1 to 3 characters`,
		`invalid: table["toolong"]: contains NUL`,
	}, "\n")
	if got != want {
		t.Errorf("errors:\n%s\nwant:\n%s", got, want)
	}
}

func TestALengthCountsUnicodeScalarValues(t *testing.T) {
	// One scalar value is two UTF-16 code units and four bytes.
	if got := failure[Rules](rules(map[string]string{"chars": `"😀😀😀😀"`})); got != "" {
		t.Errorf("four scalar values: %s", got)
	}
	if got := failure[Rules](rules(map[string]string{"chars": `"😀😀😀😀😀"`})); got == "" {
		t.Error("five scalar values were accepted")
	}
}

func TestAWireValueEncodesWhatItDecodesAndChecksItsRulesFirst(t *testing.T) {
	label := "l"
	number := 4
	shapes := Shapes{
		Text: "t", Flag: true, Small: 1, Signed: -1, Native: 1,
		Names: []string{"a"}, Nested: [][]int{{1}, {2}}, Leaf: Leaf{Name: "n"}, Leaves: []Leaf{{Name: "n"}},
		ByName: map[string]Leaf{"b": {Name: "n"}, "a": {Name: "m"}},
		Maybe:  &Leaf{Name: "n"}, MaybeNum: &number,
		Shape: Square{Side: 2, Label: &label}, Shapes: []Shape{Circle{Radius: 1}, &Circle{Radius: 2}},
		Loose: Named{Name: "n"}, Span: Span{Low: 1, High: 2},
	}
	data, err := json.Marshal(shapes, wireOptions)
	if err != nil {
		t.Fatal(err)
	}
	const want = `{"text":"t","flag":true,"small":1,"signed":-1,"native":1,"names":["a"],"nested":[[1],[2]],"leaf":{"name":"n"},"leaves":[{"name":"n"}],` +
		`"byName":{"a":{"name":"m"},"b":{"name":"n"}},"maybe":{"name":"n"},"maybeNum":4,"shape":{"kind":"square","side":2,"label":"l"},` +
		`"shapes":[{"kind":"circle","radius":1},{"kind":"circle","radius":2}],"loose":{"name":"n"},"span":{"low":1,"high":2}}`
	if string(data) != want {
		t.Errorf("encoded:\n%s\nwant:\n%s", data, want)
	}
	back, err := decode[Shapes](data)
	if err != nil {
		t.Fatal(err)
	}
	// A pointer to a variant decodes as the variant itself.
	shapes.Shapes[1] = Circle{Radius: 2}
	if !reflect.DeepEqual(back, shapes) {
		t.Errorf("decoded %#v, want %#v", back, shapes)
	}
	for name, value := range map[string]interface{ validate() error }{
		"an unset union":        Shapes{Text: "t", Leaf: Leaf{Name: "n"}, Loose: Named{Name: "n"}},
		"a nil pointer variant": Shapes{Shape: (*Circle)(nil)},
	} {
		if value.validate() == nil {
			t.Errorf("%s was accepted", name)
		}
	}
	if got := (Shapes{Loose: Named{Name: "n"}, Leaf: Leaf{Name: "n"}}).validate().Error(); got != "invalid: shape: required" {
		t.Errorf("an unset union: %q", got)
	}
}

func TestTheGeneratedHelpersRefuseWhatIsNotAWireValue(t *testing.T) {
	var leaf *Leaf
	if _, err := encode(leaf); err == nil || err.Error() != "invalid: required" {
		t.Errorf("encoding a nil pointer: %v", err)
	}
	if _, err := encode[Shape](nil); err == nil || err.Error() != "invalid: required" {
		t.Errorf("encoding a nil union: %v", err)
	}
	if _, err := decode[map[string]string]([]byte(`{}`)); err == nil || err.Error() != "wiretest: map[string]string is not a type of the wire" {
		t.Errorf("decoding a type that is not the wire's: %v", err)
	}
	if data, err := encode(Leaf{Name: "n"}); err != nil || string(data) != `{"name":"n"}` {
		t.Errorf("encoding a leaf: %s, %v", data, err)
	}
}
