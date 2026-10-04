package types

// ID identifies the transcript block.
func (b *UserBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *UserBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *UserBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*UserBlock) IsEditable() bool {
	return true
}

// ID identifies the transcript block.
func (b *ContextBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *ContextBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *ContextBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*ContextBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *WakeupBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *WakeupBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *WakeupBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*WakeupBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *SteerBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *SteerBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *SteerBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*SteerBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *AgentMessageBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *AgentMessageBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *AgentMessageBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*AgentMessageBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *ResumeBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *ResumeBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *ResumeBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*ResumeBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *AbortBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *AbortBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *AbortBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*AbortBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *ThinkingBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *ThinkingBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *ThinkingBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*ThinkingBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *RedactedThinkingBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *RedactedThinkingBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *RedactedThinkingBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*RedactedThinkingBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *TextBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *TextBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *TextBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*TextBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *ToolCallBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *ToolCallBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *ToolCallBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*ToolCallBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *ResponseBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *ResponseBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *ResponseBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*ResponseBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *ErrorBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *ErrorBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *ErrorBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*ErrorBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *CompactionBoundaryBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *CompactionBoundaryBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *CompactionBoundaryBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*CompactionBoundaryBlock) IsEditable() bool {
	return false
}

// ID identifies the transcript block.
func (b *CompactionMarkerBlock) ID() BlockID {
	return b.BlockID
}

// CreatedAt is when the block was written.
func (b *CompactionMarkerBlock) CreatedAt() Timestamp {
	return b.Timestamp
}

// Model is the selection current when the block was written.
func (b *CompactionMarkerBlock) Model() ModelSelection {
	return b.Selection
}

// IsEditable reports whether this block is a user-editable submission.
func (*CompactionMarkerBlock) IsEditable() bool {
	return false
}
