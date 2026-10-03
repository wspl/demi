package anthropicapi

import (
	"encoding/json"

	"github.com/wspl/demi/internal/contract"
)

// These private vendor types declare fields in the order the Messages request
// writes them. Generic tool inputs and schemas retain their original JSON order.
type message struct {
	Role    string     `json:"role"`
	Content []*content `json:"content"`
}
type requestBody struct {
	Model        string         `json:"model"`
	Messages     []message      `json:"messages"`
	MaxTokens    uint32         `json:"max_tokens"`
	Stream       bool           `json:"stream"`
	System       []*content     `json:"system,omitempty"`
	Tools        []tool         `json:"tools,omitempty"`
	Thinking     thinkingConfig `json:"thinking,omitempty"`
	OutputConfig *outputConfig  `json:"output_config,omitempty"`
	ServiceTier  *string        `json:"service_tier,omitempty"`
}
type content struct {
	block block
	cache *cacheControl
}

// MarshalJSON flattens a Messages block before its optional cache control,
// preserving the declared field order through the shared ordered codec.
func (c *content) MarshalJSON() ([]byte, error) {
	encoded, err := contract.EncodeJSON(c.block)
	if err != nil || c.cache == nil {
		return encoded, err
	}
	fields, err := contract.ObjectFields(encoded)
	if err != nil {
		return nil, err
	}
	fields = append(fields, contract.Field{Name: "cache_control", Value: c.cache})
	return contract.EncodeObject(fields)
}

// canMark excludes reasoning from the vendor's cache boundaries.
func (c *content) canMark() bool {
	switch c.block.(type) {
	case *textBlock, *imageBlock, *documentBlock, *toolUseBlock, *toolResultBlock:
		return true
	case *thinkingBlock, *redactedThinkingBlock:
		return false
	}
	return false
}

type cacheControl struct {
	Type string `json:"type"`
	TTL  string `json:"ttl"`
}

//sumtype:decl
type block interface{ isBlock() }

//sumtype:decl
type resultBlock interface{ isResultBlock() }

type textBlock struct {
	Type string `json:"type"`
	Text string `json:"text"`
}
type imageBlock struct {
	Type   string       `json:"type"`
	Source base64Source `json:"source"`
}
type documentBlock struct {
	Type   string       `json:"type"`
	Source base64Source `json:"source"`
	Title  string       `json:"title"`
}
type toolUseBlock struct {
	Type  string          `json:"type"`
	ID    string          `json:"id"`
	Name  string          `json:"name"`
	Input json.RawMessage `json:"input"`
}
type toolResultBlock struct {
	Type      string        `json:"type"`
	ToolUseID string        `json:"tool_use_id"`
	Content   []resultBlock `json:"content"`
	IsError   bool          `json:"is_error,omitempty"`
}
type thinkingBlock struct {
	Type      string `json:"type"`
	Thinking  string `json:"thinking"`
	Signature string `json:"signature"`
}
type redactedThinkingBlock struct {
	Type string `json:"type"`
	Data string `json:"data"`
}

func (*textBlock) isBlock()             {}
func (*imageBlock) isBlock()            {}
func (*documentBlock) isBlock()         {}
func (*toolUseBlock) isBlock()          {}
func (*toolResultBlock) isBlock()       {}
func (*thinkingBlock) isBlock()         {}
func (*redactedThinkingBlock) isBlock() {}
func (*textBlock) isResultBlock()       {}
func (*imageBlock) isResultBlock()      {}

type base64Source struct {
	Type      string `json:"type"`
	MediaType string `json:"media_type"`
	Data      string `json:"data"`
}
type tool struct {
	Name         string          `json:"name"`
	Description  string          `json:"description"`
	InputSchema  json.RawMessage `json:"input_schema"`
	CacheControl *cacheControl   `json:"cache_control,omitempty"`
}

//sumtype:decl
type thinkingConfig interface{ isThinkingConfig() }

type enabledThinking struct {
	Type         string `json:"type"`
	BudgetTokens uint32 `json:"budget_tokens"`
}
type adaptiveThinking struct {
	Type    string `json:"type"`
	Display string `json:"display"`
}

func (*enabledThinking) isThinkingConfig()  {}
func (*adaptiveThinking) isThinkingConfig() {}

type outputConfig struct {
	Effort string `json:"effort"`
}
