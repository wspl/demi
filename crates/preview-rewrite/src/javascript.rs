//! JavaScript rewriting: the proxy runtime takes over what the page reads as its own
//! location, top and messages, and module addresses go through the proxy. The source is
//! patched in place, so everything else keeps its exact text and position.

use oxc::allocator::Allocator;
use std::collections::HashMap;

use oxc::ast::ast::{
    Argument, ArrowFunctionExpression, AssignmentExpression, AssignmentTarget, AssignmentTargetProperty,
    AssignmentTargetPropertyIdentifier, BindingPattern, CallExpression, ChainElement, ChainExpression,
    ComputedMemberExpression, ExportAllDeclaration, ExportFromDeclaration, Expression, Function, FunctionBody,
    IdentifierReference, ImportDeclaration, ImportExpression, NewExpression, ObjectProperty, Program, StaticBlock,
    StaticMemberExpression, StringLiteral, VariableDeclarator,
};
use oxc::ast_visit::{Visit, walk};
use oxc::parser::{ParseOptions, Parser};
use oxc::semantic::{Scoping, SemanticBuilder};
use oxc::span::{GetSpan, SourceType, Span};
use oxc::syntax::scope::ScopeFlags;
use string_wizard::{Hires, MagicString, SourceMapOptions};

use crate::address::{Context, map_module_specifier};
use crate::boot::{WorkerScript, worker_prelude};

/// Members that read or change a Location; on any object they go through the runtime,
/// which leaves objects that are not a Location alone.
const LOCATION_MEMBERS: [&str; 13] =
    ["href", "origin", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "assign", "replace", "reload", "toString"];
const TEMPORARIES: &str = "var __proxyObject, __proxyKey;";

/// Where the rewritten script's source map goes.
#[derive(Clone, Copy, Debug)]
pub enum SourceMapMode<'o> {
    /// Appended to the code as a data URL.
    Inline,
    /// No map: small snippets such as event handlers.
    Omitted,
    /// A `sourceMappingURL` pointing where the map is served on request.
    External(&'o str),
    /// Returned beside the code, for composing with the page's own map.
    Returned,
}

/// Rewritten code, and its map when the options asked for it to be returned.
#[derive(Debug)]
pub struct Rewritten {
    pub code: String,
    pub map: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ScriptOptions<'o> {
    /// The script's logical URL, or a label for code without one.
    pub filename: &'o str,
    /// The base for dynamic `import()` specifiers; the filename when it is a URL.
    pub base: Option<&'o str>,
    /// What `import.meta.url` reads, for a module run from a generated address.
    pub module_url: Option<&'o str>,
    pub source_map: SourceMapMode<'o>,
    /// An event handler attribute's body may `return`.
    pub handler: bool,
    /// A worker's own script (not a module it imports), which starts the worker's runtime.
    pub worker: Option<WorkerScript<'o>>,
}

#[derive(Debug)]
pub struct ParseError(pub String);

/// One function's computed-member temporaries: where their declaration goes and whether
/// the function needs them. An arrow with an expression body becomes a block.
struct Frame {
    insert_at: u32,
    arrow_expression: Option<Span>,
    used: bool,
}

/// Members a destructuring pattern reads through [`Rewriter::pattern_source`].
const PATTERN_ADAPTED: [&str; 2] = ["location", "top"];

struct Rewriter<'s, 'c> {
    source: &'s str,
    scoping: &'c Scoping,
    edits: MagicString<'s>,
    context: &'c Context,
    options: ScriptOptions<'c>,
    frames: Vec<Frame>,
    /// Member accesses on an optional chain's spine, by span, and whether an optional link
    /// before them can short-circuit their object.
    chain_spine: HashMap<(u32, u32), bool>,
    failure: Option<String>,
}

impl<'s> Rewriter<'s, '_> {
    fn record(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            self.failure.get_or_insert(error);
        }
    }

    /// Text that opens a wrapper; inner wrappers added later open inside it. A wrapper
    /// starting with a name must not join a keyword before it (`return(x)`).
    fn open(&mut self, at: u32, text: String) {
        let joins = at > 0 && self.source.as_bytes()[at as usize - 1].is_ascii_alphanumeric() || at > 0 && matches!(self.source.as_bytes()[at as usize - 1], b'_' | b'$');
        let text = if joins { format!(" {text}") } else { text };
        let result = self.edits.append_right(at, text).map(|_| ());
        self.record(result);
    }

    /// Text that closes a wrapper; inner wrappers added later close before it.
    fn close(&mut self, at: u32, text: String) {
        let result = self.edits.prepend_left(at, text).map(|_| ());
        self.record(result);
    }

    fn replace(&mut self, span: Span, text: String) {
        let result = self.edits.update(span.start, span.end, text).map(|_| ());
        self.record(result);
    }

    fn unresolved(&self, identifier: &IdentifierReference) -> bool {
        self.scoping.get_reference(identifier.reference_id()).symbol_id().is_none()
    }

    /// The runtime name an unbound global identifier reads, if the proxy takes it over.
    fn global_replacement(&self, identifier: &IdentifierReference) -> Option<&'static str> {
        if !self.unresolved(identifier) {
            return None;
        }
        match identifier.name.as_str() {
            "location" => Some("__proxyLocation"),
            "top" if self.scoping.get_reference(identifier.reference_id()).is_read() => Some("__proxyTop"),
            _ => None,
        }
    }

    /// Whether a member's object can be short-circuited by an earlier optional link of the
    /// chain it is on. Wrapping such an object in an adapter ends the short circuit there,
    /// so the member itself must become optional.
    fn short_circuited(&self, member: Span) -> bool {
        self.chain_spine.get(&(member.start, member.end)).copied().unwrap_or(false)
    }

    /// Wrap the object of a member access in a runtime adapter.
    fn adapt_object(&mut self, adapter: &str, member: Span, object: Span, property_start: u32, optional: bool) {
        self.open(object.start, format!("{adapter}("));
        self.close(object.end, ")".into());
        if !optional && self.short_circuited(member) {
            self.replace(Span::new(object.end, property_start), "?.".into());
        }
    }

    /// `const { location } = window` reads the Location as a member read would: the runtime
    /// gives the pattern a source whose `location` and `top` are the logical ones.
    fn pattern_source(&mut self, source: Span) {
        self.open(source.start, "__proxyPatternSource(".into());
        self.close(source.end, ")".into());
    }

    fn member_adapter(name: &str) -> Option<&'static str> {
        match name {
            "location" => Some("__proxyLocationOwner"),
            "postMessage" => Some("__proxyMessageOwner"),
            "eval" => Some("__proxyScriptOwner"),
            "top" => Some("__proxyFrameOwner"),
            name if LOCATION_MEMBERS.contains(&name) => Some("__proxyLocationMemberOwner"),
            _ => None,
        }
    }

    fn base(&self) -> Option<&str> {
        self.options.base.or_else(|| url::Url::parse(self.options.filename).ok().map(|_| self.options.filename))
    }

    fn map_specifier(&mut self, literal: &StringLiteral) {
        let specifier = literal.value.as_str();
        let mapped = map_module_specifier(specifier, self.base(), self.context);
        if mapped != specifier {
            self.replace(literal.span, serde_json::to_string(&mapped).unwrap_or_default());
        }
    }

    /// A block body's temporaries go after its opening brace and directives.
    fn enter_body(&mut self, body: &FunctionBody) {
        let insert_at = body.directives.last().map_or(body.span.start + 1, |directive| directive.span.end);
        self.frames.push(Frame { insert_at, arrow_expression: None, used: false });
    }

    fn leave_function(&mut self) {
        let Some(frame) = self.frames.pop() else { return };
        if !frame.used {
            return;
        }
        match frame.arrow_expression {
            Some(expression) => {
                let result = self.edits.prepend_right(expression.start, format!("{{ {TEMPORARIES} return ")).map(|_| ());
                self.record(result);
                let result = self.edits.append_left(expression.end, " }").map(|_| ());
                self.record(result);
            }
            None => {
                let result = self.edits.prepend_right(frame.insert_at, TEMPORARIES).map(|_| ());
                self.record(result);
            }
        }
    }
}

/// The member accesses along an optional chain, innermost first, each with whether an
/// optional link before it can short-circuit its object. Arguments and computed keys are
/// not on the chain.
fn chain_spine(chain: &ChainExpression) -> Vec<((u32, u32), bool)> {
    let mut links = Vec::new();
    let mut next = match &chain.expression {
        ChainElement::CallExpression(call) => {
            links.push((None, call.optional));
            Some(&call.callee)
        }
        element => element.as_member_expression().map(|member| {
            links.push((Some(member.span()), member.optional()));
            member.object()
        }),
    };
    while let Some(expression) = next {
        next = match expression {
            Expression::CallExpression(call) => {
                links.push((None, call.optional));
                Some(&call.callee)
            }
            expression => expression.as_member_expression().map(|member| {
                links.push((Some(member.span()), member.optional()));
                member.object()
            }),
        };
    }
    let mut spine = Vec::new();
    let mut optional_before = false;
    for (span, optional) in links.into_iter().rev() {
        if let Some(span) = span {
            spine.push(((span.start, span.end), optional_before));
        }
        optional_before |= optional;
    }
    spine
}

/// A computed key the rewriter can read without running the program.
fn constant_key(expression: &Expression) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::NumericLiteral(literal) => Some(literal.value.to_string()),
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => {
            template.quasis.first().and_then(|quasi| quasi.value.cooked.as_ref()).map(ToString::to_string)
        }
        _ => None,
    }
}

impl<'a> Visit<'a> for Rewriter<'_, '_> {
    fn visit_program(&mut self, program: &Program<'a>) {
        let insert_at = program.directives.last().map(|directive| directive.span.end).or_else(|| program.hashbang.as_ref().map(|hashbang| hashbang.span.end)).unwrap_or(0);
        self.frames.push(Frame { insert_at, arrow_expression: None, used: false });
        walk::walk_program(self, program);
        self.leave_function();
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        let Some(body) = &function.body else { return walk::walk_function(self, function, flags) };
        self.enter_body(body);
        walk::walk_function(self, function, flags);
        self.leave_function();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        match arrow.body.as_function_body() {
            Some(body) => self.enter_body(body),
            None => {
                let expression = arrow.body.span();
                self.frames.push(Frame { insert_at: expression.start, arrow_expression: Some(expression), used: false });
            }
        }
        walk::walk_arrow_function_expression(self, arrow);
        self.leave_function();
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        let opening = self.source[block.span.start as usize..].find('{').map_or(block.span.start, |offset| block.span.start + offset as u32 + 1);
        self.frames.push(Frame { insert_at: opening, arrow_expression: None, used: false });
        walk::walk_static_block(self, block);
        self.leave_function();
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if let Some(replacement) = self.global_replacement(identifier) {
            self.replace(identifier.span, replacement.into());
        }
    }

    fn visit_object_property(&mut self, property: &ObjectProperty<'a>) {
        if property.shorthand
            && let Expression::Identifier(identifier) = &property.value
            && self.global_replacement(identifier).is_some()
        {
            self.open(identifier.span.start, format!("{}: ", identifier.name));
        }
        walk::walk_object_property(self, property);
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let (BindingPattern::ObjectPattern(pattern), Some(init)) = (&declarator.id, &declarator.init)
            && pattern.properties.iter().any(|property| !property.computed && property.key.static_name().is_some_and(|name| PATTERN_ADAPTED.contains(&name.as_ref())))
        {
            self.pattern_source(init.span());
        }
        walk::walk_variable_declarator(self, declarator);
    }

    fn visit_assignment_expression(&mut self, assignment: &AssignmentExpression<'a>) {
        if let AssignmentTarget::ObjectAssignmentTarget(target) = &assignment.left
            && target.properties.iter().any(|property| match property {
                AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(property) => PATTERN_ADAPTED.contains(&property.binding.name.as_str()),
                AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => !property.computed && property.name.static_name().is_some_and(|name| PATTERN_ADAPTED.contains(&name.as_ref())),
            })
        {
            self.pattern_source(assignment.right.span());
        }
        walk::walk_assignment_expression(self, assignment);
    }

    fn visit_assignment_target_property_identifier(&mut self, property: &AssignmentTargetPropertyIdentifier<'a>) {
        if self.global_replacement(&property.binding).is_some() {
            self.open(property.binding.span.start, format!("{}: ", property.binding.name));
        }
        walk::walk_assignment_target_property_identifier(self, property);
    }

    fn visit_chain_expression(&mut self, chain: &ChainExpression<'a>) {
        self.chain_spine.extend(chain_spine(chain));
        walk::walk_chain_expression(self, chain);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        if matches!(member.object, Expression::ImportMeta(_)) && member.property.name == "url" {
            let replacement = match self.options.module_url {
                Some(url) => serde_json::to_string(url).unwrap_or_default(),
                None => format!("__proxyLogicalUrl({})", &self.source[member.span.start as usize..member.span.end as usize]),
            };
            return self.replace(member.span, replacement);
        }
        if !matches!(member.object, Expression::Super(_))
            && let Some(adapter) = Self::member_adapter(&member.property.name)
        {
            self.adapt_object(adapter, member.span, member.object.span(), member.property.span.start, member.optional);
        }
        walk::walk_static_member_expression(self, member);
    }

    fn visit_computed_member_expression(&mut self, member: &ComputedMemberExpression<'a>) {
        if matches!(member.object, Expression::Super(_)) {
            return walk::walk_computed_member_expression(self, member);
        }
        let object = member.object.span();
        let property = member.expression.span();
        if let Some(key) = constant_key(&member.expression) {
            if let Some(adapter) = Self::member_adapter(&key) {
                self.open(object.start, format!("{adapter}("));
                self.close(object.end, ")".into());
            }
            return walk::walk_computed_member_expression(self, member);
        }
        // An optional chain evaluates the key only when the object is there; the runtime's
        // wrapper would evaluate it first. Short-circuiting keys stay with the browser, at
        // the cost of a dynamic `window?.[key]` reaching the real Location.
        if member.optional || self.short_circuited(member.span) {
            return walk::walk_computed_member_expression(self, member);
        }
        // The key is known only when the program runs: the runtime decides then whether
        // the member is a Location's. Each value is read right after it is stored.
        if let Some(frame) = self.frames.last_mut() {
            frame.used = true;
        }
        // A key written as `a[x, y]` becomes an argument, where its commas would separate
        // arguments; it keeps them inside parentheses.
        let sequence = matches!(member.expression, Expression::SequenceExpression(_));
        let (open_key, close_key) = if sequence { ("(", ")") } else { ("", "") };
        self.open(object.start, "__proxyPropertyOwner(__proxyObject = ".into());
        self.replace(Span::new(object.end, property.start), format!(", __proxyKey = __proxyPropertyKey(__proxyObject, {open_key}"));
        self.replace(Span::new(property.end, member.span.end), format!("{close_key}))[__proxyKey]"));
        walk::walk_computed_member_expression(self, member);
    }

    fn visit_new_expression(&mut self, new: &NewExpression<'a>) {
        // `new a[k](x)` constructs `a[k]`; a wrapper call written in front of the callee
        // would be what `new` binds to, so a member callee keeps its own parentheses.
        if new.callee.is_member_expression() {
            let span = new.callee.span();
            self.open(span.start, "(".into());
            self.close(span.end, ")".into());
        }
        walk::walk_new_expression(self, new);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &call.callee
            && callee.name == "eval"
            && self.unresolved(callee)
            && let Some(first) = call.arguments.first()
            && !matches!(first, Argument::SpreadElement(_))
        {
            let span = first.span();
            self.open(span.start, "__proxyRewriteJavaScript(".into());
            self.close(span.end, ")".into());
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
        let span = import.source.span();
        let base = self.base().map(|base| format!(", {}", serde_json::to_string(base).unwrap_or_default())).unwrap_or_default();
        self.open(span.start, "__proxyModuleSpecifier(".into());
        self.close(span.end, format!("{base})"));
        walk::walk_import_expression(self, import);
    }

    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        self.map_specifier(&declaration.source);
        walk::walk_import_declaration(self, declaration);
    }

    fn visit_export_from_declaration(&mut self, declaration: &ExportFromDeclaration<'a>) {
        self.map_specifier(&declaration.source);
        walk::walk_export_from_declaration(self, declaration);
    }

    fn visit_export_all_declaration(&mut self, declaration: &ExportAllDeclaration<'a>) {
        self.map_specifier(&declaration.source);
        walk::walk_export_all_declaration(self, declaration);
    }
}

fn source_map_comment(text: &str) -> bool {
    let trimmed = text.trim_start_matches(['/', '*']).trim_start();
    trimmed.starts_with("# sourceMappingURL=") || trimmed.starts_with("@ sourceMappingURL=")
}

/// Rewrite one script or module. Source that does not parse is reported, not repaired.
pub fn rewrite_javascript(source: &str, options: ScriptOptions, context: &Context) -> Result<String, ParseError> {
    rewrite_javascript_with_map(source, options, context).map(|rewritten| rewritten.code)
}

/// Rewrite one script or module, returning its map when `options.source_map` is `Returned`.
pub fn rewrite_javascript_with_map(source: &str, options: ScriptOptions, context: &Context) -> Result<Rewritten, ParseError> {
    let allocator = Allocator::default();
    let parse_options = ParseOptions { allow_return_outside_function: options.handler, ..ParseOptions::default() };
    let parsed = Parser::new(&allocator, source, SourceType::unambiguous()).with_options(parse_options).parse();
    if parsed.fatal_error || !parsed.diagnostics.is_empty() {
        let message = parsed.diagnostics.first().map_or_else(|| "unparseable script".to_owned(), ToString::to_string);
        return Err(ParseError(message));
    }
    let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
    let mut rewriter = Rewriter {
        source,
        scoping: semantic.scoping(),
        edits: MagicString::new(source),
        context,
        options,
        frames: Vec::new(),
        chain_spine: HashMap::new(),
        failure: None,
    };
    rewriter.visit_program(&parsed.program);
    // The page's own map describes the original text; the engine serves one for the result.
    for comment in parsed.program.comments.iter() {
        let span = comment.span;
        if source_map_comment(&source[span.start as usize..span.end as usize]) {
            let result = rewriter.edits.remove(span.start, span.end).map(|_| ());
            rewriter.record(result);
        }
    }
    if let Some(failure) = rewriter.failure {
        return Err(ParseError(format!("rewrite produced overlapping edits: {failure}")));
    }
    // Last, so that the boot data carries every label the script maps.
    if let Some(worker) = options.worker {
        rewriter.edits.prepend(format!("{}\n", worker_prelude(context, options.filename, worker)));
    }
    let code = rewriter.edits.to_string();
    let map = || rewriter.edits.source_map(SourceMapOptions { include_content: true, source: options.filename.into(), hires: Hires::Boundary });
    Ok(match options.source_map {
        SourceMapMode::Omitted => Rewritten { code, map: None },
        SourceMapMode::External(url) => Rewritten { code: format!("{code}\n//# sourceMappingURL={url}"), map: None },
        SourceMapMode::Inline => Rewritten { code: format!("{code}\n//# sourceMappingURL={}", map().to_data_url()), map: None },
        SourceMapMode::Returned => Rewritten { map: Some(map().to_json_string()), code },
    })
}

/// The kind of function `new Function(...)` and its relatives create.
#[derive(Clone, Copy, Debug)]
pub enum FunctionKind {
    Plain,
    Async,
    Generator,
    AsyncGenerator,
}

/// Rewrite the parameters and body given to the Function constructor, which compiles them
/// as `function anonymous(<parameters>\n) {\n<body>\n}`. Returns them in the same two parts.
pub fn rewrite_function(parameters: &str, body: &str, kind: FunctionKind, context: &Context) -> Result<(String, String), ParseError> {
    let prefix = match kind {
        FunctionKind::Plain => "function",
        FunctionKind::Async => "async function",
        FunctionKind::Generator => "function*",
        FunctionKind::AsyncGenerator => "async function*",
    };
    let opening = format!("({prefix} anonymous({parameters}\n) {{\n");
    let source = format!("{opening}{body}\n}})");
    let options = ScriptOptions { filename: "dynamic-function.js", base: None, module_url: None, source_map: SourceMapMode::Omitted, handler: false, worker: None };
    let rewritten = rewrite_javascript(&source, options, context)?;
    // Parameters and body changed only inside themselves: split the result at the same
    // landmarks as the source, found again by parsing it.
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &rewritten, SourceType::script()).parse();
    let Some(oxc::ast::ast::Statement::ExpressionStatement(statement)) = parsed.program.body.first() else {
        return Err(ParseError("Function constructor source did not parse back".into()));
    };
    let Expression::ParenthesizedExpression(parenthesized) = &statement.expression else {
        return Err(ParseError("Function constructor source did not parse back".into()));
    };
    let Expression::FunctionExpression(function) = &parenthesized.expression else {
        return Err(ParseError("Function constructor source did not parse back".into()));
    };
    let parameters_span = function.params.span;
    let parameters = rewritten[parameters_span.start as usize + 1..parameters_span.end as usize - 1].trim_end_matches('\n').to_owned();
    let body_span = function.body.as_ref().map(|body| body.span).ok_or_else(|| ParseError("Function constructor source has no body".into()))?;
    let inner = &rewritten[body_span.start as usize + 1..body_span.end as usize - 1];
    let body = inner.strip_prefix('\n').unwrap_or(inner);
    let body = body.strip_suffix('\n').unwrap_or(body).to_owned();
    Ok((parameters, body))
}
