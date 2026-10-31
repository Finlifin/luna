//! Instantiate signature bindings before ordinary function elaboration.
//! Declaration identities are retained; no source text or type heads are expanded.
use super::{Error, error};
use hir::clause::ClauseConstraintKind as C;
use hir::common::{Arg, TyParam, TyParamKind as T};
use hir::{Expr, ExprKind as E, HirArena, HirId, ItemKind, OwnerId, Package};
use std::collections::{BTreeSet, HashMap};
mod generation_attributes;
mod generation_inputs;
pub(super) use generation_inputs::{
    GenerationBinding, execution_draft, input_draft, input_group, subject_draft, value_draft,
};
mod member_clone;
pub(super) use generation_attributes::relocate as relocate_generation_attributes;
mod patterns;
mod resolution;
pub(super) use member_clone::{ClonedMember, clone_member_with_inputs};

pub(super) fn expand<'h>(source: &Package<'h>, arena: &'h HirArena) -> Result<Package<'h>, Error> {
    let mut package = source.clone();
    // Body templates need CMTT port connections, not signature-only expansion.
    for (_, info) in source.owners() {
        if let ItemKind::Fn(sig, _) = &info.node.expect_item().kind {
            if sig.modifiers.is_function_template {
                return Err(error(
                    sig.span,
                    "function template expansion with a shared CMTT body is not implemented",
                ));
            }
        }
    }
    for (owner, info) in source.owners() {
        let item = info.node.expect_item();
        let ItemKind::FunctionTemplateInstance {
            template,
            signature,
            explicit_parameters,
            parameters,
            clause_bindings,
            modifiers,
            body,
        } = &item.kind
        else {
            continue;
        };
        let formal = signature
            .as_ref()
            .ok_or_else(|| error(item.span, "unlinked function template signature"))?;
        if formal.params.len() != parameters.len()
            || formal
                .params
                .iter()
                .zip(parameters)
                .any(|(p, input)| input.formal != Some(p.hir_id))
        {
            return Err(error(
                item.span,
                "invalid function template parameter correspondence",
            ));
        }
        if matches!(template.kind, E::NFApplication(..)) {
            return Err(error(
                item.span,
                "function template clause substitution is not implemented",
            ));
        }
        if explicit_parameters.is_some() {
            return Err(error(
                item.span,
                "explicit function template parameter compatibility is not implemented",
            ));
        }
        // A separate identity namespace avoids collisions with the already-lowered body.
        let namespace = package.alloc_owner_id();
        let mut rebind = Rebind {
            source,
            arena,
            namespace,
            bindings: parameters
                .iter()
                .zip(formal.params)
                .map(|(input, formal)| (formal.hir_id, input.binding))
                .collect(),
            budget: 100_000,
            bodies: Vec::new(),
            definitions: clause_bindings
                .iter()
                .map(|input| (input.formal, input.binding))
                .collect(),
            member_owner: None,
            captures: HashMap::new(),
            capture_uses: BTreeSet::new(),
            self_type: None,
        };
        if clause_bindings.len() != formal.clause_params.len()
            || formal.clause_params.iter().any(|parameter| {
                parameter
                    .definition
                    .is_none_or(|id| !rebind.definitions.contains_key(&id))
            })
        {
            return Err(error(
                item.span,
                "invalid function template clause correspondence",
            ));
        }
        let mut sig = formal.clone();
        sig.clause_params = rebind.clauses(formal.clause_params)?;
        let params = formal
            .params
            .iter()
            .zip(parameters)
            .map(|(p, input)| {
                let mut p = p.clone();
                p.hir_id = input.binding;
                p.name = input.name;
                p.ty = rebind.ty(&p.ty, 0)?;
                Ok(p)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        sig.params = arena.alloc_fn_param_slice(params);
        sig.return_ty = formal.return_ty.map(|e| rebind.expr(e, 0)).transpose()?;
        sig.return_binding = formal.return_binding.map(|id| rebind.id(id));
        sig.clause_constraints = arena.alloc_clause_slice(
            formal
                .clause_constraints
                .iter()
                .map(|c| {
                    let mut c = c.clone();
                    c.hir_id = rebind.id(c.hir_id);
                    c.kind = match c.kind {
                        C::Requires(e) => C::Requires(rebind.expr(e, 0)?),
                        C::Ensures(e) => C::Ensures(rebind.expr(e, 0)?),
                        C::Decreases(e) => C::Decreases(rebind.expr(e, 0)?),
                        C::Outcome => C::Outcome,
                    };
                    Ok(c)
                })
                .collect::<Result<Vec<_>, Error>>()?,
        );
        sig.modifiers = *modifiers;
        sig.modifiers.is_pure |= formal.modifiers.is_pure;
        // Interface reuse cannot erase the declaration's unsafe call boundary.
        sig.modifiers.is_unsafe |= formal.modifiers.is_unsafe;
        for (id, body) in rebind.bodies.drain(..) {
            package.insert_body(id, body);
        }
        for (id, evidence) in rebind.resolution()? {
            if let Some(plan) = evidence.trait_sources {
                package.trait_sources.insert(id, plan);
            }
            if let Some(origin) = evidence.source_origin {
                package.source_origins.insert(id, origin);
            }
            if let Some(value) = evidence.projection {
                package.member_projections.insert(id, value);
            }
            if let Some(value) = evidence.deferred {
                package.deferred_member_projections.insert(id, value);
            }
            if let Some(value) = evidence.access {
                package.member_access.insert(id, value);
            }
        }
        let mut body_value = package
            .body(*body)
            .ok_or_else(|| error(item.span, "function template body missing"))?
            .clone();
        body_value.params =
            arena.alloc_param_slice(sig.params.iter().map(hir::body::BodyParam::from_fn_param));
        package.insert_body(*body, body_value);
        let mut expanded = item.clone();
        expanded.kind = ItemKind::Fn(sig, *body);
        let mut info = info.clone();
        info.node = hir::owner::OwnerNode::Item(arena.alloc_item(expanded));
        package.insert_owner(owner, info);
    }
    package
        .pending_semantics
        .retain(|(_, feature)| feature != "function template expansion");
    Ok(package)
}

#[derive(Clone, Copy)]
pub(super) enum InputCapture {
    Definition(hir::common::DefId),
    Local(HirId),
}

struct Rebind<'s, 'h> {
    source: &'s Package<'h>,
    arena: &'h HirArena,
    namespace: OwnerId,
    bindings: HashMap<HirId, HirId>,
    budget: usize,
    bodies: Vec<(hir::BodyId, hir::body::Body<'h>)>,
    definitions: HashMap<hir::common::DefId, hir::common::DefId>,
    member_owner: Option<OwnerId>,
    captures: HashMap<HirId, InputCapture>,
    capture_uses: BTreeSet<hir::common::DefId>,
    self_type: Option<&'h Expr<'h>>,
}
impl<'h> Rebind<'_, 'h> {
    fn charge(&mut self, span: rustc_span::Span, depth: usize) -> Result<(), Error> {
        if depth >= 128 || self.budget == 0 {
            return Err(error(
                span,
                "function template signature substitution budget exhausted",
            ));
        }
        self.budget -= 1;
        Ok(())
    }
    fn body(
        &mut self,
        id: hir::BodyId,
        span: rustc_span::Span,
        depth: usize,
    ) -> Result<hir::BodyId, Error> {
        self.charge(span, depth)?;
        let body = self
            .source
            .body(id)
            .ok_or_else(|| error(span, "function template signature body missing"))?
            .clone();
        let mut params = Vec::new();
        for param in body.params {
            let mut param = param.clone();
            param.hir_id = self.id(param.hir_id);
            param.ty = self.ty(&param.ty, depth + 1)?;
            params.push(param);
        }
        let value = self.expr(body.value, depth + 1)?;
        let id = hir::BodyId::new(self.id(id.hir_id));
        self.bodies.push((
            id,
            hir::body::Body {
                params: self.arena.alloc_param_slice(params),
                value,
            },
        ));
        Ok(id)
    }
    fn block(&mut self, block: &hir::Block<'h>, depth: usize) -> Result<&'h hir::Block<'h>, Error> {
        self.charge(block.span, depth)?;
        let depth = depth + 1;
        let stmts = block
            .stmts
            .iter()
            .map(|e| self.expr(e, depth).cloned())
            .collect::<Result<Vec<_>, _>>()?;
        let expr = block.expr.map(|e| self.expr(e, depth)).transpose()?;
        Ok(self.arena.alloc_block(hir::Block {
            hir_id: self.id(block.hir_id),
            stmts: self.arena.alloc_expr_slice(stmts),
            expr,
            span: block.span,
        }))
    }
    fn id(&mut self, id: HirId) -> HirId {
        // Nested bodies may have different owners with equal local indices.
        // Map the entire source identity, never only its local component.
        let next = HirId::new(
            self.namespace,
            hir::ItemLocalId::new(self.bindings.len() as u32 + 1),
        );
        *self.bindings.entry(id).or_insert(next)
    }
    fn ty(&mut self, ty: &TyParam<'h>, depth: usize) -> Result<TyParam<'h>, Error> {
        self.charge(ty.span, depth)?;
        let depth = depth + 1;
        let mut ty = ty.clone();
        ty.hir_id = self.id(ty.hir_id);
        ty.kind = match ty.kind {
            T::Positional(e) => T::Positional(self.expr(e, depth)?),
            T::PositionalDependencyCatched(name, e) => {
                T::PositionalDependencyCatched(name, self.expr(e, depth)?)
            }
            T::Optional(name, e, default) => {
                T::Optional(name, self.expr(e, depth)?, self.expr(default, depth)?)
            }
            T::Varadic(name, e) => T::Varadic(name, self.expr(e, depth)?),
            T::Itself { is_ref } => T::Itself { is_ref },
        };
        Ok(ty)
    }
    fn substitutions(
        &mut self,
        substitutions: &[hir::expr::CodeSubstitution<'h>],
        depth: usize,
    ) -> Result<Vec<hir::expr::CodeSubstitution<'h>>, Error> {
        substitutions
            .iter()
            .map(|substitution| {
                Ok(hir::expr::CodeSubstitution {
                    port: substitution.port,
                    binding: self.expr(substitution.binding, depth)?,
                })
            })
            .collect()
    }
    fn code_context(
        &mut self,
        context: &[hir::expr::CodeContextBinding<'h>],
        depth: usize,
    ) -> Result<Vec<hir::expr::CodeContextBinding<'h>>, Error> {
        context
            .iter()
            .map(|binding| {
                Ok(hir::expr::CodeContextBinding {
                    hir_id: self.id(binding.hir_id),
                    name: binding.name,
                    ty: self.expr(binding.ty, depth)?,
                    default: binding
                        .default
                        .map(|value| self.expr(value, depth))
                        .transpose()?,
                })
            })
            .collect()
    }
    fn expr(&mut self, expr: &Expr<'h>, depth: usize) -> Result<&'h Expr<'h>, Error> {
        self.charge(expr.span, depth)?;
        let depth = depth + 1;
        if let E::ResolvedFnParam(name, id) | E::ResolvedLocal(name, id) = &expr.kind {
            if let Some(capture) = self.captures.get(id).copied() {
                let hir_id = self.id(expr.hir_id);
                return Ok(self.arena.alloc_expr(Expr {
                    hir_id,
                    span: expr.span,
                    kind: match capture {
                        InputCapture::Definition(def_id) => {
                            self.capture_uses.insert(def_id);
                            E::ResolvedDef {
                                name: *name,
                                def_id,
                                has_clause_param: false,
                            }
                        }
                        InputCapture::Local(id) => E::ResolvedLocal(*name, id),
                    },
                }));
            }
            if self.member_owner.is_some_and(|owner| id.owner != owner)
                && !self.bindings.contains_key(id)
            {
                return Err(error(
                    expr.span,
                    "generated member capture requires an explicit substitution",
                ));
            }
        }
        if matches!(expr.kind, E::TySelf) {
            if let Some(subject) = self.self_type {
                return self.expr(subject, depth);
            }
        }
        let kind = match &expr.kind {
            E::ResolvedClauseParam(name, id) => E::ResolvedClauseParam(
                *name,
                self.definitions.get(id).copied().ok_or_else(|| {
                    error(
                        expr.span,
                        "generated clause reference requires an explicit identity substitution",
                    )
                })?,
            ),
            E::ResolvedFnParam(name, id) => E::ResolvedFnParam(*name, self.id(*id)),
            E::ResolvedLocal(name, id) => E::ResolvedLocal(*name, self.id(*id)),
            E::ResolvedLoopLabel(name, id) => E::ResolvedLoopLabel(*name, self.id(*id)),
            // Positional placeholders belong to their tactic boundary, not to
            // the declaration's HIR-local namespace. Clone the boundary body
            // normally so captured template inputs still get fresh identities.
            E::TacticParameter(index) => E::TacticParameter(*index),
            E::TacticArgument(value) => E::TacticArgument(self.expr(value, depth)?),
            E::CodeQuote(value, substitutions) => E::CodeQuote(
                self.expr(value, depth)?,
                self.substitutions(substitutions, depth)?,
            ),
            E::CodePatternQuote(pattern, substitutions) => E::CodePatternQuote(
                self.pattern(pattern, depth)?,
                self.substitutions(substitutions, depth)?,
            ),
            E::CodeSplice(value, substitutions) => E::CodeSplice(
                self.expr(value, depth)?,
                self.substitutions(substitutions, depth)?,
            ),
            E::ContextualExprType(context, result) => E::ContextualExprType(
                self.code_context(context, depth)?,
                self.expr(result, depth)?,
            ),
            E::ContextualPatternType(subject, context) => E::ContextualPatternType(
                self.expr(subject, depth)?,
                self.code_context(context, depth)?,
            ),
            E::BreakTo(id) => E::BreakTo(self.id(*id)),
            E::ContinueTo(id) => E::ContinueTo(self.id(*id)),
            E::ResolvedDef {
                name,
                def_id,
                has_clause_param,
            } => E::ResolvedDef {
                name: *name,
                def_id: self.definitions.get(def_id).copied().unwrap_or(*def_id),
                has_clause_param: *has_clause_param,
            },
            E::Builtin(_)
            | E::Lit(_)
            | E::TyType
            | E::TyVoid
            | E::TyNoReturn
            | E::TyAny
            | E::TyPlaceholder
            | E::TySelf
            | E::SelfValue
            | E::Unit
            | E::Null
            | E::Undefined
            | E::Break(_)
            | E::Continue(_)
            | E::RefinementSubject => expr.kind.clone(),
            E::Binary(op, a, b) => E::Binary(*op, self.expr(a, depth)?, self.expr(b, depth)?),
            E::Unary(op, a) => E::Unary(*op, self.expr(a, depth)?),
            E::If(cond, block, other) => E::If(
                self.expr(cond, depth)?,
                self.block(block, depth)?,
                other.map(|e| self.expr(e, depth)).transpose()?,
            ),
            E::Block(block) => E::Block(self.block(block, depth)?),
            E::UnsafeBlock(block) => E::UnsafeBlock(self.block(block, depth)?),
            E::Loop(block) => E::Loop(self.block(block, depth)?),
            E::Match(value, arms) => E::Match(self.expr(value, depth)?, self.arms(arms, depth)?),
            E::EffectHandle(value, arms) => {
                E::EffectHandle(self.expr(value, depth)?, self.arms(arms, depth)?)
            }
            E::InlineMatch(arms) => E::InlineMatch(self.arms(arms, depth)?),
            E::Let(decl) => {
                let mut decl = (*decl).clone();
                decl.hir_id = self.id(decl.hir_id);
                decl.pattern = self.pattern(&decl.pattern, depth)?.clone();
                decl.ty = decl.ty.map(|e| self.expr(e, depth)).transpose()?;
                decl.init = decl.init.map(|e| self.expr(e, depth)).transpose()?;
                decl.guard = decl.guard.map(|e| self.expr(e, depth)).transpose()?;
                decl.otherwise = decl.otherwise.map(|e| self.expr(e, depth)).transpose()?;
                E::Let(self.arena.alloc_let_decl(decl))
            }
            E::Assign(a, b) => E::Assign(self.expr(a, depth)?, self.expr(b, depth)?),
            E::AssignOp(op, a, b) => E::AssignOp(*op, self.expr(a, depth)?, self.expr(b, depth)?),
            E::When(arms) => {
                let arms = arms
                    .iter()
                    .map(|arm| {
                        Ok(hir::expr::CondictionArm {
                            hir_id: self.id(arm.hir_id),
                            cond: arm.cond.map(|e| self.expr(e, depth)).transpose()?,
                            body: self.expr(arm.body, depth)?,
                            span: arm.span,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                E::When(self.arena.alloc_cond_arm_slice(arms))
            }
            E::Closure(params, result, body) => {
                let params = params
                    .iter()
                    .map(|param| {
                        Ok(hir::expr::ClosureParam {
                            hir_id: self.id(param.hir_id),
                            pat: self.pattern(&param.pat, depth)?.clone(),
                            ty: param.ty.map(|e| self.expr(e, depth)).transpose()?,
                            span: param.span,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                E::Closure(
                    self.arena.alloc_closure_param_slice(params),
                    result.map(|e| self.expr(e, depth)).transpose()?,
                    self.body(*body, expr.span, depth)?,
                )
            }
            E::PostLambdaBody(body) => E::PostLambdaBody(self.body(*body, expr.span, depth)?),
            E::For {
                label,
                pat,
                guard,
                iter,
                body,
            } => E::For {
                label: *label,
                pat: self.pattern(pat, depth)?,
                guard: guard.map(|e| self.expr(e, depth)).transpose()?,
                iter: self.expr(iter, depth)?,
                body: self.block(body, depth)?,
            },
            E::InlineFor {
                label,
                pat,
                iter,
                body,
            } => E::InlineFor {
                label: *label,
                pat: self.pattern(pat, depth)?,
                iter: self.expr(iter, depth)?,
                body: self.expr(body, depth)?,
            },
            E::InlineIf {
                cond,
                then_expr,
                else_expr,
            } => E::InlineIf {
                cond: self.expr(cond, depth)?,
                then_expr: self.expr(then_expr, depth)?,
                else_expr: else_expr.map(|e| self.expr(e, depth)).transpose()?,
            },
            E::Semi(e) => E::Semi(self.expr(e, depth)?),
            E::Assumes(e) => E::Assumes(self.expr(e, depth)?),
            E::InlineControl(e) => E::InlineControl(self.expr(e, depth)?),
            E::Defer(e) => E::Defer(self.expr(e, depth)?),
            E::EffectRequest(e) => E::EffectRequest(self.expr(e, depth)?),
            E::ErrorNew(e) => E::ErrorNew(self.expr(e, depth)?),
            E::ErrorPropagate(e) => E::ErrorPropagate(self.expr(e, depth)?),
            E::OptionPropagate(e) => E::OptionPropagate(self.expr(e, depth)?),
            E::Resume(e) => E::Resume(e.map(|e| self.expr(e, depth)).transpose()?),
            E::ErrorHandle {
                value,
                before,
                arms,
                after,
            } => {
                let before = before
                    .map(|arm| self.arm(arm, depth))
                    .transpose()?
                    .map(|arm| self.arena.alloc_arm(arm));
                let after = after
                    .map(|arm| self.arm(arm, depth))
                    .transpose()?
                    .map(|arm| self.arena.alloc_arm(arm));
                E::ErrorHandle {
                    value: self.expr(value, depth)?,
                    before,
                    arms: self.arms(arms, depth)?,
                    after,
                }
            }
            E::Return(e) => E::Return(e.map(|e| self.expr(e, depth)).transpose()?),
            E::ConstantObjectEntry(value, index) => {
                E::ConstantObjectEntry(self.expr(value, depth)?, *index)
            }
            E::Projection(a, field) => E::Projection(self.expr(a, depth)?, *field),
            E::TyPtr(a) => E::TyPtr(self.expr(a, depth)?),
            E::TyOptional(a) => E::TyOptional(self.expr(a, depth)?),
            E::TyClosure(environment, signature) => {
                E::TyClosure(self.expr(environment, depth)?, self.expr(signature, depth)?)
            }
            E::TyClosureEnvironment(bindings) => {
                let bindings = bindings
                    .iter()
                    .map(|binding| {
                        Ok(hir::expr::ClosureEnvironmentBinding {
                            name: binding.name,
                            ty: self.expr(binding.ty, depth)?,
                            value: binding.value.map(|e| self.expr(e, depth)).transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                E::TyClosureEnvironment(
                    self.arena.alloc_closure_environment_binding_slice(bindings),
                )
            }
            E::TypeOf(a) => E::TypeOf(self.expr(a, depth)?),
            E::TakeOld(a) => E::TakeOld(self.expr(a, depth)?),
            E::TakeBitVec(a) => E::TakeBitVec(self.expr(a, depth)?),
            E::TakeIntrinsic(a) => E::TakeIntrinsic(self.expr(a, depth)?),
            E::TakeTagName(a) => E::TakeTagName(self.expr(a, depth)?),
            E::TakeTupleEnumerate(a) => E::TakeTupleEnumerate(self.expr(a, depth)?),
            E::MultiDeref(a) => E::MultiDeref(self.expr(a, depth)?),
            E::Ref(a) => E::Ref(self.expr(a, depth)?),
            E::Deref(a) => E::Deref(self.expr(a, depth)?),
            E::TyFnArrow(a, b) => E::TyFnArrow(self.expr(a, depth)?, self.expr(b, depth)?),
            E::ErrorQualifiedType(a, b) => {
                E::ErrorQualifiedType(self.expr(a, depth)?, self.expr(b, depth)?)
            }
            E::EffectQualifiedType(a, b) => {
                E::EffectQualifiedType(self.expr(a, depth)?, self.expr(b, depth)?)
            }
            E::TraitBound(a, b) => E::TraitBound(self.expr(a, depth)?, self.expr(b, depth)?),
            E::Cast(a, b) => E::Cast(self.expr(a, depth)?, self.expr(b, depth)?),
            E::Index(a, b) => E::Index(self.expr(a, depth)?, self.expr(b, depth)?),
            E::Range {
                start,
                end,
                inclusive,
            } => E::Range {
                start: start.map(|e| self.expr(e, depth)).transpose()?,
                end: end.map(|e| self.expr(e, depth)).transpose()?,
                inclusive: *inclusive,
            },
            E::LiteralConversion { literal, suffix } => E::LiteralConversion {
                literal: self.expr(literal, depth)?,
                suffix: *suffix,
            },
            E::Object(entries) => {
                let entries = entries
                    .iter()
                    .map(|entry| {
                        Ok(match entry {
                            hir::expr::ObjectEntry::Value(e) => {
                                hir::expr::ObjectEntry::Value(self.expr(e, depth)?)
                            }
                            hir::expr::ObjectEntry::Property(field) => {
                                hir::expr::ObjectEntry::Property(hir::expr::FieldExpr {
                                    ident: field.ident,
                                    expr: self.expr(field.expr, depth)?,
                                    span: field.span,
                                })
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                E::Object(entries)
            }
            E::CollectedProperties(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| {
                        Ok(hir::expr::FieldExpr {
                            ident: field.ident,
                            expr: self.expr(field.expr, depth)?,
                            span: field.span,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                E::CollectedProperties(self.arena.alloc_field_expr_slice(fields))
            }
            E::RefinedType { requirement, base } => E::RefinedType {
                requirement: self.expr(requirement, depth)?,
                base: self.expr(base, depth)?,
            },
            E::Application(callee, args)
            | E::NFApplication(callee, args)
            | E::ExtendedApplication(callee, args) => {
                let callee = self.expr(callee, depth)?;
                let args = args
                    .iter()
                    .map(|arg| {
                        Ok(match arg {
                            Arg::Positional(e) => Arg::Positional(self.expr(e, depth)?),
                            Arg::Named(name, e) => Arg::Named(*name, self.expr(e, depth)?),
                            Arg::Expand(e) => Arg::Expand(self.expr(e, depth)?),
                            Arg::Implicit(e) => Arg::Implicit(self.expr(e, depth)?),
                            Arg::PostLambda(e) => Arg::PostLambda(self.expr(e, depth)?),
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                let args = self.arena.alloc_arg_slice(args);
                match expr.kind {
                    E::Application(..) => E::Application(callee, args),
                    E::NFApplication(..) => E::NFApplication(callee, args),
                    _ => E::ExtendedApplication(callee, args),
                }
            }
            E::TyFn(params)
            | E::TyComptimeFn(params)
            | E::TyModifiedFn(_, params)
            | E::TyNFFn(params) => {
                let params = params
                    .iter()
                    .map(|p| self.ty(p, depth))
                    .collect::<Result<Vec<_>, _>>()?;
                let params = self.arena.alloc_ty_param_slice(params);
                match expr.kind {
                    E::TyFn(..) => E::TyFn(params),
                    E::TyComptimeFn(..) => E::TyComptimeFn(params),
                    E::TyModifiedFn(modifiers, _) => E::TyModifiedFn(modifiers, params),
                    _ => E::TyNFFn(params),
                }
            }
            E::Tuple(items) | E::List(items) => {
                let items = items
                    .iter()
                    .map(|e| self.expr(e, depth).cloned())
                    .collect::<Result<Vec<_>, _>>()?;
                let items = self.arena.alloc_expr_slice(items);
                if matches!(expr.kind, E::Tuple(_)) {
                    E::Tuple(items)
                } else {
                    E::List(items)
                }
            }
            _ => {
                return Err(error(
                    expr.span,
                    "this expression in a function template signature requires substitution not yet implemented",
                ));
            }
        };
        let hir_id = self.id(expr.hir_id);
        Ok(self.arena.alloc_expr(Expr {
            hir_id,
            kind,
            span: expr.span,
        }))
    }
}

pub(super) fn implementation_draft<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    request_owner: OwnerId,
    owner: OwnerId,
    request: &hir::item::DeriveRequest<'h>,
    strategy_trait: &'h Expr<'h>,
    members: Vec<OwnerId>,
    definitions: &HashMap<hir::common::DefId, hir::common::DefId>,
    captures: &HashMap<HirId, InputCapture>,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    *fuel = fuel
        .checked_sub(definitions.len().saturating_add(captures.len()))
        .ok_or_else(|| {
            error(
                request.self_ty.span,
                "generated impl identity budget exhausted",
            )
        })?;
    let mut rebind = Rebind {
        source,
        arena,
        namespace: owner,
        bindings: HashMap::new(),
        budget: *fuel,
        bodies: Vec::new(),
        definitions: definitions.clone(),
        member_owner: Some(request_owner),
        captures: captures.clone(),
        capture_uses: BTreeSet::new(),
        self_type: None,
    };
    let result = (|| {
        let clause_params = rebind.clauses(request.clause_params)?;
        let clause_constraints = rebind.constraints(request.clause_constraints)?;
        let self_ty = rebind.expr(request.self_ty, 0)?;
        let trait_ref = rebind.expr(request.trait_ref, 0)?;
        let generation_trait = rebind.expr(strategy_trait, 0)?;
        let resolution = rebind.resolution()?;
        let original = source.item(request_owner).unwrap();
        let item = hir::Item {
            owner_id: owner,
            ident: original.ident,
            span: original.span,
            kind: ItemKind::Impl(hir::ImplDef {
                generation_trait: Some(generation_trait),
                phase: request.phase,
                self_ty,
                trait_ref: Some(trait_ref),
                clause_params,
                clause_constraints,
                items: members,
            }),
        };
        let mut nodes = hir::OwnerNodes::new();
        for parameter in clause_params {
            nodes.insert(
                parameter.hir_id.local_id,
                hir::ItemLocalId::new(0),
                hir::Node::ClauseParam(parameter),
            );
        }
        Ok((item, resolution, nodes))
    })();
    *fuel = rebind.budget;
    let (item, resolution, nodes) = result?;
    Ok(ClonedMember {
        item,
        resolution,
        bodies: rebind.bodies,
        capture_uses: rebind.capture_uses,
        bindings: rebind.bindings,
        nodes,
        attributes: Vec::new(),
        staged_parameters: Vec::new(),
    })
}
