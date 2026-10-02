package plugin

import "github.com/wspl/demi/internal/webapi"

// ExposeAddress carries webapi's address codec through contract generation.
// webapi.ExposeAddress lacks a codec marker and generated Validate method;
// embedding delegates to its existing codec without duplicating its rules.
// +demi:codec
type ExposeAddress struct{ webapi.ExposeAddress }
