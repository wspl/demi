package plugin

import (
	"errors"

	"github.com/wspl/demi/internal/host"
)

func (e *ErrorUsage) Error() string              { return e.Message }
func (e *ErrorRefused) Error() string            { return e.Message }
func (e *ErrorFailed) Error() string             { return e.Message }
func (e *ErrorEnded) Error() string              { return e.Message }
func (e *ErrorPort) Error() string               { return e.Refusal.Error() }
func (e *ErrorPort) Unwrap() error               { return e.Refusal }
func (e *PortRefusalHost) Error() string         { return e.Message }
func (e *PortRefusalOperation) Error() string    { return "the operation failed: " + e.Stderr }
func (*PortRefusalConflict) Error() string       { return "the value changed since it was read" }
func (e *PortRefusalExpose) Error() string       { return e.Message }
func (*PortRefusalNoConversation) Error() string { return "the request has no conversation" }
func (*PortRefusalNotRunning) Error() string     { return "the conversation's Host is not running" }

// Undeclared answers a request for a contribution the manifest does not declare.
func Undeclared(what string) Error { return &ErrorFailed{Message: "the plugin declares no " + what} }

// RequestError preserves usage, cancellation and port refusal classifications.
func RequestError(err error) Error {
	var pluginError Error
	if errors.As(err, &pluginError) {
		return pluginError
	}
	var rpc *host.RPCError
	if errors.As(err, &rpc) && rpc.Kind == host.Usage {
		return &ErrorUsage{Message: rpc.Message}
	}
	var port *host.PortError
	if errors.As(err, &port) && port.Kind == host.PortEnded {
		return &ErrorEnded{Message: port.Message}
	}
	var refusal PortRefusal
	if errors.As(err, &refusal) {
		return &ErrorPort{Refusal: refusal}
	}
	return &ErrorFailed{Message: err.Error()}
}

// RPCError converts a plugin failure for an rpc handler.
func RPCError(err Error) error {
	switch e := err.(type) {
	case *ErrorUsage:
		return &host.RPCError{Kind: host.Usage, Message: e.Message, Err: e}
	case *ErrorEnded:
		return &host.PortError{Kind: host.PortEnded, Message: e.Message, Err: e}
	case *ErrorRefused, *ErrorFailed, *ErrorPort:
		return &host.RPCError{Kind: host.HandlerFailed, Message: err.Error(), Err: err}
	}
	return nil
}
