package server

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"

	"github.com/wspl/demi/internal/host"
)

// commandContract keeps a command's generated schema and decoder together.
// Non-generic constructors let contractgen type-check generic command bindings
// before the generated declarations exist.
type commandContract[A any] struct {
	input  json.RawMessage
	output json.RawMessage
	decode func([]byte) (A, error)
}

func spawnContract() commandContract[spawnArgs] {
	return commandContract[spawnArgs]{spawnArgsJSONSchema(), startedJSONSchema(), decodeSpawnArgs}
}
func sendContract() commandContract[sendArgs] {
	return commandContract[sendArgs]{sendArgsJSONSchema(), sentJSONSchema(), decodeSendArgs}
}
func abortContract() commandContract[abortArgs] {
	return commandContract[abortArgs]{abortArgsJSONSchema(), abortedJSONSchema(), decodeAbortArgs}
}
func resumeContract() commandContract[resumeArgs] {
	return commandContract[resumeArgs]{resumeArgsJSONSchema(), startedJSONSchema(), decodeResumeArgs}
}
func listContract() commandContract[listArgs] {
	return commandContract[listArgs]{nil, listingJSONSchema(), decodeListArgs}
}
func showContract() commandContract[showArgs] {
	return commandContract[showArgs]{showArgsJSONSchema(), shownJSONSchema(), decodeShowArgs}
}
func shellOutputContract() commandContract[outputArgs] {
	return commandContract[outputArgs]{input: outputArgsJSONSchema(), decode: decodeOutputArgs}
}

// reserveStart makes a request-id name exactly one durable child round.
func reserveStart(ctx context.Context, port host.RPCPort, request string, fresh startReceipt) (startReceipt, error) {
	return host.Update(ctx, port, "agent.start."+request, decodeStartReceipt, func(v startReceipt) ([]byte, error) { return v.MarshalJSON() }, func(current *startReceipt) (startReceipt, error) {
		if current == nil {
			return fresh, nil
		}
		if !reflect.DeepEqual(current.Input, fresh.Input) {
			return startReceipt{}, errors.New("request-id already belongs to different agent arguments")
		}
		return *current, nil
	})
}
