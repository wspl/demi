package todo

//revive:disable:exported
// Contract descriptions are verbatim Rust product text.

//go:generate go run github.com/wspl/demi/tools/contractgen

// +demi:enum pending in_progress done
type TodoStatus string

const (
	Pending    TodoStatus = "pending"
	InProgress TodoStatus = "in_progress"
	Done       TodoStatus = "done"
)

type TodoItem struct {
	ID     string     `json:"id"`
	Text   string     `json:"text"`
	Status TodoStatus `json:"status"`
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
	Status *TodoStatus `json:"status,omitempty"`
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
type TodoList struct {
	Todos []TodoItem `json:"todos"`
}

// What the other commands print with `--json`: the todo they changed.
// +demi:root
// +demi:schema
// +demi:tolerant
type OneTodo struct {
	Todo TodoItem `json:"todo"`
}

// +demi:root
type storedTodos []TodoItem
