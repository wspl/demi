package browserproto

// What a failure knows beyond its code and message.
// +demi:tolerant
// +demi:root
type ErrorDetails struct {
	Action *ActionProgress `json:"action,omitempty"`
	Tab    *string         `json:"tab,omitempty"`
	// The tab's URL when the action failed.
	URL *string `json:"url,omitempty"`
	// The element condition that was not met.
	Condition *string `json:"condition,omitempty"`
	// What intercepted the pointer instead of the target.
	Interceptor *string `json:"interceptor,omitempty"`
	// How many elements an ambiguous target matched.
	Count *uint `json:"count,omitempty"`
	// How many characters `type` delivered before it failed.
	Delivered *uint `json:"delivered,omitempty"`
	// The numbers of the agents whose debugging connections hold the tab
	// when a command on it times out.
	DebuggingCallers *[]uint64 `json:"debuggingCallers,omitempty"`
	// What an export wrote before it failed or was interrupted.
	*AssetsExportResult
}

// A failed operation, or a failed item of a batch such as a page of
// `content.fetch`.
// +demi:root
type BrowserFailure struct {
	Code    BrowserErrorCode `json:"code"`
	Message string           `json:"message"`
	Details *ErrorDetails    `json:"details,omitempty"`
}

// What a failed operation writes to stderr under `--json`.
// +demi:root
type FailureDocument struct {
	Error BrowserFailure `json:"error"`
}

// One URL `content.fetch` read, or its failure.
// +demi:root
type FetchedPage struct {
	RequestedURL string          `json:"requestedUrl"`
	URL          string          `json:"url"`
	Title        string          `json:"title"`
	Content      string          `json:"content"`
	Error        *BrowserFailure `json:"error,omitempty"`
}

// +demi:schema
// +demi:root
type ContentFetchResult struct {
	Pages     []FetchedPage `json:"pages"`
	Truncated bool          `json:"truncated"`
}
