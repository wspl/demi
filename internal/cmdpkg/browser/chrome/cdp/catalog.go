package cdp

import (
	"bytes"
	_ "embed"
	"encoding/json"
	"fmt"
	"strings"
	"sync"

	jsonv2 "github.com/go-json-experiment/json"
	"github.com/santhosh-tekuri/jsonschema/v6"
)

//go:embed protocol.json
var pinnedProtocol []byte

type protocolShape struct {
	// ID identifies a named protocol definition.
	ID string `json:"id"`
	// Name names the protocol field or command.
	Name string `json:"name"`
	// Optional reports whether the field may be omitted.
	Optional bool `json:"optional"`
	// Reference names the referenced protocol definition.
	Reference string `json:"$ref"`
	// Kind names the protocol value type.
	Kind string `json:"type"`
	// Variants lists the accepted enum values.
	Variants []string `json:"enum"`
	// Items describes each array element.
	Items *protocolShape `json:"items"`
	// Properties describes the object fields.
	Properties *[]protocolShape `json:"properties"`
}
type protocolDeclaration struct {
	// Name names the protocol field or command.
	Name string `json:"name"`
	// Parameters describes the command inputs.
	Parameters []protocolShape `json:"parameters"`
	// Returns describes the command reply.
	Returns []protocolShape `json:"returns"`
}
type protocolDomain struct {
	// Domain names the protocol domain.
	Domain string `json:"domain"`
	// Types lists the domain definitions.
	Types []protocolShape `json:"types"`
	// Commands lists the domain commands.
	Commands []protocolDeclaration `json:"commands"`
	// Events lists the domain events.
	Events []protocolDeclaration `json:"events"`
}
type protocolCatalog struct {
	definitions map[string]any
	schemas     map[string]any
	mu          sync.Mutex
	compiled    map[string]*jsonschema.Schema
}

var catalogOnce = sync.OnceValues(loadCatalog)

// loadCatalog derives the pinned Chrome schemas once, independently of cdproto's version.
func loadCatalog() (*protocolCatalog, error) {
	var document struct {
		Domains []protocolDomain `json:"domains"`
	}
	if err := jsonv2.Unmarshal(pinnedProtocol, &document); err != nil {
		return nil, err
	}
	c := &protocolCatalog{
		definitions: map[string]any{},
		schemas:     map[string]any{},
		compiled:    map[string]*jsonschema.Schema{},
	}
	for _, domain := range document.Domains {
		for _, shape := range domain.Types {
			if shape.ID == "" {
				return nil, fmt.Errorf("CDP type has no id")
			}
			schema, err := shape.schema(domain.Domain)
			if err != nil {
				return nil, err
			}
			c.definitions[domain.Domain+"."+shape.ID] = schema
		}
		for _, command := range domain.Commands {
			for kind, fields := range map[string][]protocolShape{"params": command.Parameters, "returns": command.Returns} {
				schema, err := recordSchema(fields, domain.Domain)
				if err != nil {
					return nil, err
				}
				c.schemas[domain.Domain+"."+command.Name+":"+kind] = schema
			}
		}
		for _, event := range domain.Events {
			schema, err := recordSchema(event.Parameters, domain.Domain)
			if err != nil {
				return nil, err
			}
			c.schemas[domain.Domain+"."+event.Name+":event"] = schema
		}
	}
	return c, nil
}

// schema translates one Chrome protocol shape into JSON Schema.
func (s protocolShape) schema(domain string) (map[string]any, error) {
	if s.Reference != "" {
		reference := s.Reference
		if !strings.Contains(reference, ".") {
			reference = domain + "." + reference
		}
		return map[string]any{"$ref": "#/$defs/" + reference}, nil
	}
	result := map[string]any{}
	switch s.Kind {
	case "any":
	case "binary":
		result = map[string]any{
			"type":    "string",
			"pattern": "^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$",
		}
	case "array":
		if s.Items == nil {
			return nil, fmt.Errorf("CDP array has no items")
		}
		item, err := s.Items.schema(domain)
		if err != nil {
			return nil, err
		}
		result = map[string]any{"type": "array", "items": item}
	case "object":
		if s.Properties != nil {
			return recordSchema(*s.Properties, domain)
		}
		result["type"] = "object"
	case "string", "integer", "number", "boolean":
		result["type"] = s.Kind
	case "":
		return nil, fmt.Errorf("CDP type has no type or reference")
	default:
		return nil, fmt.Errorf("unknown pinned CDP type %s", s.Kind)
	}
	if s.Variants != nil {
		values := make([]any, len(s.Variants))
		for i, v := range s.Variants {
			values[i] = v
		}
		result["enum"] = values
	}
	return result, nil
}

// recordSchema retains Chrome's required fields and refuses undeclared record members.
func recordSchema(fields []protocolShape, domain string) (map[string]any, error) {
	properties := map[string]any{}
	required := []any{}
	for _, field := range fields {
		if field.Name == "" {
			return nil, fmt.Errorf("CDP parameter has no name")
		}
		schema, err := field.schema(domain)
		if err != nil {
			return nil, err
		}
		properties[field.Name] = schema
		if !field.Optional {
			required = append(required, field.Name)
		}
	}
	return map[string]any{
		"type":                 "object",
		"properties":           properties,
		"required":             required,
		"additionalProperties": false,
	}, nil
}

// offlineLoader refuses external schema resolution for Chrome and page tools.
type offlineLoader struct{}

// Load refuses schema references that require an external resource.
func (offlineLoader) Load(url string) (any, error) {
	return nil, fmt.Errorf("external schema resource is unavailable offline: %s", url)
}

// compileSchema uses the selected JSON Schema library for the whole validation job.
func compileSchema(schema any) (*jsonschema.Schema, error) {
	compiler := jsonschema.NewCompiler()
	compiler.UseLoader(offlineLoader{})
	if err := compiler.AddResource("https://demi.invalid/schema", schema); err != nil {
		return nil, err
	}
	return compiler.Compile("https://demi.invalid/schema")
}

// pinnedSchema returns a compiled Chrome method schema without holding a lock during compilation.
func pinnedSchema(method, kind string) (*jsonschema.Schema, error) {
	c, err := catalogOnce()
	if err != nil {
		return nil, &BrowserError{Kind: KindCDP, Cause: err}
	}
	key := method + ":" + kind
	c.mu.Lock()
	compiled := c.compiled[key]
	c.mu.Unlock()
	if compiled != nil {
		return compiled, nil
	}
	shape, exists := c.schemas[key]
	if !exists {
		return nil, &BrowserError{
			Kind:    KindConfiguration,
			Message: fmt.Sprintf("unknown pinned CDP %s method: %s", kind, method),
		}
	}
	// Every catalog root is constructed by recordSchema above.
	object := shape.(map[string]any)
	root := make(map[string]any, len(object)+1)
	for k, v := range object {
		root[k] = v
	}
	root["$defs"] = c.definitions
	compiled, err = compileSchema(root)
	if err != nil {
		return nil, &BrowserError{Kind: KindCDP, Cause: err}
	}
	c.mu.Lock()
	c.compiled[key] = compiled
	c.mu.Unlock()
	return compiled, nil
}

// validatePinned validates raw Chrome data without trusting an asserted Go shape.
func validatePinned(method, kind string, data []byte) error {
	schema, err := pinnedSchema(method, kind)
	if err != nil {
		return err
	}
	value, err := jsonschema.UnmarshalJSON(bytes.NewReader(data))
	if err == nil {
		err = schema.Validate(value)
	}
	if err != nil {
		return &BrowserError{Kind: KindCDP, Message: fmt.Sprintf("invalid %s %s: %v", method, kind, err), Cause: err}
	}
	return nil
}

// pageSchema validates a WebMCP page's untrusted schema with offline resolution.
func pageSchema(raw json.RawMessage) (*jsonschema.Schema, error) {
	value, err := jsonschema.UnmarshalJSON(bytes.NewReader(raw))
	if err != nil {
		return nil, err
	}
	return compileSchema(value)
}

// typedShape preserves serde's tolerant typed CDP records and nullable optional
// fields. Raw debugging records keep the strict pinned schemas above.
func typedShape(value any) any {
	switch value := value.(type) {
	case map[string]any:
		result := make(map[string]any, len(value))
		for key, child := range value {
			if key != "additionalProperties" || value["type"] != "object" {
				result[key] = typedShape(child)
			}
		}
		properties, ok := result["properties"].(map[string]any)
		if ok && result["type"] == "object" {
			required := map[string]bool{}
			for _, name := range result["required"].([]any) {
				required[name.(string)] = true
			}
			for name, shape := range properties {
				if !required[name] {
					properties[name] = map[string]any{"anyOf": []any{shape, map[string]any{"type": "null"}}}
				}
			}
		}
		return result
	case []any:
		result := make([]any, len(value))
		for i, child := range value {
			result[i] = typedShape(child)
		}
		return result
	default:
		return value
	}
}

// validateTyped enforces the fields Rust's typed CDP decoder required. cdproto's
// JSON decoder alone leaves absent required fields at zero (including nil trees).
func validateTyped(method, kind string, data []byte) error {
	c, err := catalogOnce()
	if err != nil {
		return &BrowserError{Kind: KindCDP, Cause: err}
	}
	key := method + ":" + kind
	c.mu.Lock()
	compiled := c.compiled["typed:"+key]
	c.mu.Unlock()
	if compiled == nil {
		shape, exists := c.schemas[key]
		if !exists {
			return &BrowserError{Kind: KindCDP, Message: fmt.Sprintf("unknown pinned CDP %s method: %s", kind, method)}
		}
		// Catalog roots are record schemas constructed locally, never asserted wire data.
		root := typedShape(shape).(map[string]any)
		root["$defs"] = typedShape(c.definitions)
		compiled, err = compileSchema(root)
		if err != nil {
			return &BrowserError{Kind: KindCDP, Cause: err}
		}
		c.mu.Lock()
		c.compiled["typed:"+key] = compiled
		c.mu.Unlock()
	}
	value, err := jsonschema.UnmarshalJSON(bytes.NewReader(data))
	if err == nil {
		err = compiled.Validate(value)
	}
	if err != nil {
		return &BrowserError{Kind: KindCDP, Message: fmt.Sprintf("invalid %s %s: %v", method, kind, err), Cause: err}
	}
	return nil
}
