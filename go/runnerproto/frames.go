package runnerproto

import "fmt"

// WithinLimit turns an oversized reply into the error for its own request.
// Encoding does not apply the limit: the sender owns the request's fallback.
func WithinLimit(reply []byte, refuse func(string) ([]byte, error)) ([]byte, error) {
	if len(reply) <= MaxMessageBytes {
		return reply, nil
	}
	return refuse(fmt.Sprintf("the reply is %d bytes, over the %d-byte message limit", len(reply), MaxMessageBytes))
}

// FSRequestID returns the correlation ID of a FS request.
func FSRequestID(message Inbound) (string, bool) {
	switch v := message.(type) {
	case InboundFSReadFile:
		return v.ID, true
	case *InboundFSReadFile:
		if v != nil {
			return v.ID, true
		}
	case InboundFSWriteFile:
		return v.ID, true
	case *InboundFSWriteFile:
		if v != nil {
			return v.ID, true
		}
	case InboundFSExists:
		return v.ID, true
	case *InboundFSExists:
		if v != nil {
			return v.ID, true
		}
	case InboundFSStat:
		return v.ID, true
	case *InboundFSStat:
		if v != nil {
			return v.ID, true
		}
	case InboundFSLstat:
		return v.ID, true
	case *InboundFSLstat:
		if v != nil {
			return v.ID, true
		}
	case InboundFSReaddir:
		return v.ID, true
	case *InboundFSReaddir:
		if v != nil {
			return v.ID, true
		}
	case InboundFSMkdir:
		return v.ID, true
	case *InboundFSMkdir:
		if v != nil {
			return v.ID, true
		}
	case InboundFSRm:
		return v.ID, true
	case *InboundFSRm:
		if v != nil {
			return v.ID, true
		}
	case InboundFSCp:
		return v.ID, true
	case *InboundFSCp:
		if v != nil {
			return v.ID, true
		}
	case InboundFSMv:
		return v.ID, true
	case *InboundFSMv:
		if v != nil {
			return v.ID, true
		}
	case InboundFSChmod:
		return v.ID, true
	case *InboundFSChmod:
		if v != nil {
			return v.ID, true
		}
	case InboundFSSymlink:
		return v.ID, true
	case *InboundFSSymlink:
		if v != nil {
			return v.ID, true
		}
	case InboundFSLink:
		return v.ID, true
	case *InboundFSLink:
		if v != nil {
			return v.ID, true
		}
	case InboundFSReadlink:
		return v.ID, true
	case *InboundFSReadlink:
		if v != nil {
			return v.ID, true
		}
	case InboundFSRealpath:
		return v.ID, true
	case *InboundFSRealpath:
		if v != nil {
			return v.ID, true
		}
	case InboundFSUtimes:
		return v.ID, true
	case *InboundFSUtimes:
		if v != nil {
			return v.ID, true
		}
	}
	return "", false
}

// GitRequestID returns the correlation ID of a Git request.
func GitRequestID(message Inbound) (string, bool) {
	switch v := message.(type) {
	case InboundGitChanges:
		return v.ID, true
	case *InboundGitChanges:
		if v != nil {
			return v.ID, true
		}
	case InboundGitShow:
		return v.ID, true
	case *InboundGitShow:
		if v != nil {
			return v.ID, true
		}
	}
	return "", false
}
