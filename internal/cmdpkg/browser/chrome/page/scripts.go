package page

import _ "embed"

//go:embed element-state.js
var scriptElementState string

//go:embed fill.js
var scriptFill string

//go:embed focus.js
var scriptFocus string

//go:embed focused.js
var scriptFocused string

//go:embed locator.js
var scriptLocator string

//go:embed probe.js
var scriptProbe string

//go:embed read-only.js
var scriptReadOnly string

//go:embed select-options.js
var scriptSelectOptions string

//go:embed select-text.js
var scriptSelectText string
