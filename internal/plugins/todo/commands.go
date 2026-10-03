package todo

import (
	"context"
	"fmt"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/host"
)

const storageKey = "todos.json"

func list(ctx context.Context, invocation host.RPCInvocation, port host.RPCPort) (uint8, error) {
	reply, err := port.Storage(ctx, &host.StorageRead{Key: storageKey})
	if err != nil {
		return 0, err
	}
	value, ok := reply.(*host.StorageValue)
	if !ok {
		return 0, &host.RPCError{Kind: host.HandlerFailed, Message: "reading todos.json answered no value"}
	}
	todos := storedTodos{}
	if len(value.Value) != 0 && !contract.IsNull(value.Value) {
		todos, err = decodeStoredTodos(value.Value)
		if err != nil {
			return 0, fmt.Errorf("stored %s is unreadable: %w", storageKey, err)
		}
	}
	var output []byte
	if invocation.JSON {
		output, err = (TodoList{Todos: todos}).MarshalJSON()
	} else if len(todos) == 0 {
		output = []byte("No todos.\n")
	} else {
		var text strings.Builder
		for _, todo := range todos {
			text.WriteString(line(todo))
			text.WriteByte('\n')
		}
		output = []byte(text.String())
	}
	if err != nil {
		return 0, err
	}
	return 0, port.Stdout(ctx, output)
}

func add(ctx context.Context, call host.Call[AddArgs], port host.RPCPort) (uint8, error) {
	todos, err := host.Update(
		ctx,
		port,
		storageKey,
		decodeStoredTodos,
		storedTodos.MarshalJSON,
		func(current storedTodos, found bool) (storedTodos, error) {
			todos := storedTodos{}
			if found {
				todos = current
			}
			todos = append(todos, TodoItem{ID: nextID(todos), Text: call.Args.Text, Status: Pending})
			return todos, nil
		},
	)
	if err != nil {
		return 0, err
	}
	return write(ctx, port, call.Invocation.JSON, todos[len(todos)-1])
}

func update(ctx context.Context, call host.Call[UpdateArgs], port host.RPCPort) (uint8, error) {
	todo, err := change(ctx, port, call.Args.ID, func(todo *TodoItem) {
		if call.Args.Text != nil {
			todo.Text = *call.Args.Text
		}
		if call.Args.Status != nil {
			todo.Status = *call.Args.Status
		}
	})
	if err != nil {
		return 0, err
	}
	return write(ctx, port, call.Invocation.JSON, todo)
}

func done(ctx context.Context, call host.Call[DoneArgs], port host.RPCPort) (uint8, error) {
	todo, err := change(ctx, port, call.Args.ID, func(todo *TodoItem) { todo.Status = Done })
	if err != nil {
		return 0, err
	}
	return write(ctx, port, call.Invocation.JSON, todo)
}

// change commits the requested todo edit through the node's revisioned storage.
func change(ctx context.Context, port host.RPCPort, id string, edit func(*TodoItem)) (TodoItem, error) {
	var changed TodoItem
	_, err := host.Update(
		ctx,
		port,
		storageKey,
		decodeStoredTodos,
		storedTodos.MarshalJSON,
		func(current storedTodos, found bool) (storedTodos, error) {
			if found {
				for i := range current {
					if current[i].ID == id {
						edit(&current[i])
						changed = current[i]
						return current, nil
					}
				}
			}
			return nil, &host.RPCError{Kind: host.HandlerFailed, Message: fmt.Sprintf("Todo not found: %s", id)}
		},
	)
	return changed, err
}

// write prints the committed todo in the invocation's selected format.
func write(ctx context.Context, port host.RPCPort, asJSON bool, todo TodoItem) (uint8, error) {
	output := []byte(line(todo) + "\n")
	if asJSON {
		var err error
		output, err = (OneTodo{Todo: todo}).MarshalJSON()
		if err != nil {
			return 0, err
		}
	}
	return 0, port.Stdout(ctx, output)
}

// nextID allocates one more than the largest numeric T id in the list.
func nextID(todos []TodoItem) string {
	var largest uint64
	for _, todo := range todos {
		suffix, ok := strings.CutPrefix(todo.ID, "T")
		if !ok {
			continue
		}
		n, err := strconv.ParseUint(strings.TrimPrefix(suffix, "+"), 10, 64)
		if err == nil && n > largest {
			largest = n
		}
	}
	return "T" + strconv.FormatUint(largest+1, 10)
}

// line prints a todo's status mark, id and text.
func line(todo TodoItem) string {
	mark := " "
	switch todo.Status {
	case Pending:
	case InProgress:
		mark = "-"
	case Done:
		mark = "x"
	}
	return fmt.Sprintf("[%s] %s %s", mark, todo.ID, todo.Text)
}
