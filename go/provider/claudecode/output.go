package claudecode

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"math"
	"strconv"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/rustfmt"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/provider"
)

// Each registered CLI tag uses its generated decoder; unknown tags stay absent.
//
//demi:wire open
type assistantLine struct {
	Message *assistantMessage        `json:"message,omitzero" check:"nullabsent"`
	Error   *provider.ReportedString `json:"error,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:wire open
type assistantMessage struct {
	Content *[]jsontext.Value `json:"content,omitzero" check:"nullabsent"`
}

type contentBlock struct {
	Type                            string
	Text, Thinking, Signature, Data *string
	ID                              *toolUseID
	Name                            *string
	Input                           *jsontext.Value
}

//demi:wire open
type textContent struct {
	Text *string `json:"text,omitzero"`
}

//demi:wire open
type thinkingContent struct {
	Thinking  *string `json:"thinking,omitzero" check:"nullabsent"`
	Text      *string `json:"text,omitzero" check:"nullabsent"`
	Signature *string `json:"signature,omitzero" check:"nullabsent"`
}

//demi:wire open
type redactedContent struct {
	Data *string `json:"data,omitzero"`
}

//demi:wire open
type toolContent struct {
	ID    *toolUseID      `json:"id,omitzero" check:"nullabsent"`
	Name  *string         `json:"name,omitzero" check:"nullabsent"`
	Input *jsontext.Value `json:"input,omitzero" check:"nullabsent"`
}

//demi:opaque
type toolUseID struct{ value string }

func (id *toolUseID) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	if raw.Kind() == '"' {
		return json.Unmarshal(raw, &id.value)
	}
	if raw.Kind() == '0' {
		// serde reads -0 as a float. NormalizeJSON currently classifies it as
		// integer zero, so use the shared float formatter to retain its sign.
		if string(raw) == "-0" {
			id.value = rustfmt.JSONFloat(math.Copysign(0, -1))
			return nil
		}
		normalized, err := rustfmt.NormalizeJSON(raw)
		if err != nil {
			return err
		}
		id.value = string(normalized)
		return nil
	}
	return fmt.Errorf("a tool use id is text or a number")
}

//demi:wire open
type streamEventLine struct {
	Event *jsontext.Value `json:"event,omitzero" check:"nullabsent"`
}

type streamEvent struct {
	Type         string          `json:"type"`
	ContentBlock *jsontext.Value `json:"content_block,omitzero"`
	Delta        *jsontext.Value `json:"delta,omitzero"`
}

//demi:wire open
type blockStart struct {
	ContentBlock *jsontext.Value `json:"content_block,omitzero" check:"nullabsent"`
}

//demi:wire open
type blockDelta struct {
	Delta *jsontext.Value `json:"delta,omitzero" check:"nullabsent"`
}

//demi:wire open
type messageStop struct{}

func decodeStream(raw jsontext.Value) (streamEvent, bool, error) {
	return provider.DecodeTagged(raw, map[string]func([]byte) (streamEvent, error){
		"content_block_start": func(raw []byte) (streamEvent, error) {
			v, err := decode[blockStart](raw)
			return streamEvent{Type: "content_block_start", ContentBlock: v.ContentBlock}, err
		},
		"content_block_delta": func(raw []byte) (streamEvent, error) {
			v, err := decode[blockDelta](raw)
			return streamEvent{Type: "content_block_delta", Delta: v.Delta}, err
		},
		"message_stop": func(raw []byte) (streamEvent, error) {
			_, err := decode[messageStop](raw)
			return streamEvent{Type: "message_stop"}, err
		},
	})
}

type delta struct {
	Type                      string
	Text, Thinking, Signature *string
}

//demi:wire open
type thinkingDelta struct {
	Thinking *string `json:"thinking,omitzero"`
}

//demi:wire open
type signatureDelta struct {
	Signature *string `json:"signature,omitzero"`
}

//demi:wire open
type controlRequestLine struct {
	RequestID string             `json:"request_id"`
	Request   controlRequestBody `json:"request"`
}

//demi:wire open
type controlRequestBody struct {
	Subtype    string          `json:"subtype"`
	ServerName *string         `json:"server_name,omitzero" check:"nullabsent"`
	Message    *jsontext.Value `json:"message,omitzero" check:"nullabsent"`
}

//demi:wire open
type controlResponseLine struct {
	Response *controlResponseBody `json:"response,omitzero" check:"nullabsent"`
}

//demi:wire open
type controlResponseBody struct {
	Subtype   *string                  `json:"subtype,omitzero" check:"nullabsent"`
	RequestID *string                  `json:"request_id,omitzero" check:"nullabsent"`
	Error     *provider.ReportedString `json:"error,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:wire open
type resultLine struct {
	IsError        *bool                    `json:"is_error,omitzero" check:"nullabsent"`
	Result         *provider.ReportedString `json:"result,omitzero" check:"nullabsent,func=provider.Validate"`
	Errors         *reportedErrors          `json:"errors,omitzero" check:"nullabsent"`
	APIErrorStatus *uint16                  `json:"api_error_status,omitzero" check:"nullabsent"`
	Usage          *resultUsage             `json:"usage,omitzero" check:"nullabsent"`
}

//demi:opaque
type reportedErrors struct{ values []provider.ReportedString }

func (v *reportedErrors) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.values = nil
	var values []provider.ReportedString
	if json.Unmarshal(raw, &values) == nil {
		v.values = values
	}
	return nil
}

//demi:wire open
type usageCounts struct {
	InputTokens       *uint64 `json:"input_tokens,omitzero" check:"nullabsent"`
	OutputTokens      *uint64 `json:"output_tokens,omitzero" check:"nullabsent"`
	CacheRead         *uint64 `json:"cache_read_input_tokens,omitzero" check:"nullabsent"`
	CacheWrite        *uint64 `json:"cache_creation_input_tokens,omitzero" check:"nullabsent"`
	InputTokensCamel  *uint64 `json:"inputTokens,omitzero" check:"nullabsent"`
	OutputTokensCamel *uint64 `json:"outputTokens,omitzero" check:"nullabsent"`
	CacheReadCamel    *uint64 `json:"cacheReadTokens,omitzero" check:"nullabsent"`
	CacheWriteCamel   *uint64 `json:"cacheWriteTokens,omitzero" check:"nullabsent"`
}

//demi:wire open
type usageIterations struct {
	Iterations *[]usageCounts `json:"iterations,omitzero" check:"nullabsent"`
}

// resultUsage composes two generated views of the CLI's flattened usage object.
//
//demi:opaque
type resultUsage struct {
	total      usageCounts
	iterations *[]usageCounts
}

func (v *resultUsage) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	total, err := decode[usageCounts](raw)
	if err != nil {
		return err
	}
	iterations, err := decode[usageIterations](raw)
	if err != nil {
		return err
	}
	*v = resultUsage{total: total, iterations: iterations.Iterations}
	return nil
}

//demi:wire open
type errorLine struct {
	Message *provider.ReportedString `json:"message,omitzero" check:"nullabsent,func=provider.Validate"`
	Code    *provider.ReportedString `json:"code,omitzero" check:"nullabsent,func=provider.Validate"`
}

func text(v *string) string {
	if v == nil {
		return ""
	}
	return *v
}
func reported(v *provider.ReportedString) *string {
	if v == nil {
		return nil
	}
	return v.Value
}
func toolName(name string) string {
	rest, ok := strings.CutPrefix(name, "mcp__")
	if !ok {
		return name
	}
	server, tool, ok := strings.Cut(rest, "__")
	if ok && server != "" && !strings.Contains(server, "_") && tool != "" {
		return tool
	}
	return name
}
func block(raw jsontext.Value) (contentBlock, bool, error) {
	return provider.DecodeTagged(raw, map[string]func([]byte) (contentBlock, error){
		"text": func(raw []byte) (contentBlock, error) {
			v, err := decode[textContent](raw)
			return contentBlock{Type: "text", Text: v.Text}, err
		},
		"thinking": func(raw []byte) (contentBlock, error) {
			v, err := decode[thinkingContent](raw)
			return contentBlock{Type: "thinking", Text: v.Text, Thinking: v.Thinking, Signature: v.Signature}, err
		},
		"redacted_thinking": func(raw []byte) (contentBlock, error) {
			v, err := decode[redactedContent](raw)
			return contentBlock{Type: "redacted_thinking", Data: v.Data}, err
		},
		"tool_use": func(raw []byte) (contentBlock, error) {
			v, err := decode[toolContent](raw)
			return contentBlock{Type: "tool_use", ID: v.ID, Name: v.Name, Input: v.Input}, err
		},
	})
}
func (b contentBlock) events() []provider.ProviderEvent {
	switch b.Type {
	case "text":
		if text(b.Text) != "" {
			return []provider.ProviderEvent{provider.TextDelta{Text: *b.Text}}
		}
	case "thinking":
		events := []provider.ProviderEvent{provider.ThinkingStart{}}
		thinking := b.Thinking
		if thinking == nil {
			thinking = b.Text
		}
		if text(thinking) != "" {
			events = append(events, provider.ThinkingDelta{Text: *thinking})
		}
		if b.Signature != nil {
			events = append(events, provider.ThinkingSignature{Signature: *b.Signature})
		}
		return events
	case "redacted_thinking":
		return []provider.ProviderEvent{provider.RedactedThinking{Data: text(b.Data)}}
	}
	return nil
}
func (b contentBlock) call() (provider.ToolCall, error) {
	if b.ID == nil || b.ID.value == "" || text(b.Name) == "" {
		return provider.ToolCall{}, fmt.Errorf("Invalid tool_use block from Claude Code")
	}
	input := jsontext.Value(`{}`)
	if b.Input != nil {
		input = *b.Input
	}
	return provider.ToolCall{ToolUseID: b.ID.value, ToolName: toolName(*b.Name), Input: input}, nil
}
func (e streamEvent) events() ([]provider.ProviderEvent, error) {
	switch e.Type {
	case "content_block_start":
		if e.ContentBlock == nil {
			return nil, nil
		}
		b, known, err := block(*e.ContentBlock)
		if err != nil || !known {
			return nil, err
		}
		if b.Type == "thinking" {
			return []provider.ProviderEvent{provider.ThinkingStart{}}, nil
		}
		if b.Type == "text" {
			return b.events(), nil
		}
	case "content_block_delta":
		if e.Delta == nil {
			return nil, nil
		}
		d, known, err := provider.DecodeTagged(*e.Delta, map[string]func([]byte) (delta, error){"text_delta": func(raw []byte) (delta, error) {
			v, err := decode[textContent](raw)
			return delta{Type: "text_delta", Text: v.Text}, err
		},
			"thinking_delta": func(raw []byte) (delta, error) {
				v, err := decode[thinkingDelta](raw)
				return delta{Type: "thinking_delta", Thinking: v.Thinking}, err
			},
			"signature_delta": func(raw []byte) (delta, error) {
				v, err := decode[signatureDelta](raw)
				return delta{Type: "signature_delta", Signature: v.Signature}, err
			}})
		if err != nil || !known {
			return nil, err
		}
		switch d.Type {
		case "text_delta":
			if text(d.Text) != "" {
				return []provider.ProviderEvent{provider.TextDelta{Text: *d.Text}}, nil
			}
		case "thinking_delta":
			if text(d.Thinking) != "" {
				return []provider.ProviderEvent{provider.ThinkingDelta{Text: *d.Thinking}}, nil
			}
		case "signature_delta":
			return []provider.ProviderEvent{provider.ThinkingSignature{Signature: text(d.Signature)}}, nil
		}
	}
	return nil, nil
}

type turnEnd struct {
	Usage   core.TokenUsage
	Failure *provider.ProviderFailure
	Status  *uint16
}

func (r resultLine) end() turnEnd {
	if r.IsError != nil && *r.IsError {
		pieces := []string{}
		if v := reported(r.Result); v != nil {
			pieces = append(pieces, *v)
		}
		if r.Errors != nil {
			for _, v := range r.Errors.values {
				if v.Value != nil {
					pieces = append(pieces, *v.Value)
				}
			}
		}
		cleaned := []string{}
		for _, s := range pieces {
			if s = strings.TrimSpace(s); s != "" {
				cleaned = append(cleaned, s)
			}
		}
		message := strings.Join(cleaned, "\n")
		if message == "" {
			message = "Claude Code returned an error"
		}
		code := provider.ClassifyVendorFailure("", message)
		if r.APIErrorStatus != nil {
			code = provider.HTTPErrorCode(*r.APIErrorStatus, message)
		}
		return turnEnd{Failure: &provider.ProviderFailure{Message: message, Code: code}, Status: r.APIErrorStatus}
	}
	usage := core.TokenUsage{}
	if r.Usage != nil {
		counts := r.Usage.total
		if r.Usage.iterations != nil && len(*r.Usage.iterations) > 0 {
			counts = (*r.Usage.iterations)[len(*r.Usage.iterations)-1]
		}
		count := func(snake, camel *uint64) uint64 {
			if snake != nil {
				return *snake
			}
			if camel != nil {
				return *camel
			}
			return 0
		}
		usage = core.TokenUsage{InputTokens: count(counts.InputTokens, counts.InputTokensCamel), OutputTokens: count(counts.OutputTokens, counts.OutputTokensCamel), CacheReadTokens: count(counts.CacheRead, counts.CacheReadCamel), CacheWriteTokens: count(counts.CacheWrite, counts.CacheWriteCamel)}
	}
	return turnEnd{Usage: usage}
}
func (e errorLine) failure() provider.ProviderFailure {
	message := "Claude Code error"
	if v := reported(e.Message); v != nil {
		message = *v
	}
	return provider.ProviderFailure{Message: message, Code: provider.ClassifyVendorFailure(text(reported(e.Code)), message)}
}

type cliLine interface{ cliLine() }

func (assistantLine) cliLine()       {}
func (streamEventLine) cliLine()     {}
func (controlRequestLine) cliLine()  {}
func (controlResponseLine) cliLine() {}
func (resultLine) cliLine()          {}
func (errorLine) cliLine()           {}

// decodeCLI preserves the concrete generated CLI variant after tag selection.
func decodeCLI[T cliLine](raw []byte) (cliLine, error) {
	value, err := decode[T](raw)
	if err != nil {
		return value, err
	}
	switch line := cliLine(value).(type) {
	case assistantLine:
		if line.Message != nil && line.Message.Content != nil {
			for i, raw := range *line.Message.Content {
				if _, _, err := block(raw); err != nil {
					return value, wire.In("message.content["+strconv.Itoa(i)+"]", err)
				}
			}
		}
	case streamEventLine:
		if line.Event != nil {
			event, known, err := decodeStream(*line.Event)
			if err != nil {
				return value, wire.In("event", err)
			}
			if known {
				if _, err := event.events(); err != nil {
					return value, wire.In("event", err)
				}
			}
		}
	}
	return value, nil
}
func decodeLine(raw []byte) (cliLine, bool, error) {
	return provider.DecodeTagged(raw, map[string]func([]byte) (cliLine, error){
		"assistant": decodeCLI[assistantLine], "stream_event": decodeCLI[streamEventLine],
		"control_request": decodeCLI[controlRequestLine], "control_response": decodeCLI[controlResponseLine],
		"result": decodeCLI[resultLine], "error": decodeCLI[errorLine],
	})
}
