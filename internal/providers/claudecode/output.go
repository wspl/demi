package claudecode

import (
	"encoding/json"
	"errors"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

type outputLine struct {
	Type      string
	Assistant *assistantLine
	Stream    *streamLine
	Control   *controlLine
	Answer    *controlAnswer
	Result    *resultLine
	Error     *errorLine
}
type assistantLine struct {
	Message *struct {
		Content *[]contentBlock `json:"content"`
	} `json:"message"`
	Error provider.ReportedString `json:"error" wire:"optional"`
}
type contentBlock struct {
	Type      string
	Text      string
	Thinking  *string
	Fallback  *string
	Signature *string
	Data      string
	ID        json.RawMessage
	Name      *string
	Input     json.RawMessage
}

func (b *contentBlock) UnmarshalJSON(data []byte) error {
	decoders := map[string]func(string) (contentBlock, error){
		"text": func(s string) (contentBlock, error) {
			v, e := provider.DecodeUntagged[struct {
				Text string `json:"text" wire:"optional"`
			}](s)
			return contentBlock{Type: "text", Text: v.Text}, e
		},
		"thinking": func(s string) (contentBlock, error) {
			v, e := provider.DecodeUntagged[struct {
				Thinking  *string `json:"thinking"`
				Text      *string `json:"text"`
				Signature *string `json:"signature"`
			}](s)
			return contentBlock{Type: "thinking", Thinking: v.Thinking, Fallback: v.Text, Signature: v.Signature}, e
		},
		"redacted_thinking": func(s string) (contentBlock, error) {
			v, e := provider.DecodeUntagged[struct {
				Data string `json:"data" wire:"optional"`
			}](s)
			return contentBlock{Type: "redacted_thinking", Data: v.Data}, e
		},
		"tool_use": func(s string) (contentBlock, error) {
			v, e := provider.DecodeUntagged[struct {
				ID    json.RawMessage `json:"id" wire:"optional"`
				Name  *string         `json:"name"`
				Input json.RawMessage `json:"input" wire:"optional"`
			}](s)
			if e == nil && len(v.ID) > 0 && string(v.ID) != "null" {
				var id any
				id, e = provider.DecodeUntagged[any](string(v.ID))
				switch id.(type) {
				case string, json.Number:
				default:
					e = errors.New("expected text or number tool-use id")
				}
			}
			return contentBlock{Type: "tool_use", ID: v.ID, Name: v.Name, Input: v.Input}, e
		},
	}
	value, err := provider.DecodeTagged(string(data), decoders)
	if value != nil {
		*b = *value
	}
	return err
}
func (b contentBlock) events() []provider.Event {
	switch b.Type {
	case "text":
		if b.Text != "" {
			return []provider.Event{&provider.TextDelta{Text: b.Text}}
		}
	case "thinking":
		events := []provider.Event{&provider.ThinkingStart{}}
		text := b.Thinking
		if text == nil {
			text = b.Fallback
		}
		if text != nil && *text != "" {
			events = append(events, &provider.ThinkingDelta{Text: *text})
		}
		if b.Signature != nil {
			events = append(events, &provider.ThinkingSignature{Signature: *b.Signature})
		}
		return events
	case "redacted_thinking":
		return []provider.Event{&provider.RedactedThinking{Data: b.Data}}
	}
	return nil
}
func (b contentBlock) call() (*provider.ToolCall, error) {
	invalid := errors.New("Invalid tool_use block from Claude Code") //nolint:staticcheck // Product text copied verbatim from Rust.
	if len(b.ID) == 0 || string(b.ID) == "null" || b.Name == nil || *b.Name == "" {
		return nil, invalid
	}
	id, err := provider.DecodeUntagged[any](string(b.ID))
	if err != nil {
		return nil, invalid
	}
	var text string
	switch v := id.(type) {
	case string:
		text = v
	case json.Number:
		text = string(v)
	default:
		return nil, invalid
	}
	if text == "" {
		return nil, invalid
	}
	input := b.Input
	if len(input) == 0 || string(input) == "null" {
		input = json.RawMessage(`{}`)
	}
	canonical, err := serdeValue(input).MarshalJSON()
	if err != nil {
		return nil, err
	}
	return &provider.ToolCall{ToolUseID: text, ToolName: toolName(*b.Name), Input: canonical}, nil
}
func toolName(name string) string {
	if rest, ok := strings.CutPrefix(name, "mcp__"); ok {
		server, tool, found := strings.Cut(rest, "__")
		if found && server != "" && !strings.Contains(server, "_") && tool != "" {
			return tool
		}
	}
	return name
}

type streamLine struct {
	Event *streamEvent `json:"event"`
}
type streamEvent struct {
	Type  string
	Block *contentBlock
	Delta *delta
}
type delta struct{ Type, Text string }

func (d *delta) UnmarshalJSON(data []byte) error {
	decoders := make(map[string]func(string) (delta, error))
	for tag, field := range map[string]string{"text_delta": "text", "thinking_delta": "thinking", "signature_delta": "signature"} {
		decoders[tag] = func(s string) (delta, error) {
			// Each variant reads only its own declared field.
			var text string
			var err error
			switch field {
			case "text":
				v, e := provider.DecodeUntagged[struct {
					Value string `json:"text" wire:"optional"`
				}](s)
				text, err = v.Value, e
			case "thinking":
				v, e := provider.DecodeUntagged[struct {
					Value string `json:"thinking" wire:"optional"`
				}](s)
				text, err = v.Value, e
			case "signature":
				v, e := provider.DecodeUntagged[struct {
					Value string `json:"signature" wire:"optional"`
				}](s)
				text, err = v.Value, e
			}
			return delta{tag, text}, err
		}
	}
	v, err := provider.DecodeTagged(string(data), decoders)
	if v != nil {
		*d = *v
	}
	return err
}
func (s *streamEvent) UnmarshalJSON(data []byte) error {
	v, err := provider.DecodeTagged(string(data), map[string]func(string) (streamEvent, error){
		"content_block_start": func(text string) (streamEvent, error) {
			v, e := provider.DecodeUntagged[struct {
				Block *contentBlock `json:"content_block"`
			}](text)
			return streamEvent{Type: "content_block_start", Block: v.Block}, e
		},
		"content_block_delta": func(text string) (streamEvent, error) {
			v, e := provider.DecodeUntagged[struct {
				Delta *delta `json:"delta"`
			}](text)
			return streamEvent{Type: "content_block_delta", Delta: v.Delta}, e
		},
		"message_stop": func(string) (streamEvent, error) { return streamEvent{Type: "message_stop"}, nil },
	})
	if v != nil {
		*s = *v
	}
	return err
}
func (s streamEvent) events() []provider.Event {
	if s.Block != nil {
		if s.Block.Type == "thinking" {
			return []provider.Event{&provider.ThinkingStart{}}
		}
		if s.Block.Type == "text" {
			return s.Block.events()
		}
	}
	if s.Delta != nil {
		d := s.Delta
		switch d.Type {
		case "text_delta":
			if d.Text != "" {
				return []provider.Event{&provider.TextDelta{Text: d.Text}}
			}
		case "thinking_delta":
			if d.Text != "" {
				return []provider.Event{&provider.ThinkingDelta{Text: d.Text}}
			}
		case "signature_delta":
			return []provider.Event{&provider.ThinkingSignature{Signature: d.Text}}
		}
	}
	return nil
}

type controlLine struct {
	ID      string `json:"request_id"`
	Request struct {
		Subtype string          `json:"subtype"`
		Server  *string         `json:"server_name"`
		Message json.RawMessage `json:"message" wire:"optional"`
	} `json:"request"`
}
type controlAnswer struct {
	Response *struct {
		Subtype *string                 `json:"subtype"`
		ID      *string                 `json:"request_id"`
		Error   provider.ReportedString `json:"error" wire:"optional"`
	} `json:"response"`
}
type resultLine struct {
	IsError *bool                                        `json:"is_error"`
	Result  provider.ReportedString                      `json:"result" wire:"optional"`
	Errors  provider.Reported[[]provider.ReportedString] `json:"errors" wire:"optional"`
	Status  *uint16                                      `json:"api_error_status"`
	Usage   *resultUsage                                 `json:"usage"`
}
type usageCounts struct {
	Input       *uint64 `json:"input_tokens"`
	Output      *uint64 `json:"output_tokens"`
	Read        *uint64 `json:"cache_read_input_tokens"`
	Write       *uint64 `json:"cache_creation_input_tokens"`
	InputCamel  *uint64 `json:"inputTokens"`
	OutputCamel *uint64 `json:"outputTokens"`
	ReadCamel   *uint64 `json:"cacheReadTokens"`
	WriteCamel  *uint64 `json:"cacheWriteTokens"`
}
type resultUsage struct {
	total      usageCounts
	iterations []usageCounts
}

func (r *resultUsage) UnmarshalJSON(data []byte) error {
	counts, err := provider.DecodeUntagged[usageCounts](string(data))
	if err != nil {
		return err
	}
	calls, err := provider.DecodeUntagged[struct {
		Iterations *[]usageCounts `json:"iterations"`
	}](string(data))
	if err != nil {
		return err
	}
	r.total = counts
	if calls.Iterations != nil {
		r.iterations = *calls.Iterations
	}
	return nil
}
func (r resultUsage) usage() core.TokenUsage {
	u := r.total
	if len(r.iterations) > 0 {
		u = r.iterations[len(r.iterations)-1]
	}
	count := func(a, b *uint64) uint64 {
		if a != nil {
			return *a
		}
		if b != nil {
			return *b
		}
		return 0
	}
	return core.TokenUsage{InputTokens: count(u.Input, u.InputCamel), OutputTokens: count(u.Output, u.OutputCamel), CacheReadTokens: count(u.Read, u.ReadCamel), CacheWriteTokens: count(u.Write, u.WriteCamel)}
}
func (r resultLine) end(raw string) provider.Event {
	if r.IsError == nil || !*r.IsError {
		var usage core.TokenUsage
		if r.Usage != nil {
			usage = r.Usage.usage()
		}
		return &provider.Response{Usage: usage}
	}
	var parts []string
	add := func(s *string) {
		if s != nil && strings.TrimSpace(*s) != "" {
			parts = append(parts, strings.TrimSpace(*s))
		}
	}
	add(r.Result.Value)
	if r.Errors.Value != nil {
		for _, s := range *r.Errors.Value {
			add(s.Value)
		}
	}
	message := strings.Join(parts, "\n")
	if message == "" {
		message = "Claude Code returned an error"
	}
	failure := provider.ProtocolFailure(message, raw)
	failure.Code = provider.ClassifyError(nil, message)
	if r.Status != nil {
		failure.Code = provider.HTTPErrorCode(int(*r.Status), message)
		failure.Diagnostics.HTTPStatus = r.Status
	}
	return &provider.Error{Failure: failure}
}

type errorLine struct {
	Message provider.ReportedString `json:"message" wire:"optional"`
	Code    provider.ReportedString `json:"code" wire:"optional"`
}

func decodeLine(text string) (*outputLine, error) {
	return provider.DecodeTagged(text, map[string]func(string) (outputLine, error){
		"assistant": func(s string) (outputLine, error) {
			v, e := provider.DecodeUntagged[assistantLine](s)
			return outputLine{Type: "assistant", Assistant: &v}, e
		},
		"stream_event": func(s string) (outputLine, error) {
			v, e := provider.DecodeUntagged[streamLine](s)
			return outputLine{Type: "stream_event", Stream: &v}, e
		},
		"control_request": func(s string) (outputLine, error) {
			v, e := provider.DecodeUntagged[controlLine](s)
			return outputLine{Type: "control_request", Control: &v}, e
		},
		"control_response": func(s string) (outputLine, error) {
			v, e := provider.DecodeUntagged[controlAnswer](s)
			return outputLine{Type: "control_response", Answer: &v}, e
		},
		"result": func(s string) (outputLine, error) {
			v, e := provider.DecodeUntagged[resultLine](s)
			return outputLine{Type: "result", Result: &v}, e
		},
		"error": func(s string) (outputLine, error) {
			v, e := provider.DecodeUntagged[errorLine](s)
			return outputLine{Type: "error", Error: &v}, e
		},
	})
}
