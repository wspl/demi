package provider

// Optional vendor members declare serde-compatible null-as-absent decoding.

//demi:wire open
type ChatChunk struct {
	Choices *[]ChatChoice `json:"choices,omitzero" check:"nullabsent"`
	Usage   *ChatUsage    `json:"usage,omitzero" check:"nullabsent"`
	Error   *ChatError    `json:"error,omitzero" check:"nullabsent"`
}

//demi:wire open
type ChatChoice struct {
	Delta        *ChatDelta `json:"delta,omitzero" check:"nullabsent"`
	FinishReason *string    `json:"finish_reason,omitzero" check:"nullabsent"`
}

//demi:wire open
type ChatDelta struct {
	Content          *string          `json:"content,omitzero" check:"nullabsent"`
	ReasoningContent *string          `json:"reasoning_content,omitzero" check:"nullabsent"`
	ToolCalls        *[]ChatToolDelta `json:"tool_calls,omitzero" check:"nullabsent"`
}

//demi:wire open
type ChatToolDelta struct {
	Index    *uint32            `json:"index,omitzero" check:"nullabsent"`
	ID       *string            `json:"id,omitzero" check:"nullabsent"`
	Function *ChatFunctionDelta `json:"function,omitzero" check:"nullabsent"`
}

//demi:wire open
type ChatFunctionDelta struct {
	Name      *string `json:"name,omitzero" check:"nullabsent"`
	Arguments *string `json:"arguments,omitzero" check:"nullabsent"`
}

//demi:wire open
type ChatUsage struct {
	PromptTokens        *uint64                  `json:"prompt_tokens,omitzero" check:"nullabsent"`
	CompletionTokens    *uint64                  `json:"completion_tokens,omitzero" check:"nullabsent"`
	PromptTokensDetails *ChatPromptTokensDetails `json:"prompt_tokens_details,omitzero" check:"nullabsent"`
}

//demi:wire open
type ChatPromptTokensDetails struct {
	CachedTokens *uint64 `json:"cached_tokens,omitzero" check:"nullabsent"`
}

//demi:wire open
type ChatError struct {
	Message *ReportedString `json:"message,omitzero" check:"nullabsent"`
	Code    *ReportedString `json:"code,omitzero" check:"nullabsent"`
	Type    *ReportedString `json:"type,omitzero" check:"nullabsent"`
}
