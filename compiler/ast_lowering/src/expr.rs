//! Expression lowering — AST expression nodes → HIR [`Expr`].

use ast::{NodeIndex, NodeKind};
use hir::{
    body::Body,
    common::{
        Arg, BinOp, Ident, Lit, LitKind, Path, PathAnchor, PathSegment, Symbol, TyParam,
        TyParamKind, UnOp,
    },
    decl::LetDecl,
    expr::{Block, ClosureParam, CondictionArm, Expr, ExprKind},
    pattern::{Pattern, PatternArm, PatternKind},
};
use resolve::{BindingKind, RibKind};
use rustc_span::Span;

use crate::LoweringContext;

#[derive(Debug, PartialEq, Eq)]
enum LiteralDecodeError {
    Invalid(&'static str),
    UnsupportedInterpolation,
}

/// Decode source spelling without treating decoded dollar signs as syntax.
fn decode_quoted_literal(text: &str, quote: char) -> Result<String, LiteralDecodeError> {
    use LiteralDecodeError::{Invalid, UnsupportedInterpolation};

    let body = text
        .strip_prefix(quote)
        .and_then(|text| text.strip_suffix(quote))
        .ok_or(Invalid("literal is missing its surrounding quotes"))?;
    let mut chars = body.chars().peekable();
    let mut value = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let escaped = match chars.next().ok_or(Invalid("incomplete literal escape"))? {
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '0' => '\0',
                    '$' => '$',
                    'x' => {
                        if chars.next() != Some('{') {
                            return Err(Invalid("Unicode escape must use \\x{...}"));
                        }
                        let mut scalar = 0;
                        let mut digits = 0;
                        loop {
                            match chars.next() {
                                Some('}') => break,
                                Some(digit) if digit.is_ascii_hexdigit() => {
                                    digits += 1;
                                    if digits > 6 {
                                        return Err(Invalid(
                                            "Unicode escape exceeds six hex digits",
                                        ));
                                    }
                                    scalar = scalar * 16 + digit.to_digit(16).unwrap();
                                }
                                Some(_) => return Err(Invalid("invalid Unicode escape digit")),
                                None => return Err(Invalid("unterminated Unicode escape")),
                            }
                        }
                        if digits == 0 {
                            return Err(Invalid("Unicode escape requires at least one hex digit"));
                        }
                        char::from_u32(scalar)
                            .ok_or(Invalid("Unicode escape is not a Unicode scalar value"))?
                    }
                    _ => return Err(Invalid("unsupported literal escape")),
                };
                value.push(escaped);
            }
            '$' if quote == '"' && chars.peek().is_some_and(|&ch| begins_interpolation(ch)) => {
                return Err(UnsupportedInterpolation);
            }
            ch if ch == quote => return Err(Invalid("unescaped quote in literal")),
            ch => value.push(ch),
        }
    }
    Ok(value)
}

fn begins_interpolation(ch: char) -> bool {
    // Match lexer identifier starts, excluding its dedicated `$` and `@` tokens.
    ch.is_alphabetic()
        || matches!(ch, '{' | '_' | '\u{1F000}'..='\u{1F9FF}' | '\u{2600}'..='\u{26FF}' | '\u{2700}'..='\u{27BF}')
}

fn decode_character_literal(text: &str) -> Result<char, LiteralDecodeError> {
    let value = decode_quoted_literal(text, '\'')?;
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Ok(ch),
        _ => Err(LiteralDecodeError::Invalid(
            "character literal must contain exactly one Unicode scalar value",
        )),
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ExprContext {
    // Phase follows the associated declaration's eventual type/phase contract.
    AssociatedDefault,
    // comptime contexts
    Type,
    Trait,
    CastType, // this syntax is no just used for type casts, so needed to distinguish from general Type context
    ClauseParamType,
    FunctionParamType,
    ConstDefValue,
    Attribute,
    Configuration,
    ComptimeFunctionBody,
    ComptimeArgument,
    InlineBranchCondition,
    InlineForLoopIterator,
    ComptimeProposition,
    EnumVariantConstant,
    PatternConstant,

    // runtime contexts
    StructFieldDefault,
    OptionalParameterDefault,
    RuntimeFunctionBody,
    RuntimeProposition,
    QuoteArgument,
    Location,
    AtomicBlockRequiredResource,

    // others
    ReachabilityAnnotation,
}

fn binary_operator(kind: NodeKind) -> Option<BinOp> {
    match kind {
        NodeKind::AddAdd => Some(BinOp::Concat),
        NodeKind::Add => Some(BinOp::Add),
        NodeKind::Sub => Some(BinOp::Sub),
        NodeKind::Mul => Some(BinOp::Mul),
        NodeKind::Div => Some(BinOp::Div),
        NodeKind::Mod => Some(BinOp::Rem),
        NodeKind::BoolEq => Some(BinOp::Eq),
        NodeKind::BoolNotEq => Some(BinOp::Ne),
        NodeKind::BoolGt => Some(BinOp::Gt),
        NodeKind::BoolGtEq => Some(BinOp::Ge),
        NodeKind::BoolLt => Some(BinOp::Lt),
        NodeKind::BoolLtEq => Some(BinOp::Le),
        NodeKind::BoolAnd => Some(BinOp::And),
        NodeKind::BoolOr => Some(BinOp::Or),
        _ => None,
    }
}

impl<'hir, 'ast, 'resolver: 'ast> LoweringContext<'hir, 'ast, 'resolver> {
    /// Lower an AST node in expression position into an HIR [`Expr`].
    pub(crate) fn lower_expr(&mut self, ctx: ExprContext, node: NodeIndex) -> Expr<'hir> {
        let kind = self.ast.get_node(node).expect("valid expression node").0;
        if binary_operator(kind).is_some() || matches!(kind, NodeKind::Negative | NodeKind::BoolNot)
        {
            self.lower_operator_expr(ctx, node)
        } else {
            self.lower_non_operator_expr(ctx, node)
        }
    }

    #[inline(never)]
    fn lower_non_operator_expr(&mut self, ctx: ExprContext, node: NodeIndex) -> Expr<'hir> {
        let (kind, span, children) = self.ast.get_node(node).expect("valid expression node");
        match kind {
            NodeKind::ContextualExprType | NodeKind::ContextualPatternType => {
                let subject = (kind == NodeKind::ContextualPatternType)
                    .then(|| self.lower_expr(ExprContext::Type, children[0]));
                let scope = self.local_enter(resolve::RibKind::MatchBody);
                let mut bindings = Vec::new();
                for &entry in self.ast.get_multi_child_slice(children[1]).unwrap_or(&[]) {
                    let parts = self.ast.get_children(entry);
                    let name = self.node_to_ident(parts[0]);
                    let default = if kind == NodeKind::ContextualExprType
                        && self.resolve(name).is_ok_and(|resolved| {
                            matches!(
                                resolved.kind,
                                BindingKind::Local(_)
                                    | BindingKind::ExpandedLocal(..)
                                    | BindingKind::FnParam(_)
                            )
                        }) {
                        let value = self.lower_expr(ctx, parts[0]);
                        Some(&*self.arena.alloc_expr(value))
                    } else {
                        None
                    };
                    let ty = self.lower_expr(ExprContext::Type, parts[1]);
                    let hir_id = self.next_hir_id();
                    self.define_local(name, entry, hir_id);
                    bindings.push(hir::expr::CodeContextBinding {
                        hir_id,
                        name,
                        ty: self.arena.alloc_expr(ty),
                        default,
                    });
                }
                let result =
                    subject.unwrap_or_else(|| self.lower_expr(ExprContext::Type, children[0]));
                self.local_exit(scope);
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: if kind == NodeKind::ContextualPatternType {
                        ExprKind::ContextualPatternType(self.arena.alloc_expr(result), bindings)
                    } else {
                        ExprKind::ContextualExprType(bindings, self.arena.alloc_expr(result))
                    },
                }
            }
            NodeKind::QuotePattern => {
                let scope = self.local_enter(resolve::RibKind::MatchBody);
                let pattern = self.lower_pattern(children[0]);
                self.bind_arm_pattern(children[0], &pattern);
                let substitutions = self
                    .ast
                    .get_multi_child_slice(children[1])
                    .unwrap_or(&[])
                    .iter()
                    .map(|&entry| {
                        let parts = self.ast.get_children(entry);
                        let port = self.node_to_ident(parts[0]);
                        let binding = self.lower_expr(ctx, parts[1]);
                        hir::expr::CodeSubstitution {
                            port,
                            binding: self.arena.alloc_expr(binding),
                        }
                    })
                    .collect();
                self.local_exit(scope);
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::CodePatternQuote(
                        self.arena.alloc_pattern(pattern),
                        substitutions,
                    ),
                }
            }
            NodeKind::QuoteExpr | NodeKind::SpliceExpr => {
                let body = self.lower_expr(ctx, children[0]);
                let mut substitutions: Vec<hir::expr::CodeSubstitution<'hir>> = self
                    .ast
                    .get_multi_child_slice(children[1])
                    .unwrap_or(&[])
                    .iter()
                    .map(|&entry| {
                        let parts = self.ast.get_children(entry);
                        let port = self.node_to_ident(parts[0]);
                        let binding = self.lower_expr(ctx, parts[1]);
                        hir::expr::CodeSubstitution {
                            port,
                            binding: self.arena.alloc_expr(binding),
                        }
                    })
                    .collect();
                if kind == NodeKind::SpliceExpr {
                    if let ExprKind::ResolvedClauseParam(_, definition) = body.kind {
                        if let Some(ports) = self.test_template_ports.get(&definition).cloned() {
                            let mut explicit = std::collections::HashSet::new();
                            for entry in &substitutions {
                                if !explicit.insert(entry.port.name) {
                                    self.emit_malformed(
                                        "duplicate test template splice port",
                                        entry.port.span,
                                    );
                                }
                                if !ports.iter().any(|port| port.name == entry.port.name) {
                                    self.emit_malformed(
                                        "unknown test template splice port",
                                        entry.port.span,
                                    );
                                }
                                if !matches!(
                                    entry.binding.kind,
                                    ExprKind::ResolvedLocal(..) | ExprKind::ResolvedFnParam(..)
                                ) {
                                    self.emit_malformed("test template substitution requires a local binding, not a computed value or global declaration", entry.binding.span);
                                }
                            }
                            for port in ports {
                                if substitutions
                                    .iter()
                                    .any(|entry| entry.port.name == port.name)
                                {
                                    continue;
                                }
                                let name = hir::Ident::new(port.name, span);
                                let binding_kind =
                                    self.resolve(name)
                                        .ok()
                                        .and_then(|resolution| match resolution.kind {
                                            BindingKind::Local(site) => {
                                                self.ast_ref_to_hir_id.get(&site).copied().map(
                                                    |id| ExprKind::ResolvedLocal(port.name, id),
                                                )
                                            }
                                            BindingKind::FnParam(site) => {
                                                self.ast_ref_to_hir_id.get(&site).copied().map(
                                                    |id| ExprKind::ResolvedFnParam(port.name, id),
                                                )
                                            }
                                            BindingKind::ExpandedLocal(site, output) => self
                                                .expanded_locals
                                                .get(&(site, output))
                                                .copied()
                                                .map(|id| ExprKind::ResolvedLocal(port.name, id)),
                                            _ => None,
                                        });
                                let Some(binding_kind) = binding_kind else {
                                    self.emit_malformed(&format!("test template splice has no local binding for context port `{}`; supply an explicit substitution", port.name), span);
                                    continue;
                                };
                                let binding = Expr {
                                    hir_id: self.next_hir_id(),
                                    span,
                                    kind: binding_kind,
                                };
                                substitutions.push(hir::expr::CodeSubstitution {
                                    port: name,
                                    binding: self.arena.alloc_expr(binding),
                                });
                            }
                        }
                    }
                }
                let body = self.arena.alloc_expr(body);
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: if kind == NodeKind::QuoteExpr {
                        ExprKind::CodeQuote(body, substitutions)
                    } else {
                        ExprKind::CodeSplice(body, substitutions)
                    },
                }
            }
            NodeKind::Id => self.lower_id_expr(ctx, node),
            NodeKind::SelfLower => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::SelfValue,
                span,
            },
            NodeKind::SelfCap => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TySelf,
                span,
            },
            NodeKind::TakeView => {
                let view_kind = self.node_to_symbol(children[1]);
                match view_kind.as_str() {
                    "builtin" => self.lower_builtin_view_expr(ctx, node),
                    "old" => {
                        let inner_expr = self.lower_expr(ctx, children[0]);
                        let inner_ref = self.arena.alloc_expr(inner_expr);
                        Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::TakeOld(inner_ref),
                            span,
                        }
                    }
                    "tuple_enumerate" => {
                        let value = self.lower_expr(ctx, children[0]);
                        Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::TakeTupleEnumerate(self.arena.alloc_expr(value)),
                            span,
                        }
                    }
                    "intrinsic" => {
                        let tag = self.lower_expr(ctx, children[0]);
                        Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::TakeIntrinsic(self.arena.alloc_expr(tag)),
                            span,
                        }
                    }
                    "tag_name" => {
                        let value = self.lower_expr(ctx, children[0]);
                        Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::TakeTagName(self.arena.alloc_expr(value)),
                            span,
                        }
                    }
                    "bitvec" => {
                        let inner_expr = self.lower_expr(ctx, children[0]);
                        let inner_ref = self.arena.alloc_expr(inner_expr);
                        Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::TakeBitVec(inner_ref),
                            span,
                        }
                    }
                    "type" => {
                        let inner_expr = self.lower_expr(ctx, children[0]);
                        let inner_ref = self.arena.alloc_expr(inner_expr);
                        Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::TypeOf(inner_ref),
                            span,
                        }
                    }
                    "multi_deref" => {
                        let inner_expr = self.lower_expr(ctx, children[0]);
                        let inner_ref = self.arena.alloc_expr(inner_expr);
                        Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::MultiDeref(inner_ref),
                            span,
                        }
                    }
                    _ => {
                        self.emit_unsupported_node(&format!("take view '{view_kind}'"), span);
                        self.make_invalid_expr(span)
                    }
                }
            }
            NodeKind::Projection => {
                let base_expr = self.lower_expr(ctx, children[0]);
                if let ExprKind::NFApplication(head, _) = &base_expr.kind {
                    if let ExprKind::ResolvedDef { def_id, .. } = head.kind {
                        if self
                            .resolver
                            .find_scope_for_def(def_id)
                            .and_then(|id| self.resolver.scope_tree().get(id))
                            .is_some_and(|scope| scope.kind == resolve::scope::ScopeKind::AdtBody)
                        {
                            let field = self.node_to_ident(children[1]);
                            match self.resolver.resolve_member(
                                def_id,
                                field.name,
                                self.file_scope,
                                span,
                            ) {
                                Ok(resolve::Resolution {
                                    kind: BindingKind::Definition(member, _),
                                }) => {
                                    let hir_id = self.next_hir_id();
                                    self.package.member_projections.insert(hir_id, member);
                                    return Expr {
                                        hir_id,
                                        span,
                                        kind: ExprKind::Projection(
                                            self.arena.alloc_expr(base_expr),
                                            field,
                                        ),
                                    };
                                }
                                // Enum variants are resolved from the nominal descriptor.
                                Err(resolve::ResolveError::UnresolvedIdentifier { .. }) => {}
                                Err(error) => {
                                    self.emit_resolve_error(error);
                                    return self.make_invalid_expr(span);
                                }
                                Ok(_) => {
                                    self.emit_malformed("static member is not a declaration", span);
                                    return self.make_invalid_expr(span);
                                }
                            }
                        }
                    }
                }
                if let ExprKind::ResolvedDef { def_id, .. } = base_expr.kind {
                    if self
                        .resolver
                        .find_scope_for_def(def_id)
                        .and_then(|id| self.resolver.scope_tree().get(id))
                        .is_some_and(|scope| {
                            scope.is_module() || scope.kind == resolve::scope::ScopeKind::AdtBody
                        })
                    {
                        let field = self.node_to_ident(children[1]);
                        return match self.resolver.resolve_member(
                            def_id,
                            field.name,
                            self.file_scope,
                            span,
                        ) {
                            Ok(resolve::Resolution {
                                kind: BindingKind::Definition(def_id, _),
                            }) => Expr {
                                hir_id: self.next_hir_id(),
                                kind: ExprKind::ResolvedDef {
                                    name: field.name,
                                    has_clause_param: false,
                                    def_id,
                                },
                                span,
                            },
                            Ok(_) => {
                                self.emit_malformed("static member is not a declaration", span);
                                self.make_invalid_expr(span)
                            }
                            Err(error) => {
                                if matches!(
                                    &error,
                                    resolve::ResolveError::UnresolvedIdentifier { .. }
                                ) && self
                                    .resolver
                                    .find_scope_for_def(def_id)
                                    .and_then(|id| self.resolver.scope_tree().get(id))
                                    .is_some_and(|scope| {
                                        scope.kind == resolve::scope::ScopeKind::AdtBody
                                    })
                                {
                                    // Variants are selected from the completed nominal
                                    // descriptor during elaboration, including forward defs.
                                    return Expr {
                                        hir_id: self.next_hir_id(),
                                        kind: ExprKind::Projection(
                                            self.arena.alloc_expr(base_expr),
                                            field,
                                        ),
                                        span,
                                    };
                                }
                                self.emit_resolve_error(error);
                                self.make_invalid_expr(span)
                            }
                        };
                    }
                }
                let base_ref = self.arena.alloc_expr(base_expr);
                let field_ident = self.node_to_ident(children[1]);
                let hir_id = self.next_hir_id();
                let candidates = self
                    .resolver
                    .accessible_type_members(field_ident.name, self.file_scope);
                self.package
                    .member_access
                    .insert(hir_id, std::sync::Arc::clone(&candidates));
                let head = match base_ref.kind {
                    ExprKind::NFApplication(head, _) => head,
                    _ => base_ref,
                };
                if matches!(head.kind, ExprKind::ResolvedDef { def_id, .. }
                    if self.resolver.find_scope_for_def(def_id)
                        .and_then(|id| self.resolver.scope_tree().get(id))
                        .is_some_and(|scope| scope.kind == resolve::scope::ScopeKind::TypeAlias))
                    || self.package.member_projections.contains_key(&head.hir_id)
                    || self
                        .package
                        .deferred_member_projections
                        .contains_key(&head.hir_id)
                    || matches!(
                        base_ref.kind,
                        ExprKind::Application(..) | ExprKind::ExtendedApplication(..)
                    )
                {
                    self.package
                        .deferred_member_projections
                        .insert(hir_id, candidates);
                }
                Expr {
                    hir_id,
                    kind: ExprKind::Projection(base_ref, field_ident),
                    span,
                }
            }
            NodeKind::Int => self.make_lit_expr(LitKind::Integer(self.source_text(node)), span),
            NodeKind::Real => self.make_lit_expr(LitKind::Float(self.source_text(node)), span),
            NodeKind::Str | NodeKind::StringConcat => self.lower_string_literal(node, span),
            NodeKind::Char => match decode_character_literal(&self.source_text(node)) {
                Ok(value) => self.make_lit_expr(LitKind::Char(value), span),
                Err(error) => self.invalid_literal(error, span),
            },
            NodeKind::LiteralExtension => {
                let literal = self.lower_expr(ctx, children[0]);
                let literal = self.arena.alloc_expr(literal);
                let suffix = self.node_to_ident(children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::LiteralConversion { literal, suffix },
                    span,
                }
            }
            NodeKind::Bool => {
                let text = self.source_text(node);
                let val = text == "true";
                self.make_lit_expr(LitKind::Bool(val), span)
            }
            NodeKind::Symbol => {
                self.make_lit_expr(LitKind::Symbol(self.node_to_symbol(children[0])), span)
            }
            NodeKind::Unit => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Unit,
                span,
            },
            NodeKind::Null => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Null,
                span,
            },
            NodeKind::Undefined => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Undefined,
                span,
            },

            NodeKind::Wildcard => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TyPlaceholder,
                span,
            },
            NodeKind::TraitBound => {
                let subject = self.lower_expr(ctx, children[0]);
                let interface = self.lower_expr(ctx, children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::TraitBound(
                        self.arena.alloc_expr(subject),
                        self.arena.alloc_expr(interface),
                    ),
                    span,
                }
            }
            NodeKind::Refer => {
                let inner = self.lower_expr(ctx, children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Ref(inner_ref),
                    span,
                }
            }
            NodeKind::Deref => {
                let inner = self.lower_expr(ctx, children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Deref(inner_ref),
                    span,
                }
            }
            NodeKind::ErrorNew => {
                let inner = self.lower_expr(ctx, children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::ErrorNew(inner_ref),
                    span,
                }
            }
            NodeKind::Application => {
                self.lower_application_expr(ctx, children[0], children[1], span)
            }
            NodeKind::ExtendedApplication => {
                let callee = self.lower_expr(ctx, children[0]);
                let callee_ref = self.arena.alloc_expr(callee);
                let args = self.lower_extend_args(ctx, children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::ExtendedApplication(callee_ref, args),
                    span,
                }
            }
            NodeKind::NormalFormHeadApplication => {
                let base = self.lower_expr(ctx, children[0]);
                let base_ref = self.arena.alloc_expr(base);
                let args: &'hir [Arg<'hir>] = self.lower_args(ctx, children[1]);

                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::NFApplication(base_ref, args),
                    span,
                }
            }
            NodeKind::RangeFull
            | NodeKind::RangeTo
            | NodeKind::RangeToInclusive
            | NodeKind::RangeFrom
            | NodeKind::RangeFromTo
            | NodeKind::RangeFromToInclusive => {
                let (start, end) = match kind {
                    NodeKind::RangeFull => (None, None),
                    NodeKind::RangeTo | NodeKind::RangeToInclusive => (None, Some(children[0])),
                    NodeKind::RangeFrom => (Some(children[0]), None),
                    _ => (Some(children[0]), Some(children[1])),
                };
                let start = start.map(|node| {
                    let value = self.lower_expr(ctx, node);
                    self.arena.alloc_expr(value) as &Expr<'hir>
                });
                let end = end.map(|node| {
                    let value = self.lower_expr(ctx, node);
                    self.arena.alloc_expr(value) as &Expr<'hir>
                });
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::Range {
                        start,
                        end,
                        inclusive: matches!(
                            kind,
                            NodeKind::RangeToInclusive | NodeKind::RangeFromToInclusive
                        ),
                    },
                }
            }
            NodeKind::IndexApplication => {
                self.lower_index_expr(ctx, children[0], children[1], span)
            }
            NodeKind::Object => {
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[0])
                    .unwrap_or(&[])
                    .to_vec();
                let mut entries = Vec::with_capacity(nodes.len());
                for node in nodes {
                    if self.ast.get_node_kind(node) == Some(NodeKind::Property) {
                        let parts = self.ast.get_children(node);
                        let ident = self.node_to_ident(parts[0]);
                        let value = self.lower_expr(ctx, parts[1]);
                        entries.push(hir::expr::ObjectEntry::Property(hir::expr::FieldExpr {
                            ident,
                            expr: self.arena.alloc_expr(value),
                            span: self.ast.get_span(node).unwrap_or(span),
                        }));
                    } else {
                        let value = self.lower_expr(ctx, node);
                        entries.push(hir::expr::ObjectEntry::Value(self.arena.alloc_expr(value)));
                    }
                }
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::Object(entries),
                }
            }
            NodeKind::Tuple => self.lower_tuple_expr(ctx, children[0], span),
            NodeKind::ListOf => self.lower_list_expr(ctx, children[0], span),
            NodeKind::Block
            | NodeKind::DoBlock
            | NodeKind::UnsafeBlock
            | NodeKind::AsyncBlock
            | NodeKind::ComptimeBlock => {
                if matches!(kind, NodeKind::AsyncBlock | NodeKind::ComptimeBlock) {
                    self.package
                        .pending_semantics
                        .push((span, format!("{kind:?}")));
                }
                let block = self.lower_block(ctx, node);
                let block_ref = self.arena.alloc_block(block);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: if kind == NodeKind::UnsafeBlock {
                        ExprKind::UnsafeBlock(block_ref)
                    } else {
                        ExprKind::Block(block_ref)
                    },
                    span,
                }
            }
            NodeKind::IfStatement => {
                self.lower_if_stmt(ctx, children[0], children[1], children[2], span)
            }
            NodeKind::ErrorQualifiedType => {
                let errors = self.lower_expr(ExprContext::Type, children[0]);
                let result = self.lower_expr(ExprContext::Type, children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::ErrorQualifiedType(
                        self.arena.alloc_expr(errors),
                        self.arena.alloc_expr(result),
                    ),
                }
            }
            NodeKind::ErrorPropagation => {
                let value = self.lower_expr(ctx, children[0]);
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::ErrorPropagate(self.arena.alloc_expr(value)),
                }
            }
            NodeKind::ErrorElimination => {
                let value = self.lower_expr(ctx, children[0]);
                let value = self.arena.alloc_expr(value);
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[1])
                    .unwrap_or(&[])
                    .to_vec();
                let mut catches = Vec::new();
                let mut arms = Vec::new();
                for (index, node) in nodes.iter().copied().enumerate() {
                    let mut head = node;
                    while let Some((
                        NodeKind::Attribute | NodeKind::AttributeSetTrue,
                        _,
                        children,
                    )) = self.ast.get_node(head)
                    {
                        head = children[1];
                    }
                    let is_catch =
                        matches!(self.ast.get_node(head), Some((NodeKind::CatchArm, _, _)));
                    let arm = self.lower_match_arm(ctx, node);
                    if is_catch {
                        catches.push((index, arm));
                    } else {
                        arms.push(arm);
                    }
                }
                let valid = match catches.as_slice() {
                    [] => true,
                    [(index, _)] => *index + 1 == nodes.len(),
                    [(first, _), (last, _)] => *first == 0 && *last + 1 == nodes.len(),
                    _ => false,
                };
                if !valid {
                    self.emit_malformed("error elimination permits one trailing catch, or one leading and one trailing catch", span);
                }
                let mut before = None;
                let mut after = None;
                if catches.len() == 2 {
                    before = Some(&self.arena.alloc_arm_slice(vec![catches.remove(0).1])[0]);
                }
                if let Some((_, arm)) = catches.into_iter().next() {
                    after = Some(&self.arena.alloc_arm_slice(vec![arm])[0]);
                }
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::ErrorHandle {
                        value,
                        before,
                        arms: self.arena.alloc_arm_slice(arms),
                        after,
                    },
                }
            }
            NodeKind::EffectQualifiedType => {
                let effects = self.lower_expr(ExprContext::Type, children[0]);
                let result = self.lower_expr(ExprContext::Type, children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::EffectQualifiedType(
                        self.arena.alloc_expr(effects),
                        self.arena.alloc_expr(result),
                    ),
                    span,
                }
            }
            NodeKind::OptionPropagation => {
                let value = self.lower_expr(ctx, children[0]);
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::OptionPropagate(self.arena.alloc_expr(value)),
                }
            }
            NodeKind::EffectPropagation => {
                let computation = self.lower_expr(ctx, children[0]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::EffectRequest(self.arena.alloc_expr(computation)),
                    span,
                }
            }
            NodeKind::EffectElimination => {
                // Reuse lexical pattern binding, not match evaluation semantics.
                let computation = self.lower_expr(ctx, children[0]);
                let computation = self.arena.alloc_expr(computation);
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[1])
                    .unwrap_or(&[])
                    .to_vec();
                let arms: Vec<_> = nodes
                    .into_iter()
                    .map(|node| self.lower_match_arm(ctx, node))
                    .collect();
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::EffectHandle(computation, self.arena.alloc_arm_slice(arms)),
                    span,
                }
            }
            NodeKind::PostMatch => self.lower_post_match_expr(ctx, children[0], children[1], span),
            NodeKind::Assign => self.lower_assign_expr(ctx, children[0], children[1], span),
            NodeKind::AddAssign
            | NodeKind::SubAssign
            | NodeKind::MulAssign
            | NodeKind::DivAssign
            | NodeKind::RemAssign => {
                self.lower_assign_op_expr(ctx, kind, children[0], children[1], span)
            }
            NodeKind::Assumes => {
                let condition = self.lower_expr(ExprContext::RuntimeProposition, children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Assumes(self.arena.alloc_expr(condition)),
                    span,
                }
            }
            NodeKind::RefinedType => {
                let base = self.lower_expr(ctx, children[1]);
                self.refinement_depth += 1;
                let requirement = self.lower_expr(ExprContext::ComptimeProposition, children[0]);
                self.refinement_depth -= 1;
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::RefinedType {
                        requirement: self.arena.alloc_expr(requirement),
                        base: self.arena.alloc_expr(base),
                    },
                }
            }
            NodeKind::Defer => {
                let value = self.lower_expr(ctx, children[0]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Defer(self.arena.alloc_expr(value)),
                    span,
                }
            }
            NodeKind::ReturnStatement => {
                self.lower_return_stmt(ctx, children[0], children[1], span)
            }
            NodeKind::ResumeStatement => {
                self.lower_resume_stmt(ctx, children[0], children[1], span)
            }
            NodeKind::BreakStatement => self.lower_break_stmt(ctx, children[0], children[1], span),
            NodeKind::ContinueStatement => {
                self.lower_continue_stmt(ctx, children[0], children[1], span)
            }
            NodeKind::TypeCast => {
                let expr = self.lower_expr(ctx, children[0]);
                let ty = self.lower_expr(ExprContext::CastType, children[1]);
                let expr_ref = self.arena.alloc_expr(expr);
                let ty_ref = self.arena.alloc_expr(ty);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Cast(expr_ref, ty_ref),
                    span,
                }
            }
            NodeKind::Lambda => self.lower_lambda_expr(ctx, node, span),
            NodeKind::PostLambda => {
                self.lower_post_lambda_expr(ctx, children[0], children[1], span)
            }
            NodeKind::PointerType => {
                let inner = self.lower_expr(ExprContext::Type, children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::TyPtr(inner_ref),
                    span,
                }
            }
            NodeKind::OptionalType => {
                let inner = self.lower_expr(ExprContext::Type, children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::TyOptional(inner_ref),
                    span,
                }
            }
            NodeKind::ClosureQualifiedType => {
                let context = self.lower_expr(ExprContext::Type, children[0]);
                let body = self.lower_expr(ExprContext::Type, children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::TyClosure(
                        self.arena.alloc_expr(context),
                        self.arena.alloc_expr(body),
                    ),
                }
            }
            NodeKind::ClosureEnvironment => {
                let mut bindings = Vec::new();
                for node in self.ast.get_multi_child_slice(children[0]).unwrap_or(&[]) {
                    let fields = self.ast.get_children(*node);
                    let name = self.node_to_ident(fields[0]);
                    let ty = self.lower_expr(ExprContext::Type, fields[1]);
                    let value = if fields[2] == 0 {
                        None
                    } else {
                        let value = self.lower_expr(ctx, fields[2]);
                        Some(self.arena.alloc_expr(value))
                    };
                    bindings.push(hir::expr::ClosureEnvironmentBinding {
                        name,
                        ty: self.arena.alloc_expr(ty),
                        value,
                    });
                }
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::TyClosureEnvironment(
                        self.arena.alloc_closure_environment_binding_slice(bindings),
                    ),
                }
            }
            NodeKind::FnType => self.lower_fn_type_expr(node, span),
            NodeKind::Arrow => self.lower_arrow_type_expr(children[0], children[1], span),
            NodeKind::ExprStatement => self.lower_expr(ctx, children[0]),
            NodeKind::InlineStatement => {
                let control = self.lower_expr(ctx, children[0]);
                if !matches!(
                    &control.kind,
                    ExprKind::If(..)
                        | ExprKind::When(..)
                        | ExprKind::Match(..)
                        | ExprKind::For { .. }
                ) {
                    self.emit_unsupported_node(
                        "inline requires if, when, pattern matching or for control flow",
                        span,
                    );
                    return self.make_invalid_expr(span);
                }
                Expr {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: ExprKind::InlineControl(self.arena.alloc_expr(control)),
                }
            }

            // ── Desugaring: bool matches ─────────────────────────────────
            // `a matches b`  →  `match a { b => true, _ => false }`
            NodeKind::BoolMatches => {
                self.lower_bool_matches_expr(ctx, children[0], children[1], span)
            }

            // ── Desugaring: if-is-match ──────────────────────────────────
            // `if a is b do { c } else { d }`  →  `match a { b => c, _ => d }`
            NodeKind::IfIsMatch => self.lower_if_is_match_expr(
                ctx,
                children[0],
                children[1],
                children[2],
                children[3],
                span,
            ),

            // ── Desugaring: if-match (same as post-match) ────────────────
            // `if a is { b => c, … }`  →  `match a { b => c, … }`
            NodeKind::IfMatch => self.lower_post_match_expr(ctx, children[0], children[1], span),

            // ── WhenStatement ────────────────────────────────────────────
            // `when { cond1 => body1, cond2 => body2, … }`
            NodeKind::WhenStatement => {
                self.lower_when_stmt(ctx, children.first().copied().unwrap_or(0), span)
            }

            // ── Desugaring: while ────────────────────────────────────────
            // `while :label? cond { body }`
            //   →  `loop { if !cond { break :label }; body }`
            NodeKind::WhileStatement => {
                self.lower_while_stmt(ctx, children[0], children[1], children[2], span)
            }

            // ── Desugaring: while-is-match ───────────────────────────────
            // `while :label? scrutinee is pat { body }`
            //   →  `loop { match scrutinee { pat => body, _ => break :label } }`
            NodeKind::WhileIsMatch => self.lower_while_is_match_expr(
                ctx,
                children[0],
                children[1],
                children[2],
                children[3],
                span,
            ),

            // ── Desugaring: while-match ──────────────────────────────────
            // `while :label? scrutinee is do { arms… }`
            //   →  `loop { match scrutinee { arms… } }`
            NodeKind::WhileMatch => {
                self.lower_while_match_expr(ctx, children[0], children[1], children[2], span)
            }

            // Preserve the iterable, pattern, guard and loop identity in HIR.
            // Protocol selection belongs to elaboration, not AST lowering.
            NodeKind::ForStatement => self.lower_for_stmt(ctx, node),

            other => {
                self.emit_unsupported_node(&format!("{:?}", other), span);
                self.make_invalid_expr(span)
            }
        }
    }

    pub(crate) fn lower_id_expr(&mut self, ctx: ExprContext, node: NodeIndex) -> Expr<'hir> {
        let Some((NodeKind::Id, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not an id node or invalid id node: no such node index {:?}",
                node
            );
        };

        let identifier = self.node_to_ident(node);
        let next_hir_id = self.next_hir_id();
        if self.refinement_depth > 0 && identifier.name.as_str() == "Itself" {
            return Expr {
                hir_id: next_hir_id,
                kind: ExprKind::RefinementSubject,
                span,
            };
        }
        if let Some(digits) = identifier.name.as_str().strip_prefix('_') {
            if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Expr {
                    hir_id: next_hir_id,
                    kind: ExprKind::TacticParameter(digits.parse().unwrap_or(usize::MAX)),
                    span,
                };
            }
        }
        match self.resolve(identifier) {
            Ok(resolution) => match resolution.kind {
                BindingKind::FnParam(ast_node_ref) => {
                    let hir_id = self.ast_ref_to_hir_id.get(&ast_node_ref).unwrap();
                    Expr {
                        hir_id: next_hir_id,
                        kind: ExprKind::ResolvedFnParam(identifier.name, *hir_id),
                        span,
                    }
                }
                BindingKind::Local(ast_node_ref) => {
                    let hir_id = self.ast_ref_to_hir_id.get(&ast_node_ref).unwrap();
                    Expr {
                        hir_id: next_hir_id,
                        kind: ExprKind::ResolvedLocal(identifier.name, *hir_id),
                        span,
                    }
                }
                BindingKind::ExpandedLocal(site, output) => {
                    let hir_id = self.expanded_locals[&(site, output)];
                    Expr {
                        hir_id: next_hir_id,
                        span,
                        kind: ExprKind::ResolvedLocal(identifier.name, hir_id),
                    }
                }
                BindingKind::ClauseParam(def_id, _) => Expr {
                    hir_id: next_hir_id,
                    kind: ExprKind::ResolvedClauseParam(identifier.name, def_id),
                    span,
                },
                BindingKind::Definition(def_id, scope_id) => {
                    Expr {
                        hir_id: next_hir_id,
                        kind: ExprKind::ResolvedDef {
                            name: identifier.name,
                            has_clause_param: false,
                            def_id,
                            // scope_id: ..
                        },
                        span,
                    }
                }
                BindingKind::LoopLabel(_) => {
                    self.emit_malformed("loop label cannot be used as an expression", span);
                    self.make_invalid_expr(span)
                }
            },
            Err(err) => {
                // Definition metadata is still awaiting attribute elaboration.
                // Preserve unresolved names in the tree, as in its source-text
                // predecessor; access errors must not be hidden by deferral.
                if !matches!(ctx, ExprContext::Attribute | ExprContext::Configuration)
                    || !matches!(err, resolve::ResolveError::UnresolvedIdentifier { .. })
                {
                    self.emit_resolve_error(err);
                }
                Expr {
                    hir_id: next_hir_id,
                    kind: ExprKind::UnresolvedIdent(identifier.name),
                    span,
                }
            }
        }
    }

    pub(crate) fn lower_builtin_view_expr(
        &mut self,
        ctx: ExprContext,
        node: NodeIndex,
    ) -> Expr<'hir> {
        let Some((NodeKind::TakeView, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a take view node or invalid take view node: no such node index {:?}",
                node
            );
        };

        let Some((expected_symbol, span, symbol_children)) = self.ast.get_node(children[0]) else {
            unreachable!(
                "not a builtin view node or invalid builtin view node: no such node index {:?}",
                children[0]
            );
        };

        if NodeKind::Symbol != expected_symbol {
            self.emit_unsupported_node("builtin view with a non-symbol operand", span);
            return self.make_invalid_expr(span);
        }

        let builtin_name = self.node_to_symbol(symbol_children[0]);

        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Builtin(builtin_name),
            span,
        }
    }

    /// Lower an AST block (or block-like) node into an HIR [`Block`].
    pub(crate) fn lower_block(&mut self, ctx: ExprContext, node: NodeIndex) -> Block<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid block node: no such node index {:?}", node);
        };

        // For single-child block variants (DoBlock, AsyncBlock, etc.),
        // the child is the actual Block node.
        let block_node = match kind {
            NodeKind::DoBlock
            | NodeKind::AsyncBlock
            | NodeKind::UnsafeBlock
            | NodeKind::ComptimeBlock => children[0],
            _ => node,
        };

        let block_kind = self.ast.get_node_kind(block_node);

        match block_kind {
            Some(NodeKind::Block | NodeKind::FileScope) => {
                let children = self.ast.get_children(block_node);
                let elems_node = children[0];
                let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                let previous_scope = self.file_scope;
                let previous_depth = self.resolver.ribs().depth();
                if let Some(scope) = self.resolver.block_scope(self.ast_node_ref(block_node)) {
                    self.file_scope = scope;
                    self.resolver
                        .ribs_mut()
                        .push(resolve::Rib::new(RibKind::Block, scope));
                }
                let block = self.lower_stmts_to_block(ctx, elem_nodes, span);
                self.local_exit(previous_depth);
                self.file_scope = previous_scope;
                block
            }
            _ => {
                // Single expression treated as a block with a trailing expr
                let expr = self.lower_expr(ctx, block_node);
                let expr_ref = self.arena.alloc_expr(expr);
                Block {
                    hir_id: self.next_hir_id(),
                    stmts: &[],
                    expr: Some(expr_ref),
                    span,
                }
            }
        }
    }

    /// Lower a list of statement-level nodes into a Block.
    fn lower_stmts_to_block(
        &mut self,
        ctx: ExprContext,
        stmt_nodes: &[NodeIndex],
        span: Span,
    ) -> Block<'hir> {
        let rib = self.local_enter(RibKind::Block);
        let mut stmts: Vec<Expr<'hir>> = Vec::new();
        let mut trailing_expr: Option<&'hir Expr<'hir>> = None;

        for (i, &stmt_node) in stmt_nodes.iter().enumerate() {
            let Some((kind, span, children)) = self.ast.get_node(stmt_node) else {
                continue;
            };

            let is_last = i == stmt_nodes.len() - 1;

            match kind {
                // Item definitions → ExprKind::Item
                NodeKind::Function
                | NodeKind::FunctionTemplateDef
                | NodeKind::FunctionTemplateInstance
                | NodeKind::FunctionTemplateInstanceWithParams
                | NodeKind::StructDef
                | NodeKind::EnumDef
                | NodeKind::TraitDef
                | NodeKind::DeriverDef
                | NodeKind::DeriveDef
                | NodeKind::AssocDecl
                | NodeKind::ImplDef
                | NodeKind::ImplTraitDef
                | NodeKind::TypealiasDef
                | NodeKind::ModuleDef
                | NodeKind::NormalFormHeadDef
                | NodeKind::AlgebraicEffect
                | NodeKind::EffectEnumDef
                | NodeKind::UnionDef
                | NodeKind::ExtendDef
                | NodeKind::ExtendTraitDef
                | NodeKind::CaseDef
                | NodeKind::NewtypeDef
                | NodeKind::ConstDef
                | NodeKind::TestDef => {
                    let owner_id = self.lower_item_in_block(stmt_node);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Item(owner_id),
                        span: stmt_span,
                    });
                }

                // Let / Const declarations → ExprKind::Let
                NodeKind::LetDecl | NodeKind::VarDecl | NodeKind::ConstDecl => {
                    let let_decl = self.lower_let_decl(ctx, stmt_node);
                    let let_ref = self.arena.alloc_let_decl(let_decl);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Let(let_ref),
                        span: stmt_span,
                    });
                }

                // Use/mod statements → ExprKind::Item
                NodeKind::UseStatement | NodeKind::ModStatement => {
                    let owner_id = self.lower_item_in_block(stmt_node);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Item(owner_id),
                        span: stmt_span,
                    });
                }

                // Expression statements → ExprKind::Semi
                NodeKind::ExprStatement => {
                    let children = self.ast.get_children(stmt_node);
                    let expr = self.lower_expr(ctx, children[0]);
                    let expr_ref = self.arena.alloc_expr(expr);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Semi(expr_ref),
                        span: stmt_span,
                    });
                }

                // Attributes wrapping definitions
                NodeKind::Attribute | NodeKind::AttributeSetTrue => {
                    let children = self.ast.get_children(stmt_node);
                    let mut def_node = children[1];
                    while matches!(
                        self.ast.get_node_kind(def_node),
                        Some(NodeKind::Attribute | NodeKind::AttributeSetTrue)
                    ) {
                        def_node = self.ast.get_children(def_node)[1];
                    }
                    let def_kind = self.ast.get_node_kind(def_node);
                    if matches!(
                        def_kind,
                        Some(
                            NodeKind::Function
                                | NodeKind::FunctionTemplateDef
                                | NodeKind::FunctionTemplateInstance
                                | NodeKind::FunctionTemplateInstanceWithParams
                                | NodeKind::StructDef
                                | NodeKind::EnumDef
                                | NodeKind::TraitDef
                                | NodeKind::DeriverDef
                                | NodeKind::DeriveDef
                                | NodeKind::AssocDecl
                                | NodeKind::ImplDef
                                | NodeKind::ImplTraitDef
                                | NodeKind::TypealiasDef
                                | NodeKind::ModuleDef
                                | NodeKind::NormalFormHeadDef
                                | NodeKind::AlgebraicEffect
                                | NodeKind::EffectEnumDef
                                | NodeKind::UnionDef
                                | NodeKind::ExtendDef
                                | NodeKind::ExtendTraitDef
                                | NodeKind::CaseDef
                                | NodeKind::NewtypeDef
                                | NodeKind::ConstDef
                                | NodeKind::TestDef
                        )
                    ) {
                        // Classify through wrappers, but lower the original chain so
                        // attributes retain their targets and lexical environment.
                        let owner_id = self.lower_item_in_block(stmt_node);
                        let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                        stmts.push(Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::Item(owner_id),
                            span: stmt_span,
                        });
                    } else {
                        self.package.pending_semantics.push((
                            self.ast.get_span(stmt_node).unwrap_or(span),
                            "block attribute or modifier".into(),
                        ));
                        let expr = self.lower_expr(ctx, children[1]);
                        let expr_ref = self.arena.alloc_expr(expr);
                        let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                        stmts.push(Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::Semi(expr_ref),
                            span: stmt_span,
                        });
                    }
                }

                // Last expression (no semicolon) becomes trailing expr
                _ if is_last => {
                    let expr = self.lower_expr(ctx, stmt_node);
                    let expr_ref = self.arena.alloc_expr(expr);
                    trailing_expr = Some(expr_ref);
                }

                // Everything else becomes a semi expression
                _ => {
                    let expr = self.lower_expr(ctx, stmt_node);
                    let expr_ref = self.arena.alloc_expr(expr);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Semi(expr_ref),
                        span: stmt_span,
                    });
                }
            }
        }

        self.local_exit(rib);
        let stmts_slice = self.arena.alloc_expr_slice(stmts);
        Block {
            hir_id: self.next_hir_id(),
            stmts: stmts_slice,
            expr: trailing_expr,
            span,
        }
    }

    fn lower_for_stmt(&mut self, ctx: ExprContext, node: NodeIndex) -> Expr<'hir> {
        let Some((NodeKind::ForStatement, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a for stmt node or invalid for statement node: no such node index {:?}",
                node
            );
        };

        // The iterable is outside the element binding and loop-label scope.
        let iterable = self.lower_expr(ctx, children[2]);
        let iter = self.arena.alloc_expr(iterable);
        let scope = self.local_enter(RibKind::Block);
        let identity = self.next_hir_id();
        let label = if children[0] != 0 {
            let label = self.node_to_ident(children[0]);
            self.define_loop_label(label, children[0], identity);
            Some(label)
        } else {
            None
        };
        let (pattern, guard) = self.lower_arm_head(ctx, children[1]);
        let pat = self.arena.alloc_pattern(pattern);
        let body = self.lower_block(ctx, children[3]);
        let body = self.arena.alloc_block(body);
        self.local_exit(scope);
        Expr {
            hir_id: identity,
            kind: ExprKind::For {
                label,
                pat,
                guard,
                iter,
                body,
            },
            span,
        }
    }

    // Keep unary/binary mixtures on one postorder frontier. Returning through
    // lower_expr for unary operands would reintroduce recursive stack growth.
    fn lower_operator_expr(&mut self, ctx: ExprContext, node: NodeIndex) -> Expr<'hir> {
        enum Work {
            Operand(NodeIndex),
            Finish(BinOp, Span),
            Unary(UnOp, Span),
        }
        let mut pending = vec![Work::Operand(node)];
        let mut values = Vec::new();
        while let Some(work) = pending.pop() {
            match work {
                Work::Operand(node) => {
                    let (kind, span, children) =
                        self.ast.get_node(node).expect("operator operand AST node");
                    if let Some(op) = binary_operator(kind) {
                        pending.push(Work::Finish(op, span));
                        pending.push(Work::Operand(children[1]));
                        pending.push(Work::Operand(children[0]));
                    } else if matches!(kind, NodeKind::Negative | NodeKind::BoolNot) {
                        let op = if kind == NodeKind::Negative {
                            UnOp::Neg
                        } else {
                            UnOp::Not
                        };
                        pending.push(Work::Unary(op, span));
                        pending.push(Work::Operand(children[0]));
                    } else {
                        values.push(self.lower_non_operator_expr(ctx, node));
                    }
                }
                Work::Unary(op, span) => {
                    let value = values.pop().expect("lowered unary operand");
                    let value = self.arena.alloc_expr(value);
                    values.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Unary(op, value),
                        span,
                    });
                }
                Work::Finish(op, span) => {
                    let rhs = values.pop().expect("lowered right operand");
                    let lhs = values.pop().expect("lowered left operand");
                    // Allocate the parent only after both operands. This keeps
                    // binding/owner identities and lexical lookup order equal
                    // to ordinary left-to-right recursive lowering.
                    let lhs_ref = self.arena.alloc_expr(lhs);
                    let rhs_ref = self.arena.alloc_expr(rhs);
                    values.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Binary(op, lhs_ref, rhs_ref),
                        span,
                    });
                }
            }
        }
        values.pop().expect("lowered operator expression")
    }

    fn lower_application_expr(
        &mut self,
        ctx: ExprContext,
        callee_node: NodeIndex,
        args_multi: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let callee = self.lower_expr(ctx, callee_node);
        let callee_ref = self.arena.alloc_expr(callee);
        let args = self.lower_args(ctx, args_multi);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Application(callee_ref, args),
            span,
        }
    }

    fn lower_index_expr(
        &mut self,
        ctx: ExprContext,
        base_node: NodeIndex,
        index_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let base = self.lower_expr(ctx, base_node);
        let index = self.lower_expr(ctx, index_node);
        let base_ref = self.arena.alloc_expr(base);
        let index_ref = self.arena.alloc_expr(index);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Index(base_ref, index_ref),
            span,
        }
    }

    fn lower_tuple_expr(
        &mut self,
        ctx: ExprContext,
        elems_multi: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let elem_nodes = self.ast.get_multi_child_slice(elems_multi).unwrap_or(&[]);
        let elems: Vec<_> = elem_nodes
            .iter()
            .map(|&n| self.lower_expr(ctx, n))
            .collect();
        let elems_slice = self.arena.alloc_expr_slice(elems);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Tuple(elems_slice),
            span,
        }
    }

    fn lower_list_expr(
        &mut self,
        ctx: ExprContext,
        elems_multi: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let elem_nodes = self.ast.get_multi_child_slice(elems_multi).unwrap_or(&[]);
        let elems: Vec<_> = elem_nodes
            .iter()
            .map(|&n| self.lower_expr(ctx, n))
            .collect();
        let elems_slice = self.arena.alloc_expr_slice(elems);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::List(elems_slice),
            span,
        }
    }

    fn lower_if_stmt(
        &mut self,
        ctx: ExprContext,
        cond_node: NodeIndex,
        then_node: NodeIndex,
        else_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let cond = self.lower_expr(ctx, cond_node);
        let cond_ref = self.arena.alloc_expr(cond);
        let then_block = self.lower_block(ctx, then_node);
        let then_ref = self.arena.alloc_block(then_block);
        let else_expr = if else_node != 0 {
            let e = self.lower_expr(ctx, else_node);
            Some(self.arena.alloc_expr(e) as &_)
        } else {
            None
        };
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::If(cond_ref, then_ref, else_expr),
            span,
        }
    }

    fn lower_post_match_expr(
        &mut self,
        ctx: ExprContext,
        scrutinee_node: NodeIndex,
        arms_multi: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let scrutinee = self.lower_expr(ctx, scrutinee_node);
        let scrutinee_ref = self.arena.alloc_expr(scrutinee);
        let arm_nodes = self.ast.get_multi_child_slice(arms_multi).unwrap_or(&[]);
        let arms: Vec<_> = arm_nodes
            .iter()
            .map(|&n| self.lower_match_arm(ctx, n))
            .collect();
        let arms_slice = self.arena.alloc_arm_slice(arms);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Match(scrutinee_ref, arms_slice),
            span,
        }
    }

    fn lower_assign_expr(
        &mut self,
        ctx: ExprContext,
        lhs_node: NodeIndex,
        rhs_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let lhs = self.lower_expr(ExprContext::Location, lhs_node);
        let rhs = self.lower_expr(ctx, rhs_node);
        let lhs_ref = self.arena.alloc_expr(lhs);
        let rhs_ref = self.arena.alloc_expr(rhs);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Assign(lhs_ref, rhs_ref),
            span,
        }
    }

    fn lower_assign_op_expr(
        &mut self,
        ctx: ExprContext,
        kind: NodeKind,
        lhs_node: NodeIndex,
        rhs_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let op = match kind {
            NodeKind::AddAssign => BinOp::Add,
            NodeKind::SubAssign => BinOp::Sub,
            NodeKind::MulAssign => BinOp::Mul,
            NodeKind::DivAssign => BinOp::Div,
            NodeKind::RemAssign => BinOp::Rem,
            _ => unreachable!(),
        };
        let lhs = self.lower_expr(ExprContext::Location, lhs_node);
        let rhs = self.lower_expr(ctx, rhs_node);
        let lhs_ref = self.arena.alloc_expr(lhs);
        let rhs_ref = self.arena.alloc_expr(rhs);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::AssignOp(op, lhs_ref, rhs_ref),
            span,
        }
    }

    fn lower_return_stmt(
        &mut self,
        ctx: ExprContext,
        val_node: NodeIndex,
        guard_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let val = if val_node != 0 {
            let e = self.lower_expr(ctx, val_node);
            Some(self.arena.alloc_expr(e) as &_)
        } else {
            None
        };
        let return_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Return(val),
            span,
        };
        // while guard: `return val while guard`  →  `if guard { return val }`
        if guard_node != 0 {
            self.wrap_with_guard(ctx, guard_node, return_expr, span)
        } else {
            return_expr
        }
    }

    fn lower_resume_stmt(
        &mut self,
        ctx: ExprContext,
        val_node: NodeIndex,
        guard_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let val = if val_node != 0 {
            let e = self.lower_expr(ctx, val_node);
            Some(self.arena.alloc_expr(e) as &_)
        } else {
            None
        };
        let resume_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Resume(val),
            span,
        };
        if guard_node != 0 {
            self.wrap_with_guard(ctx, guard_node, resume_expr, span)
        } else {
            resume_expr
        }
    }

    fn lower_break_stmt(
        &mut self,
        ctx: ExprContext,
        label_node: NodeIndex,
        guard_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let kind = if label_node != 0 {
            let label = self.node_to_ident(label_node);
            match self.resolve(label) {
                Ok(resolution) => match resolution.kind {
                    BindingKind::LoopLabel(reference) => {
                        let Some(target) = self.ast_ref_to_hir_id.get(&reference).copied() else {
                            self.emit_malformed("loop label has no HIR target", span);
                            return self.make_invalid_expr(span);
                        };
                        ExprKind::BreakTo(target)
                    }
                    _ => {
                        self.emit_malformed("selected binding is not a loop label", span);
                        return self.make_invalid_expr(span);
                    }
                },
                Err(error) => {
                    self.emit_resolve_error(error);
                    return self.make_invalid_expr(span);
                }
            }
        } else {
            ExprKind::Break(Ident::new(Symbol::invalid(), span))
        };
        let break_expr = Expr {
            hir_id: self.next_hir_id(),
            kind,
            span,
        };
        // while guard: `break :l while guard`  →  `if guard { break :l }`
        if guard_node != 0 {
            self.wrap_with_guard(ctx, guard_node, break_expr, span)
        } else {
            break_expr
        }
    }

    fn lower_continue_stmt(
        &mut self,
        ctx: ExprContext,
        label_node: NodeIndex,
        guard_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let kind = if label_node != 0 {
            let label = self.node_to_ident(label_node);
            match self.resolve(label) {
                Ok(resolution) => match resolution.kind {
                    BindingKind::LoopLabel(reference) => {
                        let Some(target) = self.ast_ref_to_hir_id.get(&reference).copied() else {
                            self.emit_malformed("loop label has no HIR target", span);
                            return self.make_invalid_expr(span);
                        };
                        ExprKind::ContinueTo(target)
                    }
                    _ => {
                        self.emit_malformed("selected binding is not a loop label", span);
                        return self.make_invalid_expr(span);
                    }
                },
                Err(error) => {
                    self.emit_resolve_error(error);
                    return self.make_invalid_expr(span);
                }
            }
        } else {
            ExprKind::Continue(Ident::new(Symbol::invalid(), span))
        };
        let cont_expr = Expr {
            hir_id: self.next_hir_id(),
            kind,
            span,
        };
        // while guard: `continue :l while guard`  →  `if guard { continue :l }`
        if guard_node != 0 {
            self.wrap_with_guard(ctx, guard_node, cont_expr, span)
        } else {
            cont_expr
        }
    }

    fn lower_post_lambda_expr(
        &mut self,
        ctx: ExprContext,
        callee_node: NodeIndex,
        lambda_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let callee = self.lower_expr(ctx, callee_node);
        let lambda = if self
            .ast
            .get_node(lambda_node)
            .is_some_and(|(kind, _, _)| kind == NodeKind::Lambda)
        {
            self.lower_expr(ctx, lambda_node)
        } else {
            let rib = self.local_enter(RibKind::Lambda);
            let body_expr = self.lower_expr(ctx, lambda_node);
            let body = Body {
                params: &[],
                value: self.arena.alloc_expr(body_expr),
            };
            let id = self.next_hir_id();
            let body_id = self.alloc_body(id, body);
            self.local_exit(rib);
            Expr {
                hir_id: id,
                kind: ExprKind::PostLambdaBody(body_id),
                span,
            }
        };
        let lambda_ref = self.arena.alloc_expr(lambda);
        let (callee_ref, mut args, extended) = match callee.kind {
            ExprKind::Application(head, args) => (head, args.to_vec(), false),
            ExprKind::ExtendedApplication(head, args) => (head, args.to_vec(), true),
            _ => (self.arena.alloc_expr(callee) as &_, Vec::new(), false),
        };
        args.push(Arg::PostLambda(lambda_ref));
        let args_slice = self.arena.alloc_arg_slice(args);
        Expr {
            hir_id: self.next_hir_id(),
            kind: if extended {
                ExprKind::ExtendedApplication(callee_ref, args_slice)
            } else {
                ExprKind::Application(callee_ref, args_slice)
            },
            span,
        }
    }

    fn lower_arrow_type_expr(
        &mut self,
        input_node: NodeIndex,
        output_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        // The input telescope owns names used by later inputs and the result.
        // Nested result callables introduce their own scope without hiding
        // differently named outer inputs.
        let rib = self.local_enter(RibKind::Block);
        let input = if self.ast.get_node_kind(input_node) == Some(NodeKind::FnType) {
            self.lower_fn_type_telescope(input_node, self.ast.get_span(input_node).unwrap_or(span))
        } else {
            self.lower_expr(ExprContext::Type, input_node)
        };
        let output = self.lower_expr(ExprContext::Type, output_node);
        self.local_exit(rib);
        let input_ref = self.arena.alloc_expr(input);
        let output_ref = self.arena.alloc_expr(output);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::TyFnArrow(input_ref, output_ref),
            span,
        }
    }

    fn lower_bool_matches_expr(
        &mut self,
        ctx: ExprContext,
        scrutinee_node: NodeIndex,
        pat_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let scrutinee = self.lower_expr(ctx, scrutinee_node);
        let scrutinee_ref = self.arena.alloc_expr(scrutinee);
        let scope = self.local_enter(RibKind::MatchBody);
        let (pat, guard) = self.lower_arm_head(ctx, pat_node);

        let true_expr = self.make_lit_expr(LitKind::Bool(true), span);
        let true_ref = self.arena.alloc_expr(true_expr);
        let arm_true = PatternArm {
            guard,
            hir_id: self.next_hir_id(),
            pat,
            body: true_ref,
            span,
        };

        self.local_exit(scope);
        let false_expr = self.make_lit_expr(LitKind::Bool(false), span);
        let false_ref = self.arena.alloc_expr(false_expr);
        let wild_pat = self.make_wild_pattern(span);
        let arm_false = PatternArm {
            guard: None,
            hir_id: self.next_hir_id(),
            pat: wild_pat,
            body: false_ref,
            span,
        };

        let arms_slice = self.arena.alloc_arm_slice([arm_true, arm_false]);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Match(scrutinee_ref, arms_slice),
            span,
        }
    }

    fn lower_if_is_match_expr(
        &mut self,
        ctx: ExprContext,
        scrutinee_node: NodeIndex,
        pat_node: NodeIndex,
        then_node: NodeIndex,
        else_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let scrutinee = self.lower_expr(ctx, scrutinee_node);
        let scrutinee_ref = self.arena.alloc_expr(scrutinee);
        let scope = self.local_enter(RibKind::MatchBody);
        let (pat, guard) = self.lower_arm_head(ctx, pat_node);

        let then_expr = self.lower_expr(ctx, then_node);
        let then_ref = self.arena.alloc_expr(then_expr);
        let arm_then = PatternArm {
            guard,
            hir_id: self.next_hir_id(),
            pat,
            body: then_ref,
            span,
        };

        self.local_exit(scope);
        let else_expr = if else_node != 0 {
            self.lower_expr(ctx, else_node)
        } else {
            Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Unit,
                span,
            }
        };
        let else_ref = self.arena.alloc_expr(else_expr);
        let wild_pat = self.make_wild_pattern(span);
        let arm_else = PatternArm {
            guard: None,
            hir_id: self.next_hir_id(),
            pat: wild_pat,
            body: else_ref,
            span,
        };

        let arms_slice = self.arena.alloc_arm_slice([arm_then, arm_else]);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Match(scrutinee_ref, arms_slice),
            span,
        }
    }

    fn lower_when_stmt(
        &mut self,
        ctx: ExprContext,
        arms_multi: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let arm_nodes = self.ast.get_multi_child_slice(arms_multi).unwrap_or(&[]);
        let cond_arms: Vec<CondictionArm<'hir>> = arm_nodes
            .iter()
            .map(|&n| {
                let arm_span = self.ast.get_span(n).unwrap_or(span);
                let ac = self.ast.get_children(n);
                if ac.len() >= 2 {
                    let cond_ref = if ac[0] == 0 {
                        None
                    } else {
                        let cond = self.lower_expr(ctx, ac[0]);
                        Some(self.arena.alloc_expr(cond) as &Expr<'hir>)
                    };
                    // An arm accepts a statement as well as an expression.
                    // Give even an unbraced declaration its own lexical block;
                    // this also carries binding cleanup into executable NIR.
                    let body_block = if matches!(
                        self.ast.get_node_kind(ac[1]),
                        Some(NodeKind::LetDecl | NodeKind::VarDecl | NodeKind::ConstDecl)
                    ) {
                        self.lower_stmts_to_block(ctx, &[ac[1]], arm_span)
                    } else {
                        let rib = self.local_enter(RibKind::Block);
                        let value = self.lower_expr(ctx, ac[1]);
                        let value = self.arena.alloc_expr(value);
                        self.local_exit(rib);
                        Block {
                            hir_id: self.next_hir_id(),
                            stmts: &[],
                            expr: Some(value),
                            span: arm_span,
                        }
                    };
                    let body_block = self.arena.alloc_block(body_block);
                    let body = Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Block(body_block),
                        span: arm_span,
                    };
                    let body_ref = self.arena.alloc_expr(body);
                    CondictionArm {
                        hir_id: self.next_hir_id(),
                        cond: cond_ref,
                        body: body_ref,
                        span: arm_span,
                    }
                } else {
                    let inv1 = self.make_invalid_expr(arm_span);
                    let inv2 = self.make_invalid_expr(arm_span);
                    let r1 = self.arena.alloc_expr(inv1);
                    let r2 = self.arena.alloc_expr(inv2);
                    CondictionArm {
                        hir_id: self.next_hir_id(),
                        cond: Some(r1),
                        body: r2,
                        span: arm_span,
                    }
                }
            })
            .collect();
        let arms_slice = self.arena.alloc_cond_arm_slice(cond_arms);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::When(arms_slice),
            span,
        }
    }

    fn lower_while_stmt(
        &mut self,
        ctx: ExprContext,
        label_node: NodeIndex,
        cond_node: NodeIndex,
        body_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let scope = self.local_enter(RibKind::Block);
        let loop_id = self.next_hir_id();
        if label_node != 0 {
            let label = self.node_to_ident(label_node);
            self.define_loop_label(label, label_node, loop_id);
        }

        // if not cond { break :label }
        let cond = self.lower_expr(ctx, cond_node);
        let cond_ref = self.arena.alloc_expr(cond);
        let not_cond = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Unary(UnOp::Not, cond_ref),
            span,
        };
        let not_cond_ref = self.arena.alloc_expr(not_cond);
        let break_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::BreakTo(loop_id),
            span,
        };
        let break_ref = self.arena.alloc_expr(break_expr);
        let guard_then = Block {
            hir_id: self.next_hir_id(),
            stmts: &[],
            expr: Some(break_ref),
            span,
        };
        let guard_then_ref = self.arena.alloc_block(guard_then);
        let guard_if = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::If(not_cond_ref, guard_then_ref, None),
            span,
        };
        let guard_if_ref = self.arena.alloc_expr(guard_if);
        let guard_semi = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Semi(guard_if_ref),
            span,
        };

        // body
        let body_block = self.lower_block(ctx, body_node);
        let body_block_ref = self.arena.alloc_block(body_block);
        let body_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Block(body_block_ref),
            span,
        };
        let body_expr_ref = self.arena.alloc_expr(body_expr);

        let loop_stmts = self.arena.alloc_expr_slice([guard_semi]);
        let loop_block = Block {
            hir_id: self.next_hir_id(),
            stmts: loop_stmts,
            expr: Some(body_expr_ref),
            span,
        };
        let loop_block_ref = self.arena.alloc_block(loop_block);
        self.local_exit(scope);
        Expr {
            hir_id: loop_id,
            kind: ExprKind::Loop(loop_block_ref),
            span,
        }
    }

    fn lower_while_is_match_expr(
        &mut self,
        ctx: ExprContext,
        label_node: NodeIndex,
        scrutinee_node: NodeIndex,
        pat_node: NodeIndex,
        body_node: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let scope = self.local_enter(RibKind::Block);
        let loop_id = self.next_hir_id();
        if label_node != 0 {
            self.define_loop_label(self.node_to_ident(label_node), label_node, loop_id);
        }

        let scrutinee = self.lower_expr(ctx, scrutinee_node);
        let scrutinee_ref = self.arena.alloc_expr(scrutinee);
        let arm_scope = self.local_enter(RibKind::MatchBody);
        let (pat, guard) = self.lower_arm_head(ctx, pat_node);

        let body_expr = self.lower_expr(ctx, body_node);
        let body_ref = self.arena.alloc_expr(body_expr);
        let arm_body = PatternArm {
            guard,
            hir_id: self.next_hir_id(),
            pat,
            body: body_ref,
            span,
        };

        self.local_exit(arm_scope);
        let break_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::BreakTo(loop_id),
            span,
        };
        let break_ref = self.arena.alloc_expr(break_expr);
        let wild_pat = self.make_wild_pattern(span);
        let arm_break = PatternArm {
            guard: None,
            hir_id: self.next_hir_id(),
            pat: wild_pat,
            body: break_ref,
            span,
        };

        let arms_slice = self.arena.alloc_arm_slice([arm_body, arm_break]);
        let match_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Match(scrutinee_ref, arms_slice),
            span,
        };
        let match_ref = self.arena.alloc_expr(match_expr);
        let loop_block = Block {
            hir_id: self.next_hir_id(),
            stmts: &[],
            expr: Some(match_ref),
            span,
        };
        let loop_block_ref = self.arena.alloc_block(loop_block);
        self.local_exit(scope);
        Expr {
            hir_id: loop_id,
            kind: ExprKind::Loop(loop_block_ref),
            span,
        }
    }

    fn lower_while_match_expr(
        &mut self,
        ctx: ExprContext,
        label_node: NodeIndex,
        scrutinee_node: NodeIndex,
        arms_multi: NodeIndex,
        span: Span,
    ) -> Expr<'hir> {
        let scope = self.local_enter(RibKind::Block);
        let loop_id = self.next_hir_id();
        if label_node != 0 {
            self.define_loop_label(self.node_to_ident(label_node), label_node, loop_id);
        }
        let scrutinee = self.lower_expr(ctx, scrutinee_node);
        let scrutinee_ref = self.arena.alloc_expr(scrutinee);
        let arm_nodes = self.ast.get_multi_child_slice(arms_multi).unwrap_or(&[]);
        let arms: Vec<_> = arm_nodes
            .iter()
            .map(|&n| self.lower_match_arm(ctx, n))
            .collect();
        let arms_slice = self.arena.alloc_arm_slice(arms);
        let match_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Match(scrutinee_ref, arms_slice),
            span,
        };
        let match_ref = self.arena.alloc_expr(match_expr);
        let loop_block = Block {
            hir_id: self.next_hir_id(),
            stmts: &[],
            expr: Some(match_ref),
            span,
        };
        let loop_block_ref = self.arena.alloc_block(loop_block);
        self.local_exit(scope);
        Expr {
            hir_id: loop_id,
            kind: ExprKind::Loop(loop_block_ref),
            span,
        }
    }

    fn lower_let_decl(&mut self, ctx: ExprContext, node: NodeIndex) -> LetDecl<'hir> {
        let Some((
            kind @ (NodeKind::LetDecl | NodeKind::VarDecl | NodeKind::ConstDecl),
            span,
            children,
        )) = self.ast.get_node(node)
        else {
            unreachable!(
                "not a let declaration node or invalid let/const declaration node: no such node index {:?}",
                node
            );
        };

        let ty = if children[1] != 0 {
            let ty_expr = self.lower_expr(ExprContext::Type, children[1]);
            Some(self.arena.alloc_expr(ty_expr) as &_)
        } else {
            None
        };

        let code_scope = if let Some(Expr {
            kind: ExprKind::ContextualExprType(bindings, _),
            ..
        }) = ty
        {
            let scope = self.local_enter(resolve::RibKind::MatchBody);
            let type_children = self.ast.get_children(children[1]);
            for (binding, &node) in bindings.iter().zip(
                self.ast
                    .get_multi_child_slice(type_children[1])
                    .unwrap_or(&[]),
            ) {
                self.define_local(binding.name, node, binding.hir_id);
            }
            Some(scope)
        } else {
            None
        };
        let init: Option<&'hir Expr<'hir>> = if children[2] != 0 {
            let init_expr = self.lower_expr(ctx, children[2]);
            Some(self.arena.alloc_expr(init_expr) as &_)
        } else {
            None
        };

        if let Some(scope) = code_scope {
            self.local_exit(scope);
        }

        // Failure executes in the preceding scope, before success bindings exist.
        let otherwise = if let Some(&node) = children.get(3).filter(|&&node| node != 0) {
            let expr = self.lower_expr(ctx, node);
            Some(&*self.arena.alloc_expr(expr))
        } else {
            None
        };
        self.local_enter(RibKind::MatchBody);
        let (pattern, guard) = self.lower_arm_head(ctx, children[0]);
        // Code interfaces also describe generator-local let/var bindings.
        // Stage legality remains the elaborator's responsibility.
        {
            let outputs = match ty.map(|ty| &ty.kind) {
                Some(ExprKind::ContextualPatternType(_, entries)) => {
                    Some(entries.iter().map(|entry| entry.name).collect::<Vec<_>>())
                }
                _ => init.and_then(|value| self.pattern_expression_outputs(value)),
            };
            if let Some(outputs) = outputs {
                let mut target = &pattern;
                while let PatternKind::Binding(_, _, inner) = &target.kind {
                    self.pattern_code_outputs
                        .insert(target.hir_id, outputs.clone());
                    match inner {
                        Some(inner) => target = inner,
                        None => break,
                    }
                }
            }
        }
        LetDecl {
            mutable: kind == NodeKind::VarDecl,
            comptime: kind == NodeKind::ConstDecl,
            hir_id: self.next_hir_id(),
            pattern,
            guard,
            otherwise,
            ty,
            init,
            span,
        }
    }

    fn lower_args(&mut self, ctx: ExprContext, args_multi: NodeIndex) -> &'hir [Arg<'hir>] {
        let arg_nodes = self.ast.get_multi_child_slice(args_multi).unwrap_or(&[]);
        let args: Vec<Arg<'hir>> = arg_nodes.iter().map(|&n| self.lower_arg(ctx, n)).collect();
        self.arena.alloc_arg_slice(args)
    }

    fn lower_arg(&mut self, ctx: ExprContext, node: NodeIndex) -> Arg<'hir> {
        let Some((kind, _, children)) = self.ast.get_node(node) else {
            unreachable!("invalid argument node: no such node index {:?}", node);
        };

        match kind {
            NodeKind::ExpandArg => {
                let expr = self.lower_expr(ctx, children[0]);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Expand(expr_ref)
            }
            NodeKind::OptionalArg => {
                let ident = self.node_to_ident(children[0]);
                let expr = self.lower_expr(ctx, children[1]);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Named(ident, expr_ref)
            }
            // otherwise we just expect it to be a normal expression
            _ => {
                let expr = self.lower_expr(ctx, node);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Positional(expr_ref)
            }
        }
    }

    fn lower_match_arm(&mut self, ctx: ExprContext, node: NodeIndex) -> PatternArm<'hir> {
        if let Some((NodeKind::Attribute | NodeKind::AttributeSetTrue, span, children)) =
            self.ast.get_node(node)
        {
            // Multi-entry parsing applies attributes to the whole arm. Keep the
            // pending semantic obligation instead of treating it as a CaseArm
            // and panicking or silently accepting an unimplemented attribute.
            self.package
                .pending_semantics
                .push((span, "case arm attribute".into()));
            return self.lower_match_arm(ctx, children[1]);
        }
        let Some((NodeKind::CaseArm | NodeKind::CatchArm, span, children)) =
            self.ast.get_node(node)
        else {
            unreachable!(
                "not a pattern case arm node or invalid pattern arm node: no such node index {:?}",
                node
            );
        };

        let scope = self.local_enter(RibKind::MatchBody);
        let (pat, guard) = self.lower_arm_head(ctx, children[0]);
        let body = self.lower_expr(ctx, children[1]);
        let body = self.arena.alloc_expr(body);
        self.local_exit(scope);
        PatternArm {
            hir_id: self.next_hir_id(),
            pat,
            guard,
            body,
            span,
        }
    }

    fn lower_arm_head(
        &mut self,
        ctx: ExprContext,
        node: NodeIndex,
    ) -> (hir::pattern::Pattern<'hir>, Option<&'hir Expr<'hir>>) {
        let mut pat_node = node;
        let guard_node = if self.ast.get_node_kind(pat_node) == Some(NodeKind::IfGuardPattern) {
            let parts = self.ast.get_children(pat_node);
            pat_node = parts[0];
            Some(parts[1])
        } else {
            None
        };
        let pat = self.lower_pattern(pat_node);
        self.bind_arm_pattern(pat_node, &pat);
        let guard = guard_node.map(|node| {
            let expr = self.lower_expr(ctx, node);
            &*self.arena.alloc_expr(expr)
        });
        (pat, guard)
    }

    pub(crate) fn bind_arm_pattern(&mut self, node: NodeIndex, pat: &hir::pattern::Pattern<'hir>) {
        self.bind_arm_pattern_inner(node, pat, &mut std::collections::HashSet::new());
    }

    fn bind_arm_pattern_inner(
        &mut self,
        node: NodeIndex,
        pat: &hir::pattern::Pattern<'hir>,
        seen: &mut std::collections::HashSet<Symbol>,
    ) {
        use hir::pattern::PatternKind;
        let children = self.ast.get_children(node);
        match &pat.kind {
            PatternKind::CodeSplice(_, outputs) => {
                for (index, output) in outputs.iter().enumerate() {
                    if !seen.insert(output.name.name) {
                        self.emit_malformed("duplicate pattern binding", output.name.span);
                    }
                    let site = self.ast_node_ref(node);
                    let index =
                        u32::try_from(index).expect("pattern output count exceeds AST capacity");
                    self.expanded_locals.insert((site, index), output.hir_id);
                    self.define_symbol(output.name, BindingKind::ExpandedLocal(site, index));
                }
            }
            PatternKind::BitVec { segments, .. } => {
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[0])
                    .unwrap_or(&[])
                    .to_vec();
                for (node, segment) in nodes.into_iter().zip(segments) {
                    if let hir::pattern::BitSegment::Typed { pattern, .. } = segment {
                        self.bind_arm_pattern_inner(self.ast.get_children(node)[0], pattern, seen);
                    }
                }
            }
            PatternKind::Then(first, _, next) => {
                self.bind_arm_pattern_inner(children[0], first, seen);
                self.bind_arm_pattern_inner(children[2], next, seen);
            }
            PatternKind::Guard(pattern, _)
            | PatternKind::OptionSome(pattern)
            | PatternKind::ErrorOk(pattern)
            | PatternKind::ErrorErr(pattern) => {
                self.bind_arm_pattern_inner(children[0], pattern, seen)
            }
            PatternKind::Binding(_, name, inner) => {
                if let Some(inner) = inner {
                    self.bind_arm_pattern_inner(children[0], inner, seen);
                }
                if !seen.insert(name.name) {
                    self.emit_malformed("duplicate pattern binding", pat.span);
                }
                let binding_node = if inner.is_some() { children[1] } else { node };
                self.define_local(*name, binding_node, pat.hir_id);
            }
            PatternKind::Struct(_, fields, _) => {
                let nodes = self
                    .ast
                    .get_multi_child_slice(
                        children[if self.ast.get_node_kind(node) == Some(NodeKind::StructPattern) {
                            0
                        } else {
                            1
                        }],
                    )
                    .unwrap_or(&[])
                    .to_vec();
                for (node, field) in nodes.into_iter().zip(*fields) {
                    let node = if self.ast.get_node_kind(node) == Some(NodeKind::PropertyPattern) {
                        self.ast.get_children(node)[1]
                    } else {
                        node
                    };
                    self.bind_arm_pattern_inner(node, &field.pat, seen);
                }
            }
            PatternKind::NormalForm(_, arguments) => {
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[1])
                    .unwrap_or(&[])
                    .to_vec();
                for (node, argument) in nodes.into_iter().zip(*arguments) {
                    let node = if argument.name.is_some() {
                        self.ast.get_children(node)[1]
                    } else {
                        node
                    };
                    self.bind_arm_pattern_inner(node, &argument.pattern, seen);
                }
            }
            PatternKind::List(fields, rest) => {
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[0])
                    .unwrap_or(&[])
                    .to_vec();
                for (&node, field) in nodes.iter().zip(*fields) {
                    self.bind_arm_pattern_inner(node, field, seen);
                }
                if let (Some(rest), Some(node)) = (rest, nodes.last()) {
                    if self.ast.get_node_kind(*node) == Some(ast::NodeKind::ListRestPattern) {
                        self.bind_arm_pattern_inner(self.ast.get_children(*node)[0], rest, seen);
                    }
                }
            }
            PatternKind::Tuple(fields) | PatternKind::AppTuple(_, fields) => {
                let child = if matches!(pat.kind, PatternKind::Tuple(_)) {
                    0
                } else {
                    1
                };
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[child])
                    .unwrap_or(&[])
                    .to_vec();
                for (node, field) in nodes.into_iter().zip(*fields) {
                    self.bind_arm_pattern_inner(node, field, seen);
                }
            }
            PatternKind::Or(alternatives) => {
                // Name lookup uses the first branch's exported identities.
                // Elaboration checks every branch and maps corresponding binders.
                if let Some(first) = alternatives.first() {
                    self.bind_arm_pattern_inner(children[0], first, seen);
                }
            }
            _ => {}
        }
    }

    fn lower_lambda_expr(&mut self, ctx: ExprContext, node: NodeIndex, span: Span) -> Expr<'hir> {
        // Lambda: a, b, N  (return_type, body, params)
        let Some((NodeKind::Lambda, _, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a lambda node or invalid lambda node: no such node index {:?}",
                node
            );
        };

        let return_type_node = children[0];
        let body_node = children[1];
        let params_multi = children[2];

        let param_nodes = self.ast.get_multi_child_slice(params_multi).unwrap_or(&[]);

        let rib = self.local_enter(RibKind::Lambda);
        let closure_params: Vec<ClosureParam<'hir>> = param_nodes
            .iter()
            .map(|&p| self.lower_closure_param(p))
            .collect();
        for (parameter, &node) in closure_params.iter().zip(param_nodes) {
            if let PatternKind::Binding(_, name, None) = parameter.pat.kind {
                self.define_local(name, node, parameter.hir_id);
            }
        }
        let params_slice = self.arena.alloc_closure_param_slice(closure_params);

        let ret_ty = if return_type_node != 0 {
            let ty = self.lower_expr(ExprContext::Type, return_type_node);
            Some(self.arena.alloc_expr(ty) as &_)
        } else {
            None
        };

        // Create a body for the closure
        let body_expr = self.lower_expr(ctx, body_node);
        let body_expr_ref = self.arena.alloc_expr(body_expr);
        let body = Body {
            params: &[],
            value: body_expr_ref,
        };
        let closure_hir_id = self.next_hir_id();
        let body_id = self.alloc_body(closure_hir_id, body);

        self.local_exit(rib);
        Expr {
            hir_id: closure_hir_id,
            kind: ExprKind::Closure(params_slice, ret_ty, body_id),
            span,
        }
    }

    fn lower_closure_param(&mut self, node: NodeIndex) -> ClosureParam<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid closure parameter node: no such node index {:?}",
                node
            );
        };

        match kind {
            NodeKind::TypeBoundParam => {
                let pat = self.lower_pattern(children[0]);

                let ty = if children[1] != 0 {
                    let ty_expr = self.lower_expr(ExprContext::Type, children[1]);
                    Some(self.arena.alloc_expr(ty_expr) as &_)
                } else {
                    None
                };
                ClosureParam {
                    hir_id: self.next_hir_id(),
                    pat,
                    ty,
                    span,
                }
            }
            _ => {
                let pat = self.lower_pattern(node);
                ClosureParam {
                    hir_id: self.next_hir_id(),
                    pat,
                    ty: None,
                    span,
                }
            }
        }
    }

    fn lower_fn_type_expr(&mut self, node: NodeIndex, span: Span) -> Expr<'hir> {
        let rib = self.local_enter(RibKind::Block);
        let result = self.lower_fn_type_telescope(node, span);
        self.local_exit(rib);
        result
    }

    fn lower_fn_type_telescope(&mut self, node: NodeIndex, span: Span) -> Expr<'hir> {
        // FnType: flags_u32, N (modifier_flags, parameter_types)
        let Some((NodeKind::FnType, _, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a function type node or invalid function type node: no such node index {:?}",
                node
            );
        };

        let flags = children[0]; // raw u32 bitmask
        if flags & !(ast::FN_MOD_COMPTIME | ast::FN_MOD_UNSAFE) != 0 {
            self.package
                .pending_semantics
                .push((span, "function type modifiers".into()));
        }
        let params_multi = children[1];

        let param_nodes = self.ast.get_multi_child_slice(params_multi).unwrap_or(&[]);

        // Result types are separate TyFnArrow nodes. Preserve each input's
        // staging modifier instead of lowering its wrapper as a Type expression.
        let mut names = std::collections::HashSet::new();
        let ty_params: Vec<TyParam<'hir>> = param_nodes
            .iter()
            .map(|&n| {
                let mut inner = n;
                let mut flags = 0;
                while let Some((kind, _, children)) = self.ast.get_node(inner) {
                    use hir::common::*;
                    let flag = match kind {
                        NodeKind::ComptimeParam => TPARAM_COMPTIME,
                        NodeKind::ImplicitParam => TPARAM_IMPLICIT,
                        NodeKind::QuoteParam => TPARAM_QUOTE,
                        NodeKind::ErrorParam => TPARAM_ERROR,
                        NodeKind::CatchParam => TPARAM_CATCH,
                        NodeKind::LambdaParam => TPARAM_LAMBDA,
                        NodeKind::AssocParam => TPARAM_ASSOC,
                        NodeKind::OptionalTypeParam => TPARAM_OPTIONAL,
                        _ => break,
                    };
                    flags |= flag;
                    inner = children[0];
                }
                let kind = match self.ast.get_node(inner) {
                    Some((
                        NodeKind::TypeBoundDeclClause | NodeKind::VarargDeclClause,
                        _,
                        children,
                    )) => {
                        let name = self.node_to_ident(children[0]);
                        let ty = self.lower_expr(ExprContext::Type, children[1]);
                        let ty = self.arena.alloc_expr(ty);
                        if self.ast.get_node_kind(inner) == Some(NodeKind::VarargDeclClause) {
                            TyParamKind::Varadic(name, ty)
                        } else {
                            TyParamKind::PositionalDependencyCatched(name, ty)
                        }
                    }
                    Some((NodeKind::ItselfParam, _, _)) => TyParamKind::Itself { is_ref: false },
                    Some((NodeKind::ItselfRefParam, _, _)) => TyParamKind::Itself { is_ref: true },
                    _ => {
                        let ty = self.lower_expr(ExprContext::Type, inner);
                        TyParamKind::Positional(self.arena.alloc_expr(ty))
                    }
                };
                let parameter_span = self.ast.get_span(n).unwrap_or(span);
                let mut parameter = TyParam::new(self.next_hir_id(), kind, parameter_span);
                parameter.flags = flags;
                if let TyParamKind::PositionalDependencyCatched(name, _)
                | TyParamKind::Varadic(name, _) = parameter.kind
                {
                    if !names.insert(name.name) {
                        self.emit_unsupported_node(
                            "duplicate function type parameter name",
                            name.span,
                        );
                    } else {
                        self.define_local(name, n, parameter.hir_id);
                    }
                }
                parameter
            })
            .collect();

        let params_slice = self.arena.alloc_ty_param_slice(ty_params);
        Expr {
            hir_id: self.next_hir_id(),
            kind: if flags & !ast::FN_MOD_COMPTIME != 0 {
                ExprKind::TyModifiedFn(
                    hir::expr::FnTypeModifiers {
                        is_pure: flags & ast::FN_MOD_PURE != 0,
                        is_comptime: flags & ast::FN_MOD_COMPTIME != 0,
                        is_inline: flags & ast::FN_MOD_INLINE != 0,
                        is_unsafe: flags & ast::FN_MOD_UNSAFE != 0,
                        is_spec: flags & ast::FN_MOD_SPEC != 0,
                        is_verified: flags & ast::FN_MOD_VERIFIED != 0,
                    },
                    params_slice,
                )
            } else if flags & ast::FN_MOD_COMPTIME != 0 {
                ExprKind::TyComptimeFn(params_slice)
            } else {
                ExprKind::TyFn(params_slice)
            },
            span,
        }
    }

    fn lower_extend_args(&mut self, ctx: ExprContext, args_multi: NodeIndex) -> &'hir [Arg<'hir>] {
        let arg_nodes = self.ast.get_multi_child_slice(args_multi).unwrap_or(&[]);
        let args: Vec<Arg<'hir>> = arg_nodes
            .iter()
            .map(|&n| self.lower_extend_arg(ctx, n))
            .collect();
        self.arena.alloc_arg_slice(args)
    }

    // 类似div { element1, element2, style: "color: red" }，
    // 这样的extend application，property会采用Arg::Named来表达，而element则采用Arg::Positional
    fn lower_extend_arg(&mut self, ctx: ExprContext, node: NodeIndex) -> Arg<'hir> {
        let Some((kind, _, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid extend argument node: no such node index {:?}",
                node
            );
        };

        match kind {
            NodeKind::ExpandArg => self.lower_arg(ctx, node),
            NodeKind::Property => {
                let ident = self.node_to_ident(children[0]);
                let expr = self.lower_expr(ctx, children[1]);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Named(ident, expr_ref)
            }
            _ => {
                // If it's not a Property, just lower it as a normal expression.
                Arg::Positional(self.arena.alloc_expr(self.lower_expr(ctx, node)))
            }
        }
    }

    /// Create an `Invalid` expression node.
    pub(crate) fn make_invalid_expr(&mut self, span: Span) -> Expr<'hir> {
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Invalid,
            span,
        }
    }

    /// Create a literal expression.
    fn make_lit_expr(&mut self, kind: LitKind, span: Span) -> Expr<'hir> {
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Lit(Lit { kind, span }),
            span,
        }
    }

    fn invalid_literal(&mut self, error: LiteralDecodeError, span: Span) -> Expr<'hir> {
        match error {
            LiteralDecodeError::Invalid(message) => self.emit_malformed(message, span),
            LiteralDecodeError::UnsupportedInterpolation => {
                self.emit_unsupported_node("string interpolation", span);
            }
        }
        self.make_invalid_expr(span)
    }

    fn lower_string_literal(&mut self, node: NodeIndex, span: Span) -> Expr<'hir> {
        let mut value = String::new();
        if self.ast.get_node_kind(node) == Some(NodeKind::Str) {
            match decode_quoted_literal(&self.source_text(node), '"') {
                Ok(decoded) => value = decoded,
                Err(error) => return self.invalid_literal(error, span),
            }
        } else {
            let children = self.ast.get_children(node);
            let Some(leaves) = children
                .first()
                .and_then(|&index| self.ast.get_multi_child_slice(index))
                .filter(|leaves| leaves.len() >= 2)
            else {
                self.emit_malformed("string concatenation requires at least two leaves", span);
                return self.make_invalid_expr(span);
            };
            for &leaf in leaves {
                if self.ast.get_node_kind(leaf) != Some(NodeKind::Str) {
                    self.emit_malformed("string concatenation requires string leaves", span);
                    return self.make_invalid_expr(span);
                }
                match decode_quoted_literal(&self.source_text(leaf), '"') {
                    Ok(decoded) => value.push_str(&decoded),
                    Err(error) => {
                        let leaf_span = self.ast.get_span(leaf).unwrap_or(span);
                        return self.invalid_literal(error, leaf_span);
                    }
                }
            }
        }
        self.make_lit_expr(LitKind::String(value), span)
    }

    /// Create a wildcard pattern `_`.
    fn make_wild_pattern(&mut self, span: Span) -> Pattern<'hir> {
        Pattern {
            hir_id: self.next_hir_id(),
            kind: PatternKind::Wild,
            span,
        }
    }

    /// Wrap `inner` in `if guard { inner }` (no else branch).
    fn wrap_with_guard(
        &mut self,
        ctx: ExprContext,
        guard_node: NodeIndex,
        inner: Expr<'hir>,
        span: Span,
    ) -> Expr<'hir> {
        let guard = self.lower_expr(ctx, guard_node);
        let guard_ref = self.arena.alloc_expr(guard);
        let inner_ref = self.arena.alloc_expr(inner);
        let then_block = Block {
            hir_id: self.next_hir_id(),
            stmts: &[],
            expr: Some(inner_ref),
            span,
        };
        let then_ref = self.arena.alloc_block(then_block);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::If(guard_ref, then_ref, None),
            span,
        }
    }
}

#[cfg(test)]
mod literal_tests {
    use super::{LiteralDecodeError, decode_character_literal, decode_quoted_literal};

    #[test]
    fn strings_decode_documented_escapes_and_unicode_scalars() {
        for (source, expected) in [
            (r#""\\\'\"\n\r\t\0\$""#, "\\'\"\n\r\t\0$"),
            (
                r#""\x{0}\x{41}\x{000061}\x{3A3}\x{1F980}\x{10FFFF}""#,
                "\0AaΣ🦀\u{10FFFF}",
            ),
            ("\"中文 e\u{301} 🦀 {}\"", "中文 e\u{301} 🦀 {}"),
            (
                r#""$5 $! $$ $@ $ {} cost \$name \${name} \x{24}name""#,
                "$5 $! $$ $@ $ {} cost $name ${name} $name",
            ),
            (r#""\\\$name""#, "\\$name"),
        ] {
            assert_eq!(
                decode_quoted_literal(source, '"'),
                Ok(expected.into()),
                "{source}"
            );
        }
    }

    #[test]
    fn strings_reject_unimplemented_interpolation() {
        for source in [
            r#""$name""#,
            r#""${expr}""#,
            r#""$_value""#,
            r#""$中文""#,
            r#""$🦀""#,
            r#""\\$name""#,
        ] {
            assert_eq!(
                decode_quoted_literal(source, '"'),
                Err(LiteralDecodeError::UnsupportedInterpolation),
                "{source}",
            );
        }
    }

    #[test]
    fn invalid_escapes_never_produce_a_replacement_value() {
        for source in [
            r#""\u{41}""#,
            r#""\a""#,
            r#""\x41""#,
            r#""\x{}""#,
            r#""\x{0000000}""#,
            r#""\x{D800}""#,
            r#""\x{DFFF}""#,
            r#""\x{110000}""#,
            r#""\x{FF_FF}""#,
            r#""\x{g}""#,
            r#""\x{41""#,
            r#""\""#,
            "unquoted",
        ] {
            assert!(
                matches!(
                    decode_quoted_literal(source, '"'),
                    Err(LiteralDecodeError::Invalid(_))
                ),
                "{source}",
            );
        }
    }

    #[test]
    fn characters_require_exactly_one_decoded_scalar() {
        for (source, expected) in [
            ("'a'", 'a'),
            ("' '", ' '),
            ("'🦀'", '🦀'),
            ("'$'", '$'),
            (r"'\n'", '\n'),
            (r"'\0'", '\0'),
            (r"'\''", '\''),
            (r"'\x{10FFFF}'", '\u{10FFFF}'),
        ] {
            assert_eq!(decode_character_literal(source), Ok(expected), "{source}");
        }
        for source in ["''", "'ab'", "'e\u{301}'", r"'\n\r'", r"'\x{D800}'", "a"] {
            assert!(decode_character_literal(source).is_err(), "{source}");
        }
    }
}
