package todo

//revive:disable:exported
// Contract doc comments are product text: contractgen emits them as schema descriptions.

//go:generate go run github.com/wspl/demi/tools/contractgen

// +demi:enum pending in_progress done
type Status string

const (
	Pending    Status = "pending"
	InProgress Status = "in_progress"
	Done       Status = "done"
)

type Item struct {
	ID     string `json:"id"`
	Text   string `json:"text"`
	Status Status `json:"status"`
}

// The input of `demi todo add`.
// +demi:root
// +demi:schema
type AddArgs struct {
	// Todo text
	Text string `json:"text"`
}

// The input of `demi todo update`.
// +demi:root
// +demi:schema
type UpdateArgs struct {
	// Todo id
	ID string `json:"id"`
	// Replacement text
	Text *string `json:"text,omitempty"`
	// Replacement status
	Status *Status `json:"status,omitempty"`
}

// The input of `demi todo done`.
// +demi:root
// +demi:schema
type DoneArgs struct {
	// Todo id
	ID string `json:"id"`
}

// What `demi todo list --json` prints.
// +demi:root
// +demi:schema
// +demi:tolerant
type List struct {
	Todos []Item `json:"todos"`
}

// What the other commands print with `--json`: the todo they changed.
// +demi:root
// +demi:schema
// +demi:tolerant
type OneTodo struct {
	Todo Item `json:"todo"`
}

// +demi:root
type storedTodos []Item
