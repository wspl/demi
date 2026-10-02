package declare

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
)

// documentOrder retains the object-member order used by the Rust schema and instance walkers.
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
func (s *Schema) keys(path string) []string { return s.order["/"+path] }

// pointerEscape addresses schema properties, including names containing slash or tilde.
func pointerEscape(text string) string {
	return strings.ReplaceAll(strings.ReplaceAll(text, "~", "~0"), "/", "~1")
}

// exhaustiveSchemas prevents the library's type/const/enum/format fast failures
// from hiding other Rust iter_errors diagnostics. Each assertion still uses the
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
		children := []*jsonschema.Schema{s.Ref, s.RecursiveRef, s.Not, s.If, s.Then, s.Else, s.PropertyNames, s.UnevaluatedProperties, s.Contains, s.Items2020, s.UnevaluatedItems, s.ContentSchema}
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
		for _, child := range children {
			visit(child)
		}
		if s.Types != nil {
			s.AllOf = append(s.AllOf, &jsonschema.Schema{Location: s.Location, Types: s.Types, DraftVersion: s.DraftVersion})
			s.Types = nil
		}
		if s.Const != nil {
			s.AllOf = append(s.AllOf, &jsonschema.Schema{Location: s.Location, Const: s.Const, DraftVersion: s.DraftVersion})
			s.Const = nil
		}
		if s.Enum != nil {
			s.AllOf = append(s.AllOf, &jsonschema.Schema{Location: s.Location, Enum: s.Enum, DraftVersion: s.DraftVersion})
			s.Enum = nil
		}
		if s.Format != nil {
			s.AllOf = append(s.AllOf, &jsonschema.Schema{Location: s.Location, Format: s.Format, DraftVersion: s.DraftVersion})
			s.Format = nil
		}
	}
	visit(root)
}

// keywordPriority mirrors jsonschema 0.56's diagnostic traversal order.
func keywordPriority(keyword string) int {
	keywords := []string{"type", "const", "enum", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf", "minLength", "maxLength", "minItems", "maxItems", "minProperties", "maxProperties", "required", "dependentRequired", "pattern", "format", "contentEncoding", "contentMediaType", "contentSchema", "uniqueItems", "properties", "patternProperties", "additionalProperties", "propertyNames", "items", "prefixItems", "additionalItems", "contains", "dependencies", "dependentSchemas", "allOf", "anyOf", "oneOf", "not", "if", "unevaluatedProperties", "unevaluatedItems", "$ref", "$recursiveRef", "$dynamicRef"}
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

// diagnosticRank orders schema errors by Rust's keyword priority and input traversal.
func (s *Schema) diagnosticRank(failure *jsonschema.ValidationError, order map[string][]string) string {
	path := append(schemaPath(failure.SchemaURL), failure.ErrorKind.KeywordPath()...)
	if reason, ok := failure.ErrorKind.(*kind.Dependency); ok {
		path = append(schemaPath(failure.SchemaURL), "dependencies", reason.Prop)
	}
	var rank strings.Builder
	schemaPrefix := ""
	instancePrefix := ""
	instanceIndex := 0
	for i := 0; i < len(path); i++ {
		keyword := path[i]
		priority := keywordPriority(keyword)
		parent, _ := s.schemaAt(path[:i]).(map[string]any)
		additional, hasAdditional := parent["additionalProperties"]
		fused := hasAdditional && additional != true
		required, _ := parent["required"].([]any)
		properties, hasProperties := parent["properties"].(map[string]any)
		_, additionalSchema := additional.(map[string]any)
		fusedRequired1 := len(required) == 1 && additional == false && parent["patternProperties"] == nil && hasProperties
		fusedRequired2 := len(required) == 2 && hasProperties && len(properties) < 15 && additional != false && !additionalSchema && parent["patternProperties"] == nil
		if fused && (keyword == "properties" || keyword == "patternProperties") || keyword == "required" && fusedRequired1 {
			priority = keywordPriority("additionalProperties")
		}
		if keyword == "required" && fusedRequired2 {
			priority = keywordPriority("properties")
		}
		fmt.Fprintf(&rank, "%03d/", priority)
		if keyword == "required" && fusedRequired1 {
			rank.WriteString("999999999/")
		}
		if keyword == "required" && fusedRequired2 {
			rank.WriteString("-/")
		}
		if keyword == "additionalProperties" && len(path) == i+1 {
			if _, ok := failure.ErrorKind.(*kind.AdditionalProperties); ok {
				rank.WriteString("999999998/")
			} else if instanceIndex < len(failure.InstanceLocation) {
				fmt.Fprintf(&rank, "%09d/", slices.Index(order[instancePrefix], failure.InstanceLocation[instanceIndex]))
				instancePrefix += "/" + pointerEscape(failure.InstanceLocation[instanceIndex])
				instanceIndex++
			}
		}
		if keyword == "additionalProperties" && len(path) > i+1 && instanceIndex < len(failure.InstanceLocation) {
			fmt.Fprintf(&rank, "%09d/", slices.Index(order[instancePrefix], failure.InstanceLocation[instanceIndex]))
			instancePrefix += "/" + pointerEscape(failure.InstanceLocation[instanceIndex])
			instanceIndex++
		}
		if keyword == "propertyNames" {
			if reason, ok := failure.ErrorKind.(*kind.PropertyNames); ok {
				fmt.Fprintf(&rank, "%09d/", slices.Index(order[instancePrefix], reason.Property))
			}
		}
		schemaPrefix += "/" + pointerEscape(keyword)
		if (keyword == "properties" || keyword == "patternProperties" || keyword == "dependentSchemas" || keyword == "dependentRequired" || keyword == "dependencies" || keyword == "allOf" || keyword == "anyOf" || keyword == "oneOf" || keyword == "prefixItems") && i+1 < len(path) {
			i++
			member := path[i]
			if keyword == "prefixItems" && instanceIndex < len(failure.InstanceLocation) {
				instancePrefix += "/" + failure.InstanceLocation[instanceIndex]
				instanceIndex++
			}
			index := slices.Index(s.order[schemaPrefix], member)
			if keyword == "properties" && instanceIndex < len(failure.InstanceLocation) {
				// Small Rust property tables iterate the smaller side; fused tables iterate the instance.
				if fused || len(s.order[schemaPrefix]) >= 15 || len(order[instancePrefix]) <= len(s.order[schemaPrefix]) {
					index = slices.Index(order[instancePrefix], failure.InstanceLocation[instanceIndex])
				}
				instancePrefix += "/" + pointerEscape(failure.InstanceLocation[instanceIndex])
				instanceIndex++
			}
			if index < 0 {
				index, _ = strconv.Atoi(member)
			}
			if keyword == "patternProperties" && instanceIndex < len(failure.InstanceLocation) {
				fieldIndex := slices.Index(order[instancePrefix], failure.InstanceLocation[instanceIndex])
				if fused {
					fmt.Fprintf(&rank, "%09d/000000001/%09d/", fieldIndex, index)
				} else {
					fmt.Fprintf(&rank, "%09d/%09d/", index, fieldIndex)
				}
				instancePrefix += "/" + pointerEscape(failure.InstanceLocation[instanceIndex])
				instanceIndex++
			} else {
				fmt.Fprintf(&rank, "%09d/", index)
				if keyword == "properties" && fused {
					rank.WriteString("000000000/")
				}
			}
			schemaPrefix += "/" + pointerEscape(member)
		} else if keyword == "items" && instanceIndex < len(failure.InstanceLocation) {
			index, _ := strconv.Atoi(failure.InstanceLocation[instanceIndex])
			fmt.Fprintf(&rank, "%09d/", index)
			instancePrefix += "/" + failure.InstanceLocation[instanceIndex]
			instanceIndex++
		}
	}
	return rank.String()
}

// unevaluatedFailure combines library element errors into Rust's one diagnostic.
type unevaluatedFailure struct {
	items bool
	names []string
}

func (e *unevaluatedFailure) KeywordPath() []string {
	if e.items {
		return []string{"unevaluatedItems"}
	}
	return []string{"unevaluatedProperties"}
}
func (*unevaluatedFailure) LocalizedString(*message.Printer) string { return "" }

// aggregateFailures preserves Rust keywords' aggregate failure boundaries.
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
		nameIndex := len(failure.InstanceLocation) - 1 - descents
		if nameIndex < 0 {
			nameIndex = 0
		}
		parent := failure.InstanceLocation[:nameIndex]
		key := fmt.Sprintf("%q/%q", path[:index+1], parent)
		group := groups[key]
		if group == nil {
			group = &jsonschema.ValidationError{SchemaURL: strings.Split(failure.SchemaURL, "#")[0] + "#" + strings.TrimSuffix("/"+strings.Join(path[:index], "/"), "/"), InstanceLocation: slices.Clone(parent), ErrorKind: &unevaluatedFailure{items: items}}
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

// literal renders a schema value as Rust's compact JSON, retaining object order.
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
			var encoded bytes.Buffer
			encoder := json.NewEncoder(&encoded)
			encoder.SetEscapeHTML(false)
			// Schema keys and values have already been validated as JSON.
			_ = encoder.Encode(name)
			parts = append(parts, strings.TrimSuffix(encoded.String(), "\n")+":"+s.literal(append(slices.Clone(path), name)))
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
		var encoded bytes.Buffer
		encoder := json.NewEncoder(&encoded)
		encoder.SetEscapeHTML(false)
		// A scalar in a compiled schema is always JSON-encodable.
		_ = encoder.Encode(value)
		return strings.TrimSuffix(encoded.String(), "\n")
	}
}
