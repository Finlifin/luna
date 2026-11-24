//! Ordinary cross-stage conversion produces a runtime trait call, with the
//! source value routed through the existing staged-argument machinery.
use super::*;

/// A checked operand keeps a source program distinct from a value already
/// checked in the consumer's phase. Source does not mean already evaluated.
pub(super) enum Operand {
    Source(n::Expr),
    Value(n::Expr),
}

impl Operand {
    pub(super) fn into_parts(self) -> (n::Expr, bool) {
        match self {
            Self::Source(value) => (value, true),
            Self::Value(value) => (value, false),
        }
    }

    pub(super) fn projected(value: n::Expr, source: bool) -> Self {
        if source {
            Self::Source(value)
        } else {
            Self::Value(value)
        }
    }
}

impl std::fmt::Display for Operand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(value) => write!(f, "(source-operand {value})"),
            Self::Value(value) => write!(f, "(value-operand {value})"),
        }
    }
}

impl Checker<'_, '_> {
    /// Inspect a receiver before deciding whether its result crosses stages.
    /// This boundary owns the checked program; no ambient flag or source-range
    /// cache can transfer an observation to another checking attempt.
    pub(super) fn field_operand(&mut self, expr: &Expr<'_>) -> Result<Operand, Error> {
        if self.runtime_value_context() {
            if let H::ResolvedLocal(_, binding) | H::ResolvedFnParam(_, binding) = expr.kind
                && self
                    .generator_data
                    .get(&local(binding))
                    .is_some_and(Option::is_some)
            {
                // The binding reader retains current snapshot/value-reference
                // checks. Abstract generator inputs are not available values.
                return self.expr(expr, None).map(Operand::Source);
            }
            if let H::ResolvedDef { def_id, .. } = expr.kind
                && let Some(owner) = self.package.definition_owner(def_id)
                && matches!(
                    self.package.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Const(..))
                )
            {
                let mut value = if self.is_build_context(owner) {
                    self.build_context_value(expr, owner)?
                } else {
                    self.constant_value(owner, expr.span)?
                };
                value.source = source(expr.span);
                return Ok(Operand::Source(value));
            }
            if matches!(expr.kind, H::Application(..) | H::ExtendedApplication(..))
                && self.declared_comptime_call_result(expr)?.is_some()
            {
                return self
                    .with_staged_input(true, |checker| checker.expr(expr, None))
                    .map(Operand::Source);
            }
            if let H::Projection(base, field) = expr.kind {
                return self.projection_operand(expr, base, &field, None, None);
            }
        }
        self.expr(expr, None).map(Operand::Value)
    }

    pub(super) fn deliver_operand(
        &mut self,
        expr: &Expr<'_>,
        operand: Operand,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (value, source) = operand.into_parts();
        if source && self.runtime_value_context() {
            return self.deliver_comptime_result(expr, value, expected);
        }
        if let Some(expected) = expected {
            self.require_expression(&value.ty, expected, expr.span)?;
        }
        Ok(value)
    }

    pub(super) fn with_callee_delivery<R>(
        &mut self,
        parent: hir::HirId,
        delivery: Option<hir::HirId>,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let saved = self.comptime_callee_result;
        if saved == Some(parent) {
            self.comptime_callee_result = delivery;
        }
        let result = check(self);
        self.comptime_callee_result = saved;
        result
    }

    pub(super) fn deliver_comptime_result(
        &mut self,
        expr: &Expr<'_>,
        value: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        // A comptime callable or constructor Type used as the next callee
        // never enters runtime data flow. Keep the checked producer for that
        // invocation or construction instead of requesting a Splice conversion.
        if self.comptime_callee_result == Some(expr.hir_id)
            && (value.ty.is_comptime_callable() || value.ty == p(P::Type))
        {
            return Ok(value);
        }
        self.splice_value(expr, value, expected)
    }

    pub(super) fn runtime_value_context(&self) -> bool {
        if self.observing_type || self.in_required_comptime || self.in_staged_input {
            return false;
        }
        // A quote checks runtime code inside a comptime producer. Nested
        // closures own their execution phase; the enclosing declaration only
        // supplies the phase when neither boundary is active.
        if let Some(frame) = self.captures.last() {
            return !frame.comptime;
        }
        self.in_code_body
            || !self
                .signature_owner
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| {
                    matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                        if sig.modifiers.is_comptime)
                })
    }

    pub(super) fn splice_constant(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let mut value = self.constant_value(owner, expr.span)?;
        value.source = source(expr.span);
        self.splice_value(expr, value, expected)
    }

    pub(super) fn splice_value(
        &mut self,
        expr: &Expr<'_>,
        value: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let id = self.protocols.splice.ok_or_else(|| error(expr.span,
            "constant runtime use requires cross-stage Splice conversion with a bound Splice protocol"))?;
        if !self.interfaces_ready {
            return Err(error(
                expr.span,
                "Splice conversion is waiting for the trait implementation world",
            ));
        }
        crate::splice::protocol(&self.checked_traits, id).map_err(|e| error(expr.span, e))?;
        let value_ty = value.ty.clone();
        match self.selected_method(
            value_ty.clone(),
            Some(n::InterfaceInstance {
                declaration: id,
                arguments: vec![],
            }),
            "splice",
            expr.span,
        ) {
            Ok(Some(callee)) => {
                // A prepared prefix is already an elaborated input; finish_call still
                // checks ordinary contracts, specializes staged inputs and preserves
                // the trait origin. It does not invoke this runtime method at comptime.
                return self.finish_call(expr, callee, &[], vec![value], expected, None);
            }
            Ok(None) => return Err(error(expr.span, "Splice conversion has no selected method")),
            Err(failure) => {
                if !failure.retry_after_capability_progress
                    && let Some(spliced) = self.try_auto_splice_value(expr, id, value, expected)?
                {
                    return Ok(spliced);
                }
                let mut failure = failure;
                // Preserve the selection failure and its retry classification.
                // The implicit conversion otherwise looks like a failed lookup
                // of the source field or method at this same source range.
                failure.notes.push(DiagnosticNote {
                    source: source(expr.span),
                    message: format!(
                        "while selecting cross-stage Splice conversion from {value_ty}"
                    ),
                });
                return Err(failure);
            }
        }
    }

    fn try_auto_splice_value(
        &mut self,
        expr: &Expr<'_>,
        splice_id: DeclarationId,
        value: n::Expr,
        expected: Option<&Term>,
    ) -> Result<Option<n::Expr>, Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let mut visiting = Vec::new();
        let target =
            self.auto_splice_target(splice_id, &value.ty, expr.span, 0, &mut visiting, &mut fuel);
        self.required_steps = self.required_steps.saturating_sub((before - fuel) as u64);
        let Some(target_ty) = target? else {
            return Ok(None);
        };
        let spliced = self.lower_auto_spliced(expr, splice_id, value, &target_ty)?;
        if let Some(expected) = expected {
            self.require_expression(&spliced.ty, expected, expr.span)?;
        }
        Ok(Some(spliced))
    }

    fn auto_splice_target(
        &mut self,
        splice_id: DeclarationId,
        ty: &Term,
        span: Span,
        depth: usize,
        visiting: &mut Vec<Term>,
        fuel: &mut usize,
    ) -> Result<Option<Term>, Error> {
        if depth >= 128 {
            return Err(error(
                span,
                "auto-Splice structural recursion depth exceeded",
            ));
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "auto-Splice structural check budget exhausted"))?;
        let shape = self.structural_type(ty, span)?;
        if visiting.contains(&shape) {
            return Err(error(
                span,
                format!("cannot auto-derive Splice for cyclic type `{ty}`"),
            ));
        }
        visiting.push(shape.clone());
        let result = (|| -> Result<Option<Term>, Error> {
            let mut drop_visited = Vec::new();
            if self.term_has_drop(&self.context.clone(), ty, &mut drop_visited, span, fuel)? {
                return Err(error(
                    span,
                    format!("cannot auto-derive Splice for Drop type `{ty}`"),
                ));
            }
            if matches!(shape, Term::Pointer(_) | Term::Lift(_)) {
                return Err(error(
                    span,
                    format!("cannot auto-derive Splice across shared reference or lift `{ty}`"),
                ));
            }
            if depth > 0 {
                let interface = n::InterfaceInstance {
                    declaration: splice_id,
                    arguments: vec![],
                };
                if let Ok(Some(callee)) =
                    self.selected_method(ty.clone(), Some(interface.clone()), "splice", span)
                {
                    if let Some(target_member) = self
                        .checked_traits
                        .by_id(splice_id)
                        .and_then(|contract| {
                            contract
                                .associated
                                .iter()
                                .find(|member| member.name == "RuntimeTarget")
                        })
                        .map(|member| member.declaration)
                        && let Ok(resolution) = self.resolve_trait_capability(
                            &interface,
                            ty,
                            crate::ptc::Budget::default(),
                        )
                        && let crate::implementation::Selection::Selected(id) = resolution.selection
                        && let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
                            resolution.assessments.get(&id)
                    {
                        let implementation =
                            &self.checked_implementations[self.implementation_positions[&id]];
                        if let Some((_, expr)) = implementation
                            .associated
                            .iter()
                            .find(|(member, _)| *member == target_member)
                            && let K::TypeValue(defined_target) = &expr.kind
                        {
                            let instantiated = self.instantiate_input_types(
                                defined_target,
                                &implementation.type_parameters,
                                &evidence.type_arguments,
                                span,
                            )?;
                            return Ok(Some(instantiated));
                        }
                    }
                    let Term::Function { result, .. } = callee.ty.callable_signature() else {
                        return Err(error(span, "Splice method has non-function signature"));
                    };
                    return Ok(Some(result.as_ref().clone()));
                }
            }
            let require_child =
                |this: &mut Self, child: &Term, visiting: &mut Vec<Term>, fuel: &mut usize| {
                    this.auto_splice_target(splice_id, child, span, depth + 1, visiting, fuel)?
                    .ok_or_else(|| {
                        error(
                            span,
                            format!(
                                "cannot auto-derive Splice for non-spliceable component `{child}`"
                            ),
                        )
                    })
                };
            match &shape {
                Term::Tuple(elements) => {
                    let mut targets = Vec::with_capacity(elements.len());
                    let mut unchanged = true;
                    for element in elements {
                        let target = require_child(self, element, visiting, fuel)?;
                        unchanged &= self.structural_type(&target, span)?
                            == self.structural_type(element, span)?;
                        targets.push(target);
                    }
                    Ok(Some(if unchanged {
                        ty.clone()
                    } else {
                        Term::Tuple(targets)
                    }))
                }
                Term::Record(fields) => {
                    let mut targets = Vec::with_capacity(fields.len());
                    let mut unchanged = true;
                    for (name, field_ty) in fields {
                        let target = require_child(self, field_ty, visiting, fuel)?;
                        unchanged &= self.structural_type(&target, span)?
                            == self.structural_type(field_ty, span)?;
                        targets.push((name.clone(), target));
                    }
                    Ok(Some(if unchanged {
                        ty.clone()
                    } else {
                        Term::Record(targets)
                    }))
                }
                Term::Optional(inner) => {
                    let target = require_child(self, inner, visiting, fuel)?;
                    Ok(Some(
                        if self.structural_type(&target, span)?
                            == self.structural_type(inner, span)?
                        {
                            ty.clone()
                        } else {
                            Term::Optional(Box::new(target))
                        },
                    ))
                }
                Term::Nominal {
                    declaration,
                    arguments,
                } => {
                    if let Some(struct_def) = self.structs.get(declaration).cloned() {
                        let fields = nir::normal_forms::instantiate_value_fields_checked(
                            &struct_def.type_parameters,
                            &struct_def.parameter_types,
                            arguments,
                            struct_def.fields.iter().map(|(_, ty)| ty),
                            fuel,
                        )
                        .map_err(|e| error(span, e.to_string()))?;
                        for field_ty in &fields {
                            let target = require_child(self, field_ty, visiting, fuel)?;
                            if self.structural_type(&target, span)?
                                != self.structural_type(field_ty, span)?
                            {
                                return Err(error(
                                    span,
                                    format!(
                                        "auto-Splice on struct field `{field_ty}` cannot change layout to `{target}`"
                                    ),
                                ));
                            }
                        }
                        return Ok(Some(ty.clone()));
                    }
                    if let Some(enum_def) = self.enums.get(declaration).cloned() {
                        for variant in &enum_def.variants {
                            let fields = nir::normal_forms::instantiate_value_fields_checked(
                                &enum_def.type_parameters,
                                &enum_def.parameter_types,
                                arguments,
                                variant.fields.iter(),
                                fuel,
                            )
                            .map_err(|e| error(span, e.to_string()))?;
                            for field_ty in &fields {
                                let target = require_child(self, field_ty, visiting, fuel)?;
                                if self.structural_type(&target, span)?
                                    != self.structural_type(field_ty, span)?
                                {
                                    return Err(error(
                                        span,
                                        format!(
                                            "auto-Splice on enum variant field `{field_ty}` cannot change layout to `{target}`"
                                        ),
                                    ));
                                }
                            }
                        }
                        return Ok(Some(ty.clone()));
                    }
                    Ok(None)
                }
                Term::Closure {
                    environment,
                    signature,
                } if !ty.is_comptime_callable() => {
                    let target_sig = require_child(self, signature, visiting, fuel)?;
                    if target_sig != **signature {
                        return Err(error(
                            span,
                            format!(
                                "auto-Splice closure signature `{signature}` cannot change to `{target_sig}`"
                            ),
                        ));
                    }
                    for env_ty in environment {
                        let target = require_child(self, env_ty, visiting, fuel)?;
                        if target != *env_ty {
                            return Err(error(
                                span,
                                format!(
                                    "auto-Splice closure capture `{env_ty}` cannot change layout to `{target}`"
                                ),
                            ));
                        }
                    }
                    Ok(Some(ty.clone()))
                }
                _ => Ok(None),
            }
        })();
        visiting.pop();
        result
    }

    fn term_has_drop(
        &self,
        context: &crate::ptc::Context,
        ty: &Term,
        visited: &mut Vec<Term>,
        span: Span,
        fuel: &mut usize,
    ) -> Result<bool, Error> {
        let Some(drop_protocol) = self.protocols.drop else {
            return Ok(false);
        };
        if !self
            .checked_traits
            .iter()
            .any(|interface| interface.declaration == drop_protocol)
        {
            return Ok(false);
        }
        if visited.contains(ty) {
            return Ok(false);
        }
        visited.push(ty.clone());
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "auto-Splice Drop check budget exhausted"))?;
        let assumptions = [];
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context,
        };
        if let Ok(resolution) = world.resolve(drop_protocol, ty, crate::ptc::Budget::default())
            && matches!(
                resolution.types.selection,
                crate::implementation::Selection::Selected(_)
            )
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn lower_auto_spliced(
        &mut self,
        expr: &Expr<'_>,
        splice_id: DeclarationId,
        value: n::Expr,
        target_ty: &Term,
    ) -> Result<n::Expr, Error> {
        let span = expr.span;
        let loc = source(span);
        let shape = self.structural_type(&value.ty, span)?;
        let target_shape = self.structural_type(target_ty, span)?;
        match (value.kind, shape, target_shape) {
            (K::Tuple(fields), Term::Tuple(_), Term::Tuple(target_fields))
                if fields.len() == target_fields.len() =>
            {
                let spliced = fields
                    .into_iter()
                    .zip(target_fields)
                    .map(|(field, target)| self.splice_value(expr, field, Some(&target)))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::Tuple(spliced),
                })
            }
            (K::AnonymousRecord { fields }, Term::Record(_), Term::Record(target_fields))
                if fields.len() == target_fields.len() =>
            {
                let spliced = fields
                    .into_iter()
                    .zip(target_fields)
                    .map(|(field, (_, target))| self.splice_value(expr, field, Some(&target)))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::AnonymousRecord { fields: spliced },
                })
            }
            (
                K::Record {
                    declaration,
                    fields,
                },
                Term::Nominal {
                    declaration: expected_decl,
                    arguments,
                },
                _,
            ) if declaration == expected_decl => {
                let Some(struct_def) = self.structs.get(&declaration).cloned() else {
                    return Err(error(span, "auto-Splice nominal missing struct definition"));
                };
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let field_types = nir::normal_forms::instantiate_value_fields_checked(
                    &struct_def.type_parameters,
                    &struct_def.parameter_types,
                    &arguments,
                    struct_def.fields.iter().map(|(_, ty)| ty),
                    &mut fuel,
                )
                .map_err(|e| error(span, e.to_string()))?;
                let spliced = fields
                    .into_iter()
                    .zip(field_types)
                    .map(|(field, fty)| self.splice_value(expr, field, Some(&fty)))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::Record {
                        declaration,
                        fields: spliced,
                    },
                })
            }
            (
                K::Variant {
                    declaration,
                    index,
                    fields,
                },
                Term::Nominal {
                    declaration: expected_decl,
                    arguments,
                },
                _,
            ) if declaration == expected_decl => {
                let Some(enum_def) = self.enums.get(&declaration).cloned() else {
                    return Err(error(span, "auto-Splice variant missing enum definition"));
                };
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let field_types = nir::normal_forms::instantiate_value_fields_checked(
                    &enum_def.type_parameters,
                    &enum_def.parameter_types,
                    &arguments,
                    enum_def.variants[index].fields.iter(),
                    &mut fuel,
                )
                .map_err(|e| error(span, e.to_string()))?;
                let spliced = fields
                    .into_iter()
                    .zip(field_types)
                    .map(|(field, fty)| self.splice_value(expr, field, Some(&fty)))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::Variant {
                        declaration,
                        index,
                        fields: spliced,
                    },
                })
            }
            (K::OptionalAbsent, Term::Optional(_), Term::Optional(_)) => Ok(n::Expr {
                source: loc,
                ty: target_ty.clone(),
                kind: K::OptionalAbsent,
            }),
            (K::OptionalPresent(inner), Term::Optional(_), Term::Optional(target_inner)) => {
                let spliced = self.splice_value(expr, *inner, Some(&target_inner))?;
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::OptionalPresent(Box::new(spliced)),
                })
            }
            (kind, _, _) => self.synthesize_auto_splice_call(
                expr,
                splice_id,
                n::Expr { kind, ..value },
                target_ty,
            ),
        }
    }

    fn synthesize_auto_splice_call(
        &mut self,
        expr: &Expr<'_>,
        splice_id: DeclarationId,
        value: n::Expr,
        target_ty: &Term,
    ) -> Result<n::Expr, Error> {
        let span = expr.span;
        let loc = source(span);
        let helper_decl = self.generated_functions.fresh_identity(span)?;
        let mut type_scope = std::collections::BTreeMap::new();
        for term in [&value.ty, target_ty] {
            let rigids = term
                .rigid_parameters()
                .map_err(|e| error(span, e.to_string()))?;
            for (id, sort) in rigids {
                if sort != nir::Sort::Type {
                    return Err(error(
                        span,
                        "auto-Splice cannot depend on non-Type rigid parameter",
                    ));
                }
                type_scope.insert(id, sort);
            }
        }
        let scope_ids: BTreeSet<DeclarationId> = type_scope.keys().copied().collect();
        let (requirements, capabilities) = self.snapshot_contract_in_scope(&scope_ids, span)?;
        let data_id = n::LocalId {
            owner: helper_decl.local,
            index: 0,
        };
        let mut next_local = 1u32;
        let data_expr = n::Expr {
            source: loc,
            ty: value.ty.clone(),
            kind: K::Local(data_id),
        };
        let saved_locals = self.locals.clone();
        let saved_captures = std::mem::take(&mut self.captures);
        self.locals.insert(data_id, value.ty.clone());
        let body = self.build_auto_splice_body(
            expr,
            splice_id,
            helper_decl.local,
            &mut next_local,
            data_expr,
            target_ty,
        );
        self.locals = saved_locals;
        self.captures = saved_captures;
        let body = body?;
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::ComptimeValue,
                ty: value.ty.clone(),
            }],
            result: Box::new(target_ty.clone()),
        };
        self.generated_functions.publish(n::Function {
            interface: nir::program::FunctionDeclaration {
                source: loc,
                declaration: helper_decl,
                name: "<auto-splice>".into(),
                phase: n::FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                signature: signature.clone(),
                parameters: vec![data_id],
                env_parameters: vec![],
                preconditions: vec![],
                postconditions: vec![],
                entry_snapshots: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                type_parameters: scope_ids.into_iter().collect(),
                requirements,
                capabilities,
                defaults: vec![],
            },
            body,
            runtime_deliveries: Default::default(),
        });
        let type_args: Vec<Term> = type_scope
            .into_iter()
            .map(|(declaration, sort)| Term::Rigid { declaration, sort })
            .collect();
        let callee = n::Expr {
            source: loc,
            ty: signature,
            kind: if type_args.is_empty() {
                K::Function(helper_decl)
            } else {
                K::FunctionInstance {
                    declaration: helper_decl,
                    arguments: type_args,
                }
            },
        };
        Ok(n::Expr {
            source: loc,
            ty: target_ty.clone(),
            kind: K::Call {
                callee: Box::new(callee),
                arguments: vec![n::CallArgument {
                    passing: n::ArgumentPassing::Value,
                    parameter: 0,
                    value,
                    is_default: false,
                }],
                parameter_bindings: vec![data_id],
                env_arguments: vec![],
            },
        })
    }

    fn splice_auto_child(
        &mut self,
        expr: &Expr<'_>,
        splice_id: DeclarationId,
        child: n::Expr,
        target_ty: &Term,
    ) -> Result<n::Expr, Error> {
        let span = expr.span;
        match self.selected_method(
            child.ty.clone(),
            Some(n::InterfaceInstance {
                declaration: splice_id,
                arguments: vec![],
            }),
            "splice",
            span,
        ) {
            Ok(Some(callee)) => {
                let parameter_bindings = match &callee.kind {
                    K::TraitMethod { member, .. } => self
                        .checked_traits
                        .iter()
                        .find(|interface| interface.declaration == splice_id)
                        .and_then(|interface| {
                            interface
                                .methods
                                .iter()
                                .find(|method| method.declaration == *member)
                        })
                        .map(|method| method.parameter_bindings.clone())
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                Ok(n::Expr {
                    source: source(span),
                    ty: target_ty.clone(),
                    kind: K::Call {
                        callee: Box::new(callee),
                        arguments: vec![n::CallArgument {
                            passing: n::ArgumentPassing::Value,
                            parameter: 0,
                            value: child,
                            is_default: false,
                        }],
                        parameter_bindings,
                        env_arguments: vec![],
                    },
                })
            }
            Ok(None) => Err(error(span, "Splice conversion has no selected method")),
            Err(failure) => {
                if !failure.retry_after_capability_progress {
                    return self.lower_auto_spliced(expr, splice_id, child, target_ty);
                }
                Err(failure)
            }
        }
    }

    fn build_auto_splice_body(
        &mut self,
        expr: &Expr<'_>,
        splice_id: DeclarationId,
        owner_local: u32,
        next_local: &mut u32,
        data: n::Expr,
        target_ty: &Term,
    ) -> Result<n::Expr, Error> {
        let span = expr.span;
        let loc = source(span);
        let shape = self.structural_type(&data.ty, span)?;
        let target_shape = self.structural_type(target_ty, span)?;
        match (&shape, &target_shape) {
            (Term::Tuple(elements), Term::Tuple(target_elements))
                if elements.len() == target_elements.len() =>
            {
                let mut pattern_fields = Vec::with_capacity(elements.len());
                let mut locals = Vec::with_capacity(elements.len());
                for element_ty in elements {
                    let id = n::LocalId {
                        owner: owner_local,
                        index: *next_local,
                    };
                    *next_local += 1;
                    self.locals.insert(id, element_ty.clone());
                    locals.push((id, element_ty.clone()));
                    pattern_fields.push(n::Pattern::Binding {
                        binding: id,
                        inner: Box::new(n::Pattern::Wild),
                    });
                }
                let let_pat = n::Expr {
                    source: loc,
                    ty: Term::Primitive(P::Void),
                    kind: K::LetPattern {
                        pattern: n::Pattern::Tuple(pattern_fields),
                        value: Box::new(data),
                        guard: None,
                        otherwise: None,
                    },
                };
                let spliced = locals
                    .into_iter()
                    .zip(target_elements)
                    .map(|((id, fty), target)| {
                        self.splice_auto_child(
                            expr,
                            splice_id,
                            n::Expr {
                                source: loc,
                                ty: fty,
                                kind: K::Local(id),
                            },
                            target,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let tuple = n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::Tuple(spliced),
                };
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::Block {
                        statements: vec![let_pat],
                        result: Some(Box::new(tuple)),
                    },
                })
            }
            (Term::Record(fields), Term::Record(target_fields))
                if fields.len() == target_fields.len() =>
            {
                let spliced = fields
                    .iter()
                    .zip(target_fields)
                    .enumerate()
                    .map(|(index, ((name, fty), (_, target)))| {
                        let projected = n::Expr {
                            source: loc,
                            ty: fty.clone(),
                            kind: K::AnonymousField {
                                value: Box::new(data.clone()),
                                name: name.clone(),
                                index,
                            },
                        };
                        self.splice_auto_child(expr, splice_id, projected, target)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::AnonymousRecord { fields: spliced },
                })
            }
            (
                Term::Nominal {
                    declaration,
                    arguments,
                },
                _,
            ) => {
                if let Some(struct_def) = self.structs.get(declaration).cloned() {
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let field_types = nir::normal_forms::instantiate_value_fields_checked(
                        &struct_def.type_parameters,
                        &struct_def.parameter_types,
                        arguments,
                        struct_def.fields.iter().map(|(_, ty)| ty),
                        &mut fuel,
                    )
                    .map_err(|e| error(span, e.to_string()))?;
                    let spliced = field_types
                        .into_iter()
                        .enumerate()
                        .map(|(index, fty)| {
                            let projected = n::Expr {
                                source: loc,
                                ty: fty.clone(),
                                kind: K::Field {
                                    value: Box::new(data.clone()),
                                    declaration: *declaration,
                                    index,
                                },
                            };
                            self.splice_auto_child(expr, splice_id, projected, &fty)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(n::Expr {
                        source: loc,
                        ty: target_ty.clone(),
                        kind: K::Record {
                            declaration: *declaration,
                            fields: spliced,
                        },
                    });
                }
                if let Some(enum_def) = self.enums.get(declaration).cloned() {
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let mut arms = Vec::with_capacity(enum_def.variants.len());
                    for (index, variant) in enum_def.variants.iter().enumerate() {
                        let field_types = nir::normal_forms::instantiate_value_fields_checked(
                            &enum_def.type_parameters,
                            &enum_def.parameter_types,
                            arguments,
                            variant.fields.iter(),
                            &mut fuel,
                        )
                        .map_err(|e| error(span, e.to_string()))?;
                        let mut pat_fields = Vec::with_capacity(field_types.len());
                        let mut locals = Vec::with_capacity(field_types.len());
                        for fty in field_types {
                            let id = n::LocalId {
                                owner: owner_local,
                                index: *next_local,
                            };
                            *next_local += 1;
                            self.locals.insert(id, fty.clone());
                            locals.push((id, fty));
                            pat_fields.push(n::Pattern::Binding {
                                binding: id,
                                inner: Box::new(n::Pattern::Wild),
                            });
                        }
                        let spliced = locals
                            .into_iter()
                            .map(|(id, fty)| {
                                self.splice_auto_child(
                                    expr,
                                    splice_id,
                                    n::Expr {
                                        source: loc,
                                        ty: fty.clone(),
                                        kind: K::Local(id),
                                    },
                                    &fty,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        arms.push(n::MatchArm {
                            pattern: n::Pattern::Variant {
                                declaration: *declaration,
                                index,
                                fields: pat_fields,
                            },
                            guard: None,
                            body: n::Expr {
                                source: loc,
                                ty: target_ty.clone(),
                                kind: K::Variant {
                                    declaration: *declaration,
                                    index,
                                    fields: spliced,
                                },
                            },
                        });
                    }
                    return Ok(n::Expr {
                        source: loc,
                        ty: target_ty.clone(),
                        kind: K::Match {
                            value: Box::new(data),
                            arms,
                        },
                    });
                }
                Ok(data)
            }
            (Term::Optional(inner), Term::Optional(target_inner)) => {
                let id = n::LocalId {
                    owner: owner_local,
                    index: *next_local,
                };
                *next_local += 1;
                self.locals.insert(id, inner.as_ref().clone());
                let present = self.splice_auto_child(
                    expr,
                    splice_id,
                    n::Expr {
                        source: loc,
                        ty: inner.as_ref().clone(),
                        kind: K::Local(id),
                    },
                    target_inner,
                )?;
                Ok(n::Expr {
                    source: loc,
                    ty: target_ty.clone(),
                    kind: K::Match {
                        value: Box::new(data),
                        arms: vec![
                            n::MatchArm {
                                pattern: n::Pattern::OptionalPresent(Box::new(
                                    n::Pattern::Binding {
                                        binding: id,
                                        inner: Box::new(n::Pattern::Wild),
                                    },
                                )),
                                guard: None,
                                body: n::Expr {
                                    source: loc,
                                    ty: target_ty.clone(),
                                    kind: K::OptionalPresent(Box::new(present)),
                                },
                            },
                            n::MatchArm {
                                pattern: n::Pattern::OptionalAbsent,
                                guard: None,
                                body: n::Expr {
                                    source: loc,
                                    ty: target_ty.clone(),
                                    kind: K::OptionalAbsent,
                                },
                            },
                        ],
                    },
                })
            }
            _ => Ok(data),
        }
    }
}
