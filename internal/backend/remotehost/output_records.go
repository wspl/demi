package remotehost

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// DecodeOutput reads kept-output records in the runner wire's encoding.
func DecodeOutput(bytes []byte, missing *host.Missing) (host.WholeOutput, error) {
	records, err := runnerwire.DecodeRecords(bytes)
	if err != nil {
		return host.WholeOutput{}, err
	}
	output := host.WholeOutput{Missing: missing}
	for _, record := range records {
		switch record := record.(type) {
		case *runnerwire.KeptOutput:
			output.Records = append(
				output.Records,
				host.OutputRecord{Stream: core.StreamKind(record.Stream), Bytes: record.Bytes},
			)
		case *runnerwire.KeptLeftOut:
			output.Records = append(output.Records, host.OutputRecord{LeftOut: new(record.Bytes)})
		}
	}
	return output, nil
}

// EncodeOutput writes whole output as the runner wire's kept-output records.
func EncodeOutput(output host.WholeOutput) ([]byte, error) {
	var data []byte
	for _, record := range output.Records {
		var kept runnerwire.KeptRecord
		if record.LeftOut != nil {
			kept = &runnerwire.KeptLeftOut{Bytes: *record.LeftOut}
		} else {
			kept = &runnerwire.KeptOutput{Stream: runnerwire.OutputStream(record.Stream), Bytes: record.Bytes}
		}
		encoded, err := runnerwire.EncodeRecord(kept)
		if err != nil {
			return nil, err
		}
		data = append(data, encoded...)
	}
	return data, nil
}
