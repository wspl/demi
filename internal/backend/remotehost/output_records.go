package remotehost

import (
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
)

// DecodeOutput reads kept-output records in the runner wire's encoding.
func DecodeOutput(bytes []byte, missing *host.Missing) (host.WholeOutput, error) {
	records, err := runnerproto.DecodeRecords(bytes)
	if err != nil {
		return host.WholeOutput{}, err
	}
	output := host.WholeOutput{Missing: missing}
	for _, record := range records {
		switch record := record.(type) {
		case *runnerproto.KeptOutput:
			output.Records = append(
				output.Records,
				host.OutputRecord{Stream: types.StreamKind(record.Stream), Bytes: record.Bytes},
			)
		case *runnerproto.KeptLeftOut:
			output.Records = append(output.Records, host.OutputRecord{LeftOut: new(record.Bytes)})
		}
	}
	return output, nil
}

// EncodeOutput writes whole output as the runner wire's kept-output records.
func EncodeOutput(output host.WholeOutput) ([]byte, error) {
	var data []byte
	for _, record := range output.Records {
		var kept runnerproto.KeptRecord
		if record.LeftOut != nil {
			kept = &runnerproto.KeptLeftOut{Bytes: *record.LeftOut}
		} else {
			kept = &runnerproto.KeptOutput{Stream: runnerproto.OutputStream(record.Stream), Bytes: record.Bytes}
		}
		encoded, err := runnerproto.EncodeRecord(kept)
		if err != nil {
			return nil, err
		}
		data = append(data, encoded...)
	}
	return data, nil
}
