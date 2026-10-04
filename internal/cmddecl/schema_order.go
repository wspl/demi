package cmddecl

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"net/url"
	"slices"
	"strconv"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/santhosh-tekuri/jsonschema/v6/kind"
	"golang.org/x/text/message"

	"github.com/wspl/demi/internal/contract"
)

// documentOrder records each object's member names in document order, by JSON pointer, for diagnostic order.
func documentOrder(document []byte) (map[string][]string, error) {
	decoder := json.NewDecoder(bytes.NewReader(document))
	decoder.UseNumber()
	order := map[string][]string{}
	var read func(string) error
	read = func(path string) error {
		token, err := decoder.Token()
		if err != nil {
			return err
		}
		switch token {
		case json.Delim('{'):
			order[path] = []string{}
			for decoder.More() {
				token, err := decoder.Token()
				if err != nil {
					return err
				}
				name, _ := token.(string)
				order[path] = append(order[path], name)
				if err := read(path + "/" + pointerEscape(name)); err != nil {
					return err
				}
			}
			_, err = decoder.Token()
			return err
		case json.Delim('['):
			for i := 0; decoder.More(); i++ {
				if err := read(path + "/" + strconv.Itoa(i)); err != nil {
					return err
				}
			}
			_, err = decoder.Token()
			return err
		}
		return nil
	}
	return order, read("")
}

// keys reads the original order of a schema object's members.
func (s *Schema) keys(path string) []string {
	return s.order["/"+path]
}

// pointerEscape addresses schema properties, including names containing slash or tilde.
func pointerEscape(text string) string {
	return strings.ReplaceAll(strings.ReplaceAll(text, "~", "~0"), "/", "~1")
}

// exhaustiveSchemas prevents the library's type/const/enum/format fast failures
// from hiding the schema's other diagnostics: every failing keyword reports. Each assertion still uses the
// library's compiled validator. All changes occur before the schema is published.
func exhaustiveSchemas(root *jsonschema.Schema) {
	seen := map[*jsonschema.Schema]bool{}
	var visit func(*jsonschema.Schema)
	visit = func(s *jsonschema.Schema) {
		if s == nil || seen[s] {
			return
		}
		seen[s] = true
		if s.DraftVersion >= 2019 {
			s.ContentEncoding = nil
			s.ContentMediaType = nil
			s.ContentSchema = nil
		}
		children := schemaChildren(s)
		for _, child := range children {
			visit(child)
		}
		separateSchemaAssertions(s)
	}
	visit(root)
}

// keywordPriority ranks a keyword's diagnostics: a keyword earlier in the list reports first.
func keywordPriority(keyword string) int {
	keywords := []string{
		"type",
		"const",
		"enum",
		"minimum",
		"maximum",
		"exclusiveMinimum",
		"exclusiveMaximum",
		"multipleOf",
		"minLength",
		"maxLength",
		"minItems",
		"maxItems",
		"minProperties",
		"maxProperties",
		"required",
		"dependentRequired",
		"pattern",
		"format",
		"contentEncoding",
		"contentMediaType",
		"contentSchema",
		"uniqueItems",
		"properties",
		"patternProperties",
		"additionalProperties",
		"propertyNames",
		"items",
		"prefixItems",
		"additionalItems",
		"contains",
		"dependencies",
		"dependentSchemas",
		"allOf",
		"anyOf",
		"oneOf",
		"not",
		"if",
		"unevaluatedProperties",
		"unevaluatedItems",
		"$ref",
		"$recursiveRef",
		"$dynamicRef",
	}
	index := slices.Index(keywords, keyword)
	if index < 0 {
		return len(keywords)
	}
	return index
}

// schemaPath resolves the validator's location to original document tokens.
func schemaPath(location string) []string {
	_, fragment, _ := strings.Cut(location, "#")
	fragment, err := url.PathUnescape(fragment)
	if err != nil || fragment == "" {
		return nil
	}
	parts := strings.Split(strings.TrimPrefix(fragment, "/"), "/")
	for i, part := range parts {
		parts[i] = strings.ReplaceAll(strings.ReplaceAll(part, "~1", "/"), "~0", "~")
	}
	return parts
}

// schemaAt obtains the original keyword value for diagnostics such as `not`.
func (s *Schema) schemaAt(path []string) any {
	var value any = s.object
	for _, part := range path {
		switch object := value.(type) {
		case map[string]any:
			value = object[part]
		case []any:
			index, err := strconv.Atoi(part)
			if err != nil || index < 0 || index >= len(object) {
				return nil
			}
			value = object[index]
		default:
			return nil
		}
	}
	return value
}

// diagnosticRank builds a failure's sort key: at each schema step the keyword's priority,
// then the member's position in the schema or the instance.
func (s *Schema) diagnosticRank(failure *jsonschema.ValidationError, order map[string][]string) string {
	path := append(schemaPath(failure.SchemaURL), failure.ErrorKind.KeywordPath()...)
	if reason, ok := failure.ErrorKind.(*kind.Dependency); ok {
		path = append(schemaPath(failure.SchemaURL), "dependencies", reason.Prop)
	}
	traversal := diagnosticTraversal{failure: failure, order: order}
	for i := 0; i < len(path); i++ {
		keyword := path[i]
		parent, _ := s.schemaAt(path[:i]).(map[string]any)
		fused := writeKeywordRank(&traversal.rank, keyword, parent)
		traversal.writeAdditionalRank(keyword, path, i)
		traversal.schemaPrefix += "/" + pointerEscape(keyword)
		if (keyword == "properties" || keyword == "patternProperties" ||
			keyword == "dependentSchemas" || keyword == "dependentRequired" || keyword == "dependencies" ||
			keyword == "allOf" || keyword == "anyOf" || keyword == "oneOf" || keyword == "prefixItems") &&
			i+1 < len(path) {
			i++
			traversal.writeMemberRank(s, keyword, path[i], fused)
		} else if keyword == "items" &&
			traversal.instanceIndex < len(failure.InstanceLocation) {
			index, _ := strconv.Atoi(failure.InstanceLocation[traversal.instanceIndex])
			fmt.Fprintf(&traversal.rank, "%09d/", index)
			traversal.instancePrefix += "/" + failure.InstanceLocation[traversal.instanceIndex]
			traversal.instanceIndex++
		}
	}
	return traversal.rank.String()
}

// unevaluatedFailure combines the library's per-member errors of one unevaluated keyword into one diagnostic.
type unevaluatedFailure struct {
	items bool
	names []string
}

// KeywordPath identifies the aggregated unevaluated keyword.
func (e *unevaluatedFailure) KeywordPath() []string {
	if e.items {
		return []string{"unevaluatedItems"}
	}
	return []string{"unevaluatedProperties"}
}

// LocalizedString is empty because command diagnostics render this aggregate themselves.
func (*unevaluatedFailure) LocalizedString(*message.Printer) string {
	return ""
}

// aggregateFailures keeps one contains failure per schema and instance location, and one
// unevaluated failure per keyword and parent instance that lists every member.
func aggregateFailures(failures []*jsonschema.ValidationError) []*jsonschema.ValidationError {
	groups := map[string]*jsonschema.ValidationError{}
	contains := map[string]bool{}
	var result []*jsonschema.ValidationError
	for _, failure := range failures {
		switch failure.ErrorKind.(type) {
		case *kind.Contains, *kind.MinContains, *kind.MaxContains:
			key := fmt.Sprintf("%s/%q", failure.SchemaURL, failure.InstanceLocation)
			if contains[key] {
				continue
			}
			contains[key] = true
		}
		path := schemaPath(failure.SchemaURL)
		index := slices.Index(path, "unevaluatedProperties")
		items := false
		if index < 0 {
			index = slices.Index(path, "unevaluatedItems")
			items = true
		}
		if index < 0 || len(failure.InstanceLocation) == 0 {
			result = append(result, failure)
			continue
		}
		nameIndex := unevaluatedNameIndex(path, index, failure.InstanceLocation)
		parent := failure.InstanceLocation[:nameIndex]
		key := fmt.Sprintf("%q/%q", path[:index+1], parent)
		group := groups[key]
		if group == nil {
			group = &jsonschema.ValidationError{
				SchemaURL: strings.Split(failure.SchemaURL, "#")[0] + "#" + strings.TrimSuffix(
					"/"+strings.Join(path[:index], "/"),
					"/",
				),
				InstanceLocation: slices.Clone(parent),
				ErrorKind:        &unevaluatedFailure{items: items},
			}
			groups[key] = group
			result = append(result, group)
		}
		reason, _ := group.ErrorKind.(*unevaluatedFailure)
		name := failure.InstanceLocation[nameIndex]
		if !slices.Contains(reason.names, name) {
			reason.names = append(reason.names, name)
		}
	}
	return result
}

// literal renders a schema value as compact JSON in document order: an integer as written,
// another number as the shortest float with a .0 or, below 1e-5 or from 1e16, a signed exponent.
func (s *Schema) literal(path []string) string {
	value := s.schemaAt(path)
	prefix := ""
	for _, part := range path {
		prefix += "/" + pointerEscape(part)
	}
	switch value := value.(type) {
	case map[string]any:
		parts := make([]string, 0, len(value))
		for _, name := range s.order[prefix] {
			// Schema keys have already been validated as JSON.
			encoded, _ := contract.EncodeJSON(name)
			parts = append(parts, string(encoded)+":"+s.literal(append(slices.Clone(path), name)))
		}
		return "{" + strings.Join(parts, ",") + "}"
	case []any:
		parts := make([]string, len(value))
		for i := range value {
			parts[i] = s.literal(append(slices.Clone(path), strconv.Itoa(i)))
		}
		return "[" + strings.Join(parts, ",") + "]"
	case json.Number:
		text := string(value)
		if !strings.ContainsAny(text, ".eE") && text != "-0" {
			if _, err := strconv.ParseInt(text, 10, 64); err == nil {
				return text
			}
			if _, err := strconv.ParseUint(text, 10, 64); err == nil {
				return text
			}
		}
		number, _ := value.Float64() // The compiled document contains a valid JSON number.
		format := byte('f')
		if abs := math.Abs(number); abs >= 1e16 || abs != 0 && abs < 1e-5 {
			format = 'e'
		}
		text = strconv.FormatFloat(number, format, -1, 64)
		if mantissa, exponent, ok := strings.Cut(text, "e"); ok {
			power, _ := strconv.Atoi(exponent)
			return mantissa + "e" + fmt.Sprintf("%+d", power)
		}
		if !strings.Contains(text, ".") {
			text += ".0"
		}
		return text
	default:
		// A scalar in a compiled schema is always JSON-encodable.
		encoded, _ := contract.EncodeJSON(value)
		return string(encoded)
	}
}

// schemaChildren enumerates the compiled subschemas before assertions are separated.
func schemaChildren(s *jsonschema.Schema) []*jsonschema.Schema {
	children := []*jsonschema.Schema{
		s.Ref,
		s.RecursiveRef,
		s.Not,
		s.If,
		s.Then,
		s.Else,
		s.PropertyNames,
		s.UnevaluatedProperties,
		s.Contains,
		s.Items2020,
		s.UnevaluatedItems,
		s.ContentSchema,
	}
	if s.DynamicRef != nil {
		children = append(children, s.DynamicRef.Ref)
	}
	for _, list := range [][]*jsonschema.Schema{s.AllOf, s.AnyOf, s.OneOf, s.PrefixItems} {
		children = append(children, list...)
	}
	for _, child := range s.Properties {
		children = append(children, child)
	}
	for _, child := range s.PatternProperties {
		children = append(children, child)
	}
	for _, child := range s.DependentSchemas {
		children = append(children, child)
	}
	for _, child := range s.Dependencies {
		if sub, ok := child.(*jsonschema.Schema); ok {
			children = append(children, sub)
		}
	}
	for _, child := range []any{s.Items, s.AdditionalItems, s.AdditionalProperties} {
		switch child := child.(type) {
		case *jsonschema.Schema:
			children = append(children, child)
		case []*jsonschema.Schema:
			children = append(children, child...)
		}
	}
	return children
}

// separateSchemaAssertions moves type, const, enum and format into allOf subschemas, so each failure is collected.
func separateSchemaAssertions(s *jsonschema.Schema) {
	if s.Types != nil {
		s.AllOf = append(
			s.AllOf,
			&jsonschema.Schema{Location: s.Location, Types: s.Types, DraftVersion: s.DraftVersion},
		)
		s.Types = nil
	}
	if s.Const != nil {
		s.AllOf = append(
			s.AllOf,
			&jsonschema.Schema{Location: s.Location, Const: s.Const, DraftVersion: s.DraftVersion},
		)
		s.Const = nil
	}
	if s.Enum != nil {
		s.AllOf = append(
			s.AllOf,
			&jsonschema.Schema{Location: s.Location, Enum: s.Enum, DraftVersion: s.DraftVersion},
		)
		s.Enum = nil
	}
	if s.Format != nil {
		s.AllOf = append(
			s.AllOf,
			&jsonschema.Schema{Location: s.Location, Format: s.Format, DraftVersion: s.DraftVersion},
		)
		s.Format = nil
	}
}

// unevaluatedNameIndex locates the member owned by the unevaluated keyword.
func unevaluatedNameIndex(path []string, index int, location []string) int {
	descents := 0
	for j := index + 1; j < len(path); j++ {
		switch path[j] {
		case "properties", "patternProperties", "prefixItems":
			descents++
			j++
		case "items", "additionalProperties", "additionalItems", "unevaluatedProperties", "unevaluatedItems":
			descents++
		}
	}
	nameIndex := len(location) - 1 - descents
	if nameIndex < 0 {
		nameIndex = 0
	}
	return nameIndex
}

// writeKeywordRank writes a keyword's priority. When additionalProperties is present and
// not true, properties and patternProperties rank with it. A one-field required, with
// additionalProperties false, properties and no patternProperties, ranks after every
// additionalProperties diagnostic. A two-field required, with fewer than 15 properties,
// no patternProperties and additionalProperties absent or true, ranks before the properties' diagnostics.
func writeKeywordRank(rank *strings.Builder, keyword string, parent map[string]any) bool {
	priority := keywordPriority(keyword)
	additional, hasAdditional := parent["additionalProperties"]
	fused := hasAdditional && additional != true
	required, _ := parent["required"].([]any)
	properties, hasProperties := parent["properties"].(map[string]any)
	_, additionalSchema := additional.(map[string]any)
	fusedRequired1 := len(required) == 1 && additional == false && parent["patternProperties"] == nil &&
		hasProperties
	fusedRequired2 := len(required) == 2 && hasProperties && len(properties) < 15 && additional != false &&
		!additionalSchema &&
		parent["patternProperties"] == nil
	if fused && (keyword == "properties" || keyword == "patternProperties") ||
		keyword == "required" && fusedRequired1 {
		priority = keywordPriority("additionalProperties")
	}
	if keyword == "required" && fusedRequired2 {
		priority = keywordPriority("properties")
	}
	fmt.Fprintf(rank, "%03d/", priority)
	if keyword == "required" && fusedRequired1 {
		rank.WriteString("999999999/")
	}
	if keyword == "required" && fusedRequired2 {
		rank.WriteString("-/")
	}
	return fused
}

// diagnosticTraversal bundles the shared arguments and position of diagnostic ordering steps.
type diagnosticTraversal struct {
	failure        *jsonschema.ValidationError
	order          map[string][]string
	rank           strings.Builder
	schemaPrefix   string
	instancePrefix string
	instanceIndex  int
}

func (t *diagnosticTraversal) writeAdditionalRank(keyword string, path []string, i int) {
	if keyword == "additionalProperties" && len(path) == i+1 {
		if _, ok := t.failure.ErrorKind.(*kind.AdditionalProperties); ok {
			t.rank.WriteString("999999998/")
		} else if t.instanceIndex < len(t.failure.InstanceLocation) {
			fmt.Fprintf(
				&t.rank,
				"%09d/",
				slices.Index(t.order[t.instancePrefix], t.failure.InstanceLocation[t.instanceIndex]),
			)
			t.instancePrefix += "/" + pointerEscape(t.failure.InstanceLocation[t.instanceIndex])
			t.instanceIndex++
		}
	}
	if keyword == "additionalProperties" && len(path) > i+1 && t.instanceIndex < len(t.failure.InstanceLocation) {
		fmt.Fprintf(
			&t.rank,
			"%09d/",
			slices.Index(t.order[t.instancePrefix], t.failure.InstanceLocation[t.instanceIndex]),
		)
		t.instancePrefix += "/" + pointerEscape(t.failure.InstanceLocation[t.instanceIndex])
		t.instanceIndex++
	}
	if keyword == "propertyNames" {
		if reason, ok := t.failure.ErrorKind.(*kind.PropertyNames); ok {
			fmt.Fprintf(&t.rank, "%09d/", slices.Index(t.order[t.instancePrefix], reason.Property))
		}
	}
}

func (t *diagnosticTraversal) writeMemberRank(s *Schema, keyword, member string, fused bool) {
	if keyword == "prefixItems" && t.instanceIndex < len(t.failure.InstanceLocation) {
		t.instancePrefix += "/" + t.failure.InstanceLocation[t.instanceIndex]
		t.instanceIndex++
	}
	index := slices.Index(s.order[t.schemaPrefix], member)
	if keyword == "properties" && t.instanceIndex < len(t.failure.InstanceLocation) {
		// Rank by the instance's member order when the table is fused, has 15 or more properties,
		// or the instance has no more members than the schema; otherwise by the schema's order.
		if fused || len(s.order[t.schemaPrefix]) >= 15 ||
			len(t.order[t.instancePrefix]) <= len(s.order[t.schemaPrefix]) {
			index = slices.Index(t.order[t.instancePrefix], t.failure.InstanceLocation[t.instanceIndex])
		}
		t.instancePrefix += "/" + pointerEscape(t.failure.InstanceLocation[t.instanceIndex])
		t.instanceIndex++
	}
	if index < 0 {
		index, _ = strconv.Atoi(member)
	}
	if keyword == "patternProperties" && t.instanceIndex < len(t.failure.InstanceLocation) {
		fieldIndex := slices.Index(t.order[t.instancePrefix], t.failure.InstanceLocation[t.instanceIndex])
		if fused {
			fmt.Fprintf(&t.rank, "%09d/000000001/%09d/", fieldIndex, index)
		} else {
			fmt.Fprintf(&t.rank, "%09d/%09d/", index, fieldIndex)
		}
		t.instancePrefix += "/" + pointerEscape(t.failure.InstanceLocation[t.instanceIndex])
		t.instanceIndex++
	} else {
		fmt.Fprintf(&t.rank, "%09d/", index)
		if keyword == "properties" && fused {
			t.rank.WriteString("000000000/")
		}
	}
	t.schemaPrefix += "/" + pointerEscape(member)
}
