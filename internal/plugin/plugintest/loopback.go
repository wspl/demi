package plugintest

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

// Loopback sends every plugin request, reply and port message through JSON.
func Loopback(p plugin.Plugin) plugin.Plugin {
	return loopback{p}
}

// Port constructs a fresh in-memory plugin port using rpc for command operations.
func Port(rpc host.PortTransport) plugin.Port {
	return WithRPC(rpc).Port()
}

type loopback struct{ plugin plugin.Plugin }

// Call round-trips the request and reply through their JSON contracts.
func (l loopback) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	request, err := across(plugin.RequestJSON{Value: request}, plugin.DecodeRequest)
	if err != nil {
		return nil, err
	}
	reply, err := l.plugin.Call(ctx, request, plugin.NewPort(jsonMessages{port}))
	if err != nil {
		value, wireErr := across(plugin.ErrorJSON{Value: plugin.RequestError(err)}, plugin.DecodeError)
		if wireErr != nil {
			return nil, wireErr
		}
		return nil, value
	}
	return across(plugin.ReplyJSON{Value: reply}, plugin.DecodeReply)
}

type jsonMessages struct{ port plugin.Port }

// Request round-trips the port message and answer through their JSON contracts.
func (j jsonMessages) Request(ctx context.Context, message plugin.PortMessage) (plugin.PortAnswer, error) {
	message, err := across(plugin.PortMessageJSON{Value: message}, plugin.DecodePortMessage)
	if err != nil {
		return nil, err
	}
	answer, err := j.port.Forward(ctx, message)
	if err != nil {
		return nil, err
	}
	return across(plugin.PortAnswerJSON{Value: answer}, plugin.DecodePortAnswer)
}

// across detaches one plugin message by encoding and decoding its wire contract.
func across[T any](value json.Marshaler, decode func([]byte) (T, error)) (T, error) {
	var zero T
	data, err := value.MarshalJSON()
	if err != nil {
		return zero, fmt.Errorf("encode plugin loopback: %w", err)
	}
	result, err := decode(data)
	if err != nil {
		return zero, fmt.Errorf("decode plugin loopback: %w", err)
	}
	return result, nil
}
