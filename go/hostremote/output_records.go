package hostremote

import (
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

// EncodeOutput writes the runner's concatenated MessagePack kept-output format.
func EncodeOutput(output *shell.WholeOutput) ([]byte, error) {
	var data []byte
	for _, record := range output.Records() {
		var kept runnerproto.KeptRecord
		switch record := record.(type) {
		case shell.OutputRead:
			kept = runnerproto.KeptOutput{Stream: runnerproto.OutputStream(record.Stream), Bytes: record.Bytes}
		case shell.OutputLeftOut:
			kept = runnerproto.KeptLeftOut{Bytes: uint64(record)}
		}
		encoded, err := runnerproto.EncodeRecord(kept)
		if err != nil {
			return nil, err
		}
		data = append(data, encoded...)
	}
	return data, nil
}

// DecodeOutput validates the disk format and bounds before publishing records.
func DecodeOutput(data []byte, missing *shell.Missing) (*shell.WholeOutput, error) {
	kept, err := runnerproto.DecodeRecords(data)
	if err != nil {
		return nil, err
	}
	records := make([]shell.OutputRecord, 0, len(kept))
	for _, record := range kept {
		switch record := record.(type) {
		case runnerproto.KeptOutput:
			records = append(records, shell.OutputRead{Stream: core.StreamKind(record.Stream), Bytes: record.Bytes})
		case runnerproto.KeptLeftOut:
			records = append(records, shell.OutputLeftOut(record.Bytes))
		}
	}
	return shell.NewWholeOutput(records, missing), nil
}
