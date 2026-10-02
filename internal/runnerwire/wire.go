package runnerwire

import (
	"fmt"
	"reflect"
)

// DecodeInbound decodes and validates one backend message, refusing trailing bytes.
func DecodeInbound(data []byte) (Inbound, error) { return DecodeInboundMsgpack(data) }

// DecodeOutbound decodes and validates one runner message, refusing trailing bytes.
func DecodeOutbound(data []byte) (Outbound, error) { return DecodeOutboundMsgpack(data) }

// Encode validates and encodes a wire message. The sender applies the size
// limit with WithinLimit, so an oversized reply fails its own request.
func Encode(message any) ([]byte, error) {
	switch message := message.(type) {
	case Inbound:
		return EncodeInboundMsgpack(message)
	case Outbound:
		return EncodeOutboundMsgpack(message)
	default:
		return nil, fmt.Errorf("not a runner message: %T", message)
	}
}

// WithinLimit returns the reply, or asks refuse to build that request's error
// when the encoded reply exceeds MaxMessageBytes.
func WithinLimit(reply []byte, refuse func(string) ([]byte, error)) ([]byte, error) {
	if len(reply) <= MaxMessageBytes {
		return reply, nil
	}
	return refuse(fmt.Sprintf("the reply is %d bytes, over the %d-byte message limit", len(reply), MaxMessageBytes))
}

// FSRequestID returns the id of an fs call, which its reply carries.
func FSRequestID(message Inbound) (string, bool) {
	request, ok := message.(interface{ fsRequestID() string })
	if !ok || reflect.ValueOf(request).IsNil() {
		return "", false
	}
	return request.fsRequestID(), true
}

// GitRequestID returns the id of a working-tree call, which its reply carries.
func GitRequestID(message Inbound) (string, bool) {
	request, ok := message.(interface{ gitRequestID() string })
	if !ok || reflect.ValueOf(request).IsNil() {
		return "", false
	}
	return request.gitRequestID(), true
}

func (request *FSReadFile) fsRequestID() string { return request.ID }

func (request *FSWriteFile) fsRequestID() string { return request.ID }

func (request *FSExists) fsRequestID() string { return request.ID }

func (request *FSStat) fsRequestID() string { return request.ID }

func (request *FSLstat) fsRequestID() string { return request.ID }

func (request *FSReaddir) fsRequestID() string { return request.ID }

func (request *FSMkdir) fsRequestID() string { return request.ID }

func (request *FSRm) fsRequestID() string { return request.ID }

func (request *FSCp) fsRequestID() string { return request.ID }

func (request *FSMv) fsRequestID() string { return request.ID }

func (request *FSChmod) fsRequestID() string { return request.ID }

func (request *FSSymlink) fsRequestID() string { return request.ID }

func (request *FSLink) fsRequestID() string { return request.ID }

func (request *FSReadlink) fsRequestID() string { return request.ID }

func (request *FSRealpath) fsRequestID() string { return request.ID }

func (request *FSUtimes) fsRequestID() string { return request.ID }

func (request *GitChangesMessage) gitRequestID() string { return request.ID }

func (request *GitShow) gitRequestID() string { return request.ID }
