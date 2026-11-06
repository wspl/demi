---
status: proposed
---

# A command declares what its output holds and what a failure leaves, never why it fails

## Problem

Each command leaf may carry two texts, `success_output` and
`failure_output`, which `--help` prints as `Output:` and `Failure:`. No
definition says what they hold, and the 37 and 25 of them hold different
things. One `Failure:` says what a failure looks like and leaves:
"a browser error on stderr with a nonzero exit status; an action is
never replayed automatically". Another lists the causes it can think of:
no model given, no provider that makes audio, an entry that cannot make
it now, and more. A list of causes is never complete, goes stale as the
code changes, and says nothing the error itself does not say when it
happens.

The SDK is about to let a program declare commands, and would copy the
two fields as they are.

## Decision

The two texts become `output` and `failure`, each with one meaning:

- `output`: what the command writes to standard output when it succeeds,
  such as "the conversation started, its title and id".
- `failure`: what a failure looks like and what it leaves: where the error
  goes and in what form, the exit status, what is already done, and the
  state left behind, such as "an error on stderr and exit status 1; the
  pages sent before it stay sent". It names no cause.

A cause belongs where it can be acted on: an argument's description says
what the argument must be, and the error says which cause happened, when
it happens. Each of the 25 `failure` texts is rewritten so; each cause it
listed moves into the description of the argument it concerns, or is
checked to be what the command's error says when it happens, and the
error is fixed where it does not.

`--help` keeps its labels, `Output:` and `Failure:`. The leaf's `output`
already holds the JSON Schema of the command's `--json` form; the text
joins it there, so one field says what the output is. The leaf's fields,
its builder's methods and its wire form change together:

```rust
pub struct Leaf<B = Binding> {
    pub name: String,
    pub summary: String,
    pub failure: Option<String>,
    pub output: Option<LeafOutput>,
    // ...
}

pub struct LeafOutput {
    pub description: Option<String>,
    pub json: Option<Schema>,
}
```

`description` is the name OpenAPI gives a response's text beside its
schema. Declarations travel to runners (`runner-protocol` reads them), so
the wire's `successOutput` becomes `output.description` and
`failureOutput` becomes `failure`. A runner of an earlier release updates
before its socket opens, as the server's 409 makes it.

## Built on

| What the design uses | Where it is today |
| --- | --- |
| The two texts | `command-declarations`: `Leaf`, `RawLeaf`, `LeafOutput`; `host-interface`: `LeafBuilder::success_output`, `failure_output`, `json_output` |
| Their rendering | `command-declarations/src/help.rs`, `Output:` and `Failure:` |
| Success and failure themselves | the exit status of `Reply::Exit` |
| A runner's update first | the 409 with the required release |

## Spike

None: the change is names and texts.

## Prior art

- **POSIX utility descriptions**: each utility's page has STDOUT, STDERR,
  EXIT STATUS and CONSEQUENCES OF ERRORS, which say what is written and
  what an error leaves, and leave the causes to the error messages.
- **Unix manual pages**: EXIT STATUS lists the codes and what each
  means, not every way to reach one.

## Alternatives

- **Keep the names and add the definitions.** `success_output` reads as
  the output itself, and `failure_output` as the output of a failure,
  which is the reading that produced the lists of causes.
- **Drop `failure`.** A failure's consequences, such as what is already
  sent, are what an agent needs before it runs a command again, and only
  the command knows them.

## Open questions

- None.
