package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

// LineBytes is the byte limit after which a log line continues on the next line.
const LineBytes = 4096

// LineSplitter cuts arbitrary byte chunks into nonempty log lines at newlines
// or LineBytes. Its zero value is ready to use.
type LineSplitter struct{}

// Push returns the lines chunk completes, retaining an unfinished line.
func (s *LineSplitter) Push(chunk []byte) []string { panic("not written: r-process") }

// Finish returns the final unterminated nonempty line, or nil.
func (s *LineSplitter) Finish() *string { panic("not written: r-process") }
