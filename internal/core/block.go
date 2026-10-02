package core

// BlockIdentity reads the common metadata of a transcript block.
func BlockIdentity(block Block) BlockID {
	switch b := block.(type) {
	case *UserBlock:
		if b != nil {
			return b.ID
		}
	case *ContextBlock:
		if b != nil {
			return b.ID
		}
	case *WakeupBlock:
		if b != nil {
			return b.ID
		}
	case *SteerBlock:
		if b != nil {
			return b.ID
		}
	case *AgentMessageBlock:
		if b != nil {
			return b.ID
		}
	case *ResumeBlock:
		if b != nil {
			return b.ID
		}
	case *AbortBlock:
		if b != nil {
			return b.ID
		}
	case *ThinkingBlock:
		if b != nil {
			return b.ID
		}
	case *RedactedThinkingBlock:
		if b != nil {
			return b.ID
		}
	case *TextBlock:
		if b != nil {
			return b.ID
		}
	case *ToolCallBlock:
		if b != nil {
			return b.ID
		}
	case *ResponseBlock:
		if b != nil {
			return b.ID
		}
	case *ErrorBlock:
		if b != nil {
			return b.ID
		}
	case *CompactionBoundaryBlock:
		if b != nil {
			return b.ID
		}
	case *CompactionMarkerBlock:
		if b != nil {
			return b.ID
		}
	}
	return ""
}

// BlockCreatedAt reads the common metadata of a transcript block.
func BlockCreatedAt(block Block) Timestamp {
	switch b := block.(type) {
	case *UserBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *ContextBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *WakeupBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *SteerBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *AgentMessageBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *ResumeBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *AbortBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *ThinkingBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *RedactedThinkingBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *TextBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *ToolCallBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *ResponseBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *ErrorBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *CompactionBoundaryBlock:
		if b != nil {
			return b.CreatedAt
		}
	case *CompactionMarkerBlock:
		if b != nil {
			return b.CreatedAt
		}
	}
	return ""
}

// IsEditable reports whether the user can edit this transcript block.
func IsEditable(block Block) bool {
	_, ok := block.(*UserBlock)
	return ok
}
