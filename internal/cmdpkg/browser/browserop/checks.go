package browserop

import (
	"fmt"

	"github.com/wspl/demi/internal/contract"
)

// These garde checks were not present in Rust's JSON schemas for optional
// vectors. Keep them Go-only, as required by the browser migration ruling.
func validateSelectInput(input SelectInput) error {
	for _, field := range []struct {
		name   string
		values *[]string
	}{{"value", input.Value}, {"option-label", input.OptionLabel}} {
		if field.values == nil {
			continue
		}
		for i, value := range *field.values {
			if err := contract.Text(value, 0, StdinBytes, ""); err != nil {
				return contract.At(fmt.Sprintf("%s[%d]", field.name, i), err)
			}
		}
	}
	return nil
}

func validateCdpEventsInput(input CdpEventsInput) error {
	return validateLocatorItems("method", input.Method)
}

func validateAssetsExportInput(input AssetsExportInput) error {
	return validateLocatorItems("id", input.ID)
}

// validateLocatorItems applies the browser locator limit to an optional list.
func validateLocatorItems(field string, values *[]string) error {
	if values == nil {
		return nil
	}
	for i, value := range *values {
		if err := contract.Text(value, 1, LocatorLength, ""); err != nil {
			return contract.At(fmt.Sprintf("%s[%d]", field, i), err)
		}
	}
	return nil
}
