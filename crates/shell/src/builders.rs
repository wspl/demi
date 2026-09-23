//! Declaration builders: a leaf's input and `--json` output schemas are
//! derived from Rust types with the settings every declaration uses, so the
//! schema a runner enforces and the type a handler decodes are one
//! definition (`commands.md` § Declare a command).

use std::{future::Future, marker::PhantomData, rc::Rc};

use demi_command_tree::{
    Group, Leaf, LeafKind, LeafOutput, NativeOperation, Node, Schema, command_schema_settings,
};
use futures_util::future::{LocalBoxFuture, ready};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{Declared, RpcError, RpcHandler, RpcInvocation, RpcPort};

/// Builds a command group.
pub struct GroupBuilder {
    name: String,
    summary: String,
    children: Vec<Declared>,
}

impl GroupBuilder {
    pub fn new(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            children: Vec::new(),
        }
    }

    pub fn leaf(mut self, leaf: LeafBuilder) -> Self {
        self.children.push(leaf.into());
        self
    }

    pub fn group(mut self, group: GroupBuilder) -> Self {
        self.children.push(group.into());
        self
    }
}

impl From<GroupBuilder> for Declared {
    fn from(group: GroupBuilder) -> Self {
        let mut subcommands = Vec::new();
        let mut handlers = Vec::new();
        let mut error = None;
        for child in group.children {
            for (path, handler) in child.handlers {
                handlers.push(([vec![group.name.clone()], path].concat(), handler));
            }
            error = error.or(child.error);
            subcommands.push(child.tree);
        }
        Declared {
            tree: Node::Group(Group {
                name: group.name,
                summary: group.summary,
                subcommands,
            }),
            handlers,
            error,
        }
    }
}

/// Builds a command: an `rpc` leaf whose handler runs in the backend, or a
/// native leaf naming a package operation.
pub struct LeafBuilder {
    leaf: Leaf<NativeOperation>,
    handler: Option<Rc<dyn RpcHandler>>,
    error: Option<String>,
}

impl LeafBuilder {
    /// An `rpc` leaf; [`LeafBuilder::bind`] gives it its handler.
    pub fn rpc(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self::with_kind(name.into(), summary.into(), LeafKind::Rpc)
    }

    /// A leaf that runs `operation` of a native package.
    pub fn native(
        name: impl Into<String>,
        summary: impl Into<String>,
        operation: NativeOperation,
    ) -> Self {
        Self::with_kind(name.into(), summary.into(), LeafKind::Native(operation))
    }

    fn with_kind(name: String, summary: String, kind: LeafKind<NativeOperation>) -> Self {
        Self {
            leaf: Leaf {
                name,
                summary,
                success_output: None,
                failure_output: None,
                running_hint: None,
                input: None,
                positionals: None,
                stdin_field: None,
                rest_field: None,
                output: None,
                kind,
            },
            handler: None,
            error: None,
        }
    }

    /// The command's input: the schema of `A`, whose doc comments describe
    /// its fields.
    pub fn input<A: JsonSchema>(mut self) -> Self {
        match schema_of::<A>(false) {
            Ok(schema) => self.leaf.input = Some(schema),
            Err(error) => self.refuse(error),
        }
        self
    }

    /// Replaces a field's description, for a text known only when the
    /// command is built.
    pub fn describe(mut self, field: &str, description: impl Into<String>) -> Self {
        let Some(input) = &self.leaf.input else {
            self.refuse(format!("describes {field} before its input"));
            return self;
        };
        let mut value = input.value().clone();
        let Some(property) = value
            .get_mut("properties")
            .and_then(|properties| properties.get_mut(field))
            .and_then(Value::as_object_mut)
        else {
            self.refuse(format!("describes {field}, which its input does not have"));
            return self;
        };
        property.insert("description".into(), description.into().into());
        match Schema::new(value) {
            Ok(schema) => self.leaf.input = Some(schema),
            Err(error) => self.refuse(error.to_string()),
        }
        self
    }

    /// The fields given as positional arguments, in order.
    pub fn positionals<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.leaf.positionals = Some(fields.into_iter().map(Into::into).collect());
        self
    }

    /// The string field read from finite standard input.
    pub fn stdin_field(mut self, field: impl Into<String>) -> Self {
        self.leaf.stdin_field = Some(field.into());
        self
    }

    /// The string array field that receives the tokens after `--`.
    pub fn rest_field(mut self, field: impl Into<String>) -> Self {
        self.leaf.rest_field = Some(field.into());
        self
    }

    pub fn success_output(mut self, text: impl Into<String>) -> Self {
        self.leaf.success_output = Some(text.into());
        self
    }

    pub fn failure_output(mut self, text: impl Into<String>) -> Self {
        self.leaf.failure_output = Some(text.into());
        self
    }

    /// The guidance the model sees in place of the generic running hint
    /// while this command runs.
    pub fn running_hint(mut self, text: impl Into<String>) -> Self {
        self.leaf.running_hint = Some(text.into());
        self
    }

    /// The command prints `T` as JSON when the caller passes `--json`.
    pub fn json_output<T: JsonSchema>(mut self) -> Self {
        match schema_of::<T>(true) {
            Ok(json) => self.leaf.output = Some(LeafOutput { json: Some(json) }),
            Err(error) => self.refuse(error),
        }
        self
    }

    /// The handler of an `rpc` leaf.
    pub fn bind(mut self, handler: impl RpcHandler + 'static) -> Self {
        self.handler = Some(Rc::new(handler));
        self
    }

    /// Keeps the first refusal; registration reports it.
    fn refuse(&mut self, error: String) {
        self.error.get_or_insert(error);
    }
}

impl From<LeafBuilder> for Declared {
    fn from(leaf: LeafBuilder) -> Self {
        let name = leaf.leaf.name.clone();
        Declared {
            tree: Node::Leaf(leaf.leaf),
            handlers: leaf
                .handler
                .map(|handler| (vec![name], handler))
                .into_iter()
                .collect(),
            error: leaf.error,
        }
    }
}

/// The schema of `T` as a declaration carries it: an input's for
/// deserializing, an output's for serializing.
fn schema_of<T: JsonSchema>(output: bool) -> Result<Schema, String> {
    let settings = command_schema_settings();
    let settings = if output {
        settings.for_serialize()
    } else {
        settings
    };
    let value = settings
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value();
    let Value::Object(object) = value else {
        return Err(format!(
            "the schema of {} is not an object",
            T::schema_name()
        ));
    };
    Schema::new(object.into_iter().collect()).map_err(|error| error.to_string())
}

/// A call as a typed handler receives it: the arguments decoded into `A`,
/// and the rest of the invocation.
pub struct Call<A> {
    pub args: A,
    pub invocation: RpcInvocation,
}

/// An [`RpcHandler`] over typed arguments. Dispatch validated the arguments
/// against the schema `A` derives, and `A` decodes with serde's derived rules
/// only, so a value that does not decode is a declaration bug, reported as
/// the call's failure.
pub struct TypedRpc<A, F> {
    run: F,
    arguments: PhantomData<fn() -> A>,
}

impl<A, F, Fut> TypedRpc<A, F>
where
    A: DeserializeOwned,
    F: Fn(Call<A>, RpcPort) -> Fut,
    Fut: Future<Output = Result<u8, RpcError>> + 'static,
{
    pub fn new(run: F) -> Self {
        Self {
            run,
            arguments: PhantomData,
        }
    }
}

impl<A, F, Fut> RpcHandler for TypedRpc<A, F>
where
    A: DeserializeOwned,
    F: Fn(Call<A>, RpcPort) -> Fut,
    Fut: Future<Output = Result<u8, RpcError>> + 'static,
{
    fn call(
        &self,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'_, Result<u8, RpcError>> {
        match serde_json::from_value::<A>(Value::Object(invocation.args.clone())) {
            Ok(args) => Box::pin((self.run)(Call { args, invocation }, port)),
            Err(error) => Box::pin(ready(Err(RpcError::Failed(format!(
                "the arguments of \"{}\" do not decode as declared: {error}",
                invocation.path.join(" ")
            ))))),
        }
    }
}
