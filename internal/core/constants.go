package core

// MaxSafeInteger and related constants define the supported contract values.
const (
	MaxSafeInteger uint64 = 1<<53 - 1
	ThinkingOff           = "disabled"
	ProfileInherit        = "default"
)

// WakeupPlacementNewTurn and related constants define the supported contract values.
const (
	WakeupPlacementNewTurn WakeupPlacement = "new_turn"
	WakeupPlacementSteer   WakeupPlacement = "steer"
)

// ToolCallStatusExecuting and related constants define the supported contract values.
const (
	ToolCallStatusExecuting ToolCallStatus = "executing"
	ToolCallStatusCompleted ToolCallStatus = "completed"
	ToolCallStatusError     ToolCallStatus = "error"
)

// ModelMediaKindImage and related constants define the supported contract values.
const (
	ModelMediaKindImage ModelMediaKind = "image"
	ModelMediaKindVideo ModelMediaKind = "video"
)

// FileExtensionPNG and related constants define the supported contract values.
const (
	FileExtensionPNG  FileExtension = "png"
	FileExtensionJPG  FileExtension = "jpg"
	FileExtensionJPEG FileExtension = "jpeg"
	FileExtensionGIF  FileExtension = "gif"
	FileExtensionWebP FileExtension = "webp"
	FileExtensionPDF  FileExtension = "pdf"
	FileExtensionMP4  FileExtension = "mp4"
	FileExtensionMov  FileExtension = "mov"
	FileExtensionWebM FileExtension = "webm"
	FileExtensionM4V  FileExtension = "m4v"
)

// ThinkingSummaryAuto and related constants define the supported contract values.
const (
	ThinkingSummaryAuto     ThinkingSummary = "auto"
	ThinkingSummaryConcise  ThinkingSummary = "concise"
	ThinkingSummaryDetailed ThinkingSummary = "detailed"
	ThinkingSummaryOff      ThinkingSummary = "off"
	ThinkingSummaryOn       ThinkingSummary = "on"
)

// ShellViewStatusRunning and related constants define the supported contract values.
const (
	ShellViewStatusRunning ShellViewStatus = "running"
	ShellViewStatusExited  ShellViewStatus = "exited"
	ShellViewStatusAborted ShellViewStatus = "aborted"
)

// StreamKindStdout and related constants define the supported contract values.
const (
	StreamKindStdout StreamKind = "stdout"
	StreamKindStderr StreamKind = "stderr"
)

// EditKindAdded and related constants define the supported contract values.
const (
	EditKindAdded    EditKind = "added"
	EditKindModified EditKind = "modified"
)

// FailureSourceHTTP and related constants define the supported contract values.
const (
	FailureSourceHTTP      FailureSource = "http"
	FailureSourceStream    FailureSource = "stream"
	FailureSourceTransport FailureSource = "transport"
	FailureSourceUnknown   FailureSource = "unknown"
)

// CompletionOutcomeCompleted and related constants define the supported contract values.
const (
	CompletionOutcomeCompleted CompletionOutcome = "completed"
	CompletionOutcomeFailed    CompletionOutcome = "failed"
	CompletionOutcomeAborted   CompletionOutcome = "aborted"
)

// SequenceCommand and related constants define the supported contract values.
const (
	SequenceCommand Sequence = "command"
	SequenceShell   Sequence = "shell"
	SequenceAgent   Sequence = "agent"
	SequenceTab     Sequence = "tab"
)

// SessionPhaseIdle and related constants define the supported contract values.
const (
	SessionPhaseIdle       SessionPhase = "idle"
	SessionPhaseRunning    SessionPhase = "running"
	SessionPhaseCompacting SessionPhase = "compacting"
)

// SnapshotSourceProbe and related constants define the supported contract values.
const (
	SnapshotSourceProbe       SnapshotSource = "probe"
	SnapshotSourceObservation SnapshotSource = "observation"
)

// QuotaUnitPercent and related constants define the supported contract values.
const (
	QuotaUnitPercent  QuotaUnit = "percent"
	QuotaUnitCredits  QuotaUnit = "credits"
	QuotaUnitUSDMinor QuotaUnit = "usd_minor"
	QuotaUnitRequests QuotaUnit = "requests"
	QuotaUnitTokens   QuotaUnit = "tokens"
)

// QuotaSeverityNormal and related constants define the supported contract values.
const (
	QuotaSeverityNormal   QuotaSeverity = "normal"
	QuotaSeverityWarning  QuotaSeverity = "warning"
	QuotaSeverityCritical QuotaSeverity = "critical"
)

// WireAPIResponses and related constants define the supported contract values.
const (
	WireAPIResponses       WireAPI = "responses"
	WireAPIChatCompletions WireAPI = "chat-completions"
)
