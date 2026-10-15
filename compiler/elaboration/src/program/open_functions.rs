//! Source definitions whose explicit deferred annotations need invocation inputs.
//! Ordinary descriptions never enter Term or typed NIR. Only checked concrete
//! instances are published to the runtime package.
use super::required::prepared::PreparedRequired;
use super::*;
use crate::gradual::FrozenDescription as D;
use std::sync::Arc;

#[derive(Clone)]
pub(super) enum Annotation {
    Known(Term),
    Pending(Arc<PreparedRequired>),
}
impl Annotation {
    pub(super) fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_))
    }
    fn description(&self) -> D {
        match self {
            Self::Known(ty) => D::Known(ty.clone()),
            Self::Pending(_) => D::Any,
        }
    }
}
impl std::fmt::Display for Annotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Known(ty) => write!(f, "(known {ty})"),
            Self::Pending(code) => write!(f, "(deferred {code})"),
        }
    }
}
#[derive(Clone)]
pub(super) struct Definition {
    parameters: Vec<(Sort, Annotation)>,
    result: Annotation,
    checked: bool,
}
impl Definition {
    pub(super) fn input_sorts(&self) -> Vec<Sort> {
        self.parameters.iter().map(|(sort, _)| *sort).collect()
    }
    pub(super) fn known_prefix(&self) -> Vec<Parameter> {
        self.parameters
            .iter()
            .map_while(|(sort, annotation)| match annotation {
                Annotation::Known(ty) => Some(Parameter {
                    sort: *sort,
                    ty: ty.clone(),
                }),
                Annotation::Pending(_) => None,
            })
            .collect()
    }
}
impl std::fmt::Display for Definition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(open-function (parameters")?;
        for (sort, annotation) in &self.parameters {
            write!(f, " (parameter {sort} {annotation})")?;
        }
        write!(
            f,
            ") (result {}) (definition-checked {}))",
            self.result, self.checked
        )
    }
}

impl Checker<'_, '_> {
    pub(super) fn signature_annotation(&mut self, expr: &Expr<'_>) -> Result<Annotation, Error> {
        let H::Defer(value) = &expr.kind else {
            return self.type_expr(expr).map(Annotation::Known);
        };
        // A direct Type input already has a stable symbolic identity.
        if !matches!(
            value.kind,
            H::Application(..)
                | H::ExtendedApplication(..)
                | H::Index(..)
                | H::If(..)
                | H::When(_)
                | H::Match(..)
                | H::Block(_)
        ) {
            return self.type_expr(value).map(Annotation::Known);
        }
        let prepared =
            self.prepare_required_expression(value, Some(&p(P::Type)), None, vec![], true)?;
        if prepared.inputs.iter().any(|(_, _, value)| value.is_none()) {
            Ok(Annotation::Pending(Arc::new(prepared)))
        } else {
            let (value, _) = prepared.invoke(&mut self.required_steps, expr.span)?;
            let comptime_interpretor::Value::Type(ty) = value else {
                return Err(error(
                    expr.span,
                    "deferred annotation did not produce a Type",
                ));
            };
            Ok(Annotation::Known(ty))
        }
    }

    pub(super) fn register_open_function(
        &mut self,
        owner: OwnerId,
        parameters: Vec<(Sort, Annotation)>,
        result: Annotation,
        span: Span,
    ) -> Result<(), Error> {
        let Some(ItemKind::Fn(sig, _)) = self.package.item(owner).map(|item| &item.kind) else {
            return Err(error(
                span,
                "deferred interface and effect signatures are not yet implemented",
            ));
        };
        if sig.modifiers.is_comptime
            || sig.modifiers.is_normal_form_head
            || !sig.clause_params.is_empty()
            || self.self_containers.contains_key(&owner)
            || self.self_types.contains_key(&owner)
        {
            return Err(error(
                span,
                "open signatures with declaration parameters, methods, or comptime bodies are not yet implemented",
            ));
        }
        if sig.modifiers.is_pure {
            return Err(error(
                span,
                "runtime pure open definitions require definition-side behavior evidence not yet represented",
            ));
        }
        for parameter in sig.params {
            if !matches!(
                parameter.ty.kind,
                TyParamKind::Positional(_)
                    | TyParamKind::PositionalDependencyCatched(..)
                    | TyParamKind::Optional(..)
                    | TyParamKind::Varadic(..)
            ) || parameter.ty.is_implicit()
                || parameter.ty.is_lambda()
            {
                return Err(error(
                    parameter.span,
                    "open signature parameter routing protocols are not yet implemented",
                ));
            }
        }
        self.open_functions.insert(
            owner,
            Definition {
                parameters,
                result,
                checked: false,
            },
        );
        Ok(())
    }

    // Isolate definition/instance checking from the caller's locals, return and
    // control-flow context. Global demanded declarations and budgets survive.
    fn in_open_function<T>(
        &mut self,
        owner: OwnerId,
        check: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let saved_owner = self.signature_owner.replace(owner);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_result = self.result.take();
        let saved_partial = self.partial_return_context.take();
        let saved_optional = std::mem::take(&mut self.pending_optional_returns);
        let saved_returns = self.return_observations.take();
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_immutable = std::mem::take(&mut self.immutable);
        let saved_context = self.context.clone();
        let saved_contract = std::mem::replace(&mut self.checking_contract, false);
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let saved_staged = std::mem::replace(&mut self.in_staged_input, false);
        let saved_required = std::mem::replace(&mut self.in_required_comptime, false);
        let saved_inputs = std::mem::take(&mut self.specialization_inputs);
        let saved_default_scope = self.default_input_scope.take();
        let saved_effects = self.effect_environment.take();
        let saved_resume = self.effect_resume_result.take();
        let saved_type_bindings = self.type_input_bindings.clone();
        let saved_value_bindings = self.value_input_bindings.clone();
        self.context = self.definition_context(owner);
        let result = check(self);
        self.signature_owner = saved_owner;
        self.locals = saved_locals;
        self.captures = saved_captures;
        self.result = saved_result;
        self.partial_return_context = saved_partial;
        self.pending_optional_returns = saved_optional;
        self.return_observations = saved_returns;
        self.loops = saved_loops;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.checking_contract = saved_contract;
        self.numeric_defaults = saved_numeric;
        self.in_staged_input = saved_staged;
        self.in_required_comptime = saved_required;
        self.specialization_inputs = saved_inputs;
        self.default_input_scope = saved_default_scope;
        self.effect_environment = saved_effects;
        self.effect_resume_result = saved_resume;
        self.type_input_bindings = saved_type_bindings;
        self.value_input_bindings = saved_value_bindings;
        result
    }

    pub(super) fn check_open_definition(&mut self, owner: OwnerId) -> Result<(), Error> {
        let definition = self.open_functions[&owner].clone();
        if definition.checked {
            return Ok(());
        }
        let span = self.package.item(owner).unwrap().span;
        self.ensure_contract(owner, span)?;
        if self.active_open_definitions.len() >= 64 || !self.active_open_definitions.insert(owner) {
            return Err(error(
                self.package.item(owner).unwrap().span,
                "cyclic or excessively deep ordinary open-definition dependency",
            ));
        }
        let checked = self.in_open_function(owner, |this| {
            let item = this.package.item(owner).unwrap();
            let ItemKind::Fn(sig, body) = &item.kind else {
                unreachable!()
            };
            let metadata = n::Package {
                structs: this.structs.values().cloned().collect(),
                enums: this.enums.values().cloned().collect(),
                normal_forms: this.normal_form_metadata(),
                ..Default::default()
            };
            let frames = vec![
                definition
                    .parameters
                    .iter()
                    .map(|(sort, _)| *sort)
                    .collect(),
            ];
            for (_, annotation) in definition
                .parameters
                .iter()
                .filter(|(sort, _)| *sort == Sort::Runtime)
                .chain(std::iter::once(&(Sort::Runtime, definition.result.clone())))
            {
                if let Annotation::Known(ty) = annotation {
                    nir::phases::validate_runtime_storage(ty, &metadata, &frames)
                        .map_err(|message| error(item.span, message))?;
                }
            }
            let mut locals = BTreeMap::new();
            for (slot, (parameter, (sort, annotation))) in
                sig.params.iter().zip(&definition.parameters).enumerate()
            {
                let description = annotation.description();
                if let TyParamKind::Optional(_, _, default) = parameter.ty.kind {
                    let mut default_locals = locals.clone();
                    this.with_staged_input(*sort != Sort::Runtime, |this| {
                        let actual = this.describe_open_expression(
                            default,
                            &mut default_locals,
                            None,
                            Some(&description),
                            0,
                        )?;
                        if *sort == Sort::Runtime && matches!(actual, D::Any) {
                            return Err(error(
                                default.span,
                                "Any supplies no P3 stability evidence for this default",
                            ));
                        }
                        let expected = match &description {
                            D::Known(ty) => Some(ty),
                            D::Any => None,
                        };
                        let value = this.expr(default, expected)?;
                        if *sort == Sort::Runtime {
                            this.require_stable_open_default(&value, default.span)?;
                        }
                        // Keep the whole default together so branches retain
                        // their proof facts. This checks operations, not values:
                        // an overridden comptime supplier still never executes.
                        this.check_open_fragment(&value, default.span, true)?;
                        Ok(())
                    })?;
                }
                if let D::Known(ty) = &description {
                    this.locals.insert(local(parameter.hir_id), ty.clone());
                }
                if *sort == Sort::Type {
                    this.type_input_bindings.insert(
                        local(parameter.hir_id),
                        Term::Bound {
                            depth: 0,
                            index: slot,
                            sort: Sort::Type,
                        },
                    );
                }
                locals.insert(local(parameter.hir_id), description);
            }
            let preconditions = this
                .program_preconditions(owner)
                .map_err(open_contract_error)?;
            if let Some(predicate) = preconditions.into_iter().reduce(|left, right| n::Expr {
                source: source(item.span),
                ty: p(P::Bool),
                kind: K::And(Box::new(left), Box::new(right)),
            }) {
                // Validate entry predicates before assuming them, preserving
                // short-circuit facts from earlier conjuncts.
                this.check_open_fragment(&predicate, item.span, true)?;
            }
            let result_evidence = match &definition.result {
                Annotation::Known(ty) => sig.return_binding.map(|binding| (local(binding), ty)),
                Annotation::Pending(_) => None,
            };
            this.program_postconditions_with_result(owner, result_evidence)
                .map_err(open_contract_error)?;
            let body = this
                .package
                .body(*body)
                .ok_or_else(|| error(item.span, "missing open function body"))?;
            let result = definition.result.description();
            this.result = match &result {
                D::Known(ty) => Some(ty.clone()),
                D::Any => None,
            };
            this.describe_open_expression(
                body.value,
                &mut locals,
                Some(&result),
                Some(&result),
                0,
            )?;
            Ok(())
        });
        self.active_open_definitions.remove(&owner);
        checked?;
        self.open_functions.get_mut(&owner).unwrap().checked = true;
        Ok(())
    }

    fn check_open_fragment(
        &mut self,
        value: &n::Expr,
        span: Span,
        arithmetic: bool,
    ) -> Result<(), Error> {
        let owner = self.signature_owner.unwrap();
        if self.in_staged_input {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let free = nir::free_bindings::collect(value, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            for binding in free.map_err(|message| error(span, message))? {
                if self.required_runtime_parameter(binding) {
                    return Err(error(
                        span,
                        "required comptime computation cannot read a runtime parameter",
                    ));
                }
                if !self.type_input_bindings.contains_key(&binding)
                    && !self.value_input_bindings.contains_key(&binding)
                {
                    return Err(error(
                        span,
                        "staged local provenance in an Any definition is not yet implemented",
                    ));
                }
            }
        }
        let definition = &self.open_functions[&owner];
        let ItemKind::Fn(sig, _) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
        let mut parameters = Vec::new();
        let mut bindings = Vec::new();
        for (parameter, (sort, annotation)) in sig.params.iter().zip(&definition.parameters) {
            if let Annotation::Known(ty) = annotation {
                parameters.push(Parameter {
                    sort: *sort,
                    ty: ty.clone(),
                });
                bindings.push(local(parameter.hir_id));
            }
        }
        let mut members = BTreeSet::new();
        let mut associated = BTreeSet::new();
        let mut pending = required::references(value, &mut members, &mut associated)
            .into_iter()
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        let mut functions = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(owner) = self.package.owners().find_map(|(owner, _)| {
                (declaration(self.package, &self.package_identities, owner) == id).then_some(owner)
            }) else {
                continue;
            };
            if matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
            {
                let function = self.required_function(owner, span)?;
                pending.extend(required::references(
                    &function.body,
                    &mut members,
                    &mut associated,
                ));
                functions.push(function);
            } else {
                self.ensure_signature(owner, span)?;
            }
        }
        functions.push(n::Function {
            declaration: DeclarationId {
                package: u64::MAX,
                local: u32::MAX,
            },
            name: "<ordinary-deferred-region>".into(),
            phase: if self.in_staged_input {
                n::FunctionPhase::Comptime
            } else {
                n::FunctionPhase::Runtime
            },
            inline: false,
            pure: false,
            signature: Term::Function {
                parameters,
                result: Box::new(value.ty.clone()),
            },
            parameters: bindings,
            body: value.clone(),
            defaults: vec![],
            type_parameters: vec![],
            preconditions: vec![],
            postconditions: vec![],
            entry_snapshots: vec![],
            result_binding: None,
            requirements: self.definition_requirements(owner),
            capabilities: self.capabilities.get(&owner).cloned().unwrap_or_default(),
        });
        let runtime_function_identities = self.signatures.iter().filter_map(|(owner, signature)| {
            matches!(&self.package.item(*owner)?.kind, ItemKind::Fn(sig, _) if !sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
                .then(|| n::RuntimeFunctionIdentity {
                    declaration: declaration(self.package, &self.package_identities, *owner),
                    signature: signature.clone(),
                    type_parameters: self.type_parameters[owner].clone(),
                })
        }).collect();
        let package = n::Package {
            functions,
            runtime_function_identities,
            normal_forms: self.normal_form_metadata(),
            structs: self.structs.values().cloned().collect(),
            enums: self.enums.values().cloned().collect(),
            traits: self.checked_traits.clone(),
            ..Default::default()
        };
        if arithmetic {
            crate::verification::flow::validate_machine_operations(&package)
                .map_err(|failure| error(span, failure.to_string()))
        } else {
            nir::phases::validate(&package).map_err(|failure| error(span, failure.message))
        }
    }

    fn require_description(
        &self,
        actual: &D,
        expected: Option<&D>,
        span: Span,
    ) -> Result<(), Error> {
        if let (D::Known(actual), Some(D::Known(expected))) = (actual, expected) {
            self.require_expression(actual, expected, span)?;
        }
        Ok(())
    }

    pub(super) fn assume_open_continuation(&mut self, expr: &Expr<'_>) -> Result<(), Error> {
        if let Some(fact) = self.open_continuation_fact(expr, 0)? {
            self.context
                .validate_formula(&fact, &mut crate::ptc::Budget::default())
                .map_err(|failure| error(expr.span, failure.to_string()))?;
            self.context.assume(fact);
        }
        Ok(())
    }

    fn open_continuation_fact(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
    ) -> Result<Option<crate::ptc::Formula>, Error> {
        use crate::ptc::Formula as F;
        if depth >= 128 {
            return Err(error(expr.span, "open continuation fact depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(expr.span, "open continuation fact budget exhausted"))?;
        Ok(match &expr.kind {
            H::Assumes(condition) if self.is_type_assumption(condition) => {
                Some(F::from_predicate(&self.type_predicate(condition)?))
            }
            H::Semi(value) => self.open_continuation_fact(value, depth + 1)?,
            H::Let(binding) => match binding.init {
                Some(value) => self.open_continuation_fact(value, depth + 1)?,
                None => None,
            },
            H::Block(block) => {
                let mut facts = Vec::new();
                for statement in block.stmts.iter().chain(block.expr) {
                    if let Some(fact) = self.open_continuation_fact(statement, depth + 1)? {
                        facts.push(fact);
                    }
                }
                if facts.is_empty() {
                    None
                } else {
                    Some(F::And(facts))
                }
            }
            H::If(_, yes, no) => {
                let yes_expr = Expr {
                    kind: H::Block(yes),
                    ..expr.clone()
                };
                let yes_fact = self.open_continuation_fact(&yes_expr, depth + 1)?;
                let no_fact = match no {
                    Some(no) => self.open_continuation_fact(no, depth + 1)?,
                    None => None,
                };
                if source_always_exits(&yes_expr, depth + 1) {
                    no_fact
                } else if no.is_some_and(|no| source_always_exits(no, depth + 1)) {
                    yes_fact
                } else {
                    match (yes_fact, no_fact) {
                        (Some(yes), Some(no)) if yes == no => Some(yes),
                        (Some(yes), Some(no)) => Some(F::Or(vec![yes, no])),
                        _ => None,
                    }
                }
            }
            // Loop bodies can be skipped or left through break; closure bodies
            // have not executed here. Neither contributes continuation facts.
            _ => None,
        })
    }

    fn describe_open_block(
        &mut self,
        block: &hir::expr::Block<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let saved = locals.clone();
        let saved_known = self.locals.clone();
        let saved_context = self.context.clone();
        let result = (|| {
            let mut completing = true;
            for statement in block.stmts {
                let ty =
                    self.describe_open_expression(statement, locals, returns, None, depth + 1)?;
                completing &= !matches!(ty, D::Known(ref ty) if is_never(ty));
                if completing {
                    self.assume_open_continuation(statement)?;
                }
            }
            let result = match block.expr {
                Some(tail) => {
                    self.describe_open_expression(tail, locals, returns, expected, depth + 1)?
                }
                None => D::Known(p(P::Void)),
            };
            if completing {
                self.require_description(&result, expected, block.span)?;
                Ok(result)
            } else {
                Ok(D::Known(p(P::Never)))
            }
        })();
        *locals = saved;
        self.locals = saved_known;
        self.context = saved_context;
        result
    }

    fn describe_open_expression(
        &mut self,
        expr: &Expr<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        if depth >= 128 {
            return Err(error(expr.span, "ordinary deferred body depth exhausted"));
        }
        self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
            error(
                expr.span,
                "ordinary deferred body checking budget exhausted",
            )
        })?;
        let result = match &expr.kind {
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                if self.in_staged_input && self.required_runtime_parameter(local(*id)) {
                    return Err(error(
                        expr.span,
                        "required comptime computation cannot read a runtime parameter",
                    ));
                }
                locals
                    .get(&local(*id))
                    .cloned()
                    .ok_or_else(|| error(expr.span, "local binding is unavailable in this scope"))?
            }
            H::When(arms) => {
                self.describe_open_when(expr, arms, locals, returns, expected, depth + 1)?
            }
            H::Match(value, arms) => {
                self.describe_open_match(expr, value, arms, locals, returns, expected, depth + 1)?
            }
            H::Application(callee, args) => {
                self.describe_open_call(expr, callee, args, locals, returns, expected, depth + 1)?
            }
            H::Block(block) => {
                return self.describe_open_block(block, locals, returns, expected, depth + 1);
            }
            H::Semi(inner) => {
                let ty = self.describe_open_expression(inner, locals, returns, None, depth + 1)?;
                if matches!(ty, D::Known(ref ty) if is_never(ty)) {
                    ty
                } else {
                    D::Known(p(P::Void))
                }
            }
            H::Return(value) => {
                let result = returns.ok_or_else(|| {
                    error(expr.span, "return is unavailable in a parameter default")
                })?;
                if let Some(value) = value {
                    self.describe_open_expression(value, locals, returns, Some(result), depth + 1)?;
                } else {
                    self.require_description(&D::Known(p(P::Void)), Some(result), expr.span)?;
                }
                D::Known(p(P::Never))
            }
            H::Assumes(condition) if self.is_type_assumption(condition) => {
                let predicate = self.type_predicate(condition)?;
                let fact = crate::ptc::Formula::from_predicate(&predicate);
                self.context
                    .validate_formula(&fact, &mut crate::ptc::Budget::default())
                    .map_err(|failure| error(condition.span, failure.to_string()))?;
                self.context.assume(fact);
                D::Known(p(P::Void))
            }
            H::Loop(body) => {
                let identity = local(expr.hir_id);
                self.loops.push(identity);
                let checked = self.describe_open_block(body, locals, returns, None, depth + 1);
                self.loops.pop();
                checked?;
                D::Known(p(P::Void))
            }
            H::Assign(target, value)
                if !self.in_staged_input
                    && !self.in_required_comptime
                    && match target.kind {
                        H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                            matches!(locals.get(&local(id)), Some(D::Any))
                        }
                        _ => false,
                    } =>
            {
                let binding = match target.kind {
                    H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => local(id),
                    _ => unreachable!(),
                };
                let target_type = locals
                    .get(&binding)
                    .cloned()
                    .ok_or_else(|| error(target.span, "assignment target is outside its scope"))?;
                if self.immutable.contains(&binding) {
                    return Err(error(
                        target.span,
                        "cannot reassign a let binding; use var for whole-value assignment",
                    ));
                }
                let value = self.describe_open_expression(
                    value,
                    locals,
                    returns,
                    Some(&target_type),
                    depth + 1,
                )?;
                if matches!(&value, D::Known(ty) if is_never(ty)) {
                    value
                } else {
                    D::Known(p(P::Void))
                }
            }
            H::Let(binding) => {
                let annotation = binding
                    .ty
                    .map(|ty| {
                        self.signature_annotation(ty)
                            .map(|annotation| annotation.description())
                    })
                    .transpose()?;
                let init = binding.init.ok_or_else(|| {
                    error(expr.span, "open definition binding requires an initializer")
                })?;
                let actual = self.describe_open_expression(
                    init,
                    locals,
                    returns,
                    annotation.as_ref(),
                    depth + 1,
                )?;
                let plain = matches!(
                    binding.pattern.kind,
                    hir::pattern::PatternKind::Binding(hir::common::BindingMode::ByValue, _, None)
                        | hir::pattern::PatternKind::Wild
                ) && binding.guard.is_none()
                    && binding.otherwise.is_none();
                if !plain && matches!(actual, D::Known(_)) && !matches!(annotation, Some(D::Any)) {
                    // Reuse pattern protocols, refutability, guard scope and
                    // immutable-binding checks in the ordinary checker.
                    let value = self.expr(expr, None)?;
                    self.check_open_fragment(&value, expr.span, false)?;
                    for id in pattern_binding_ids(&binding.pattern) {
                        if let Some(ty) = self.locals.get(&id) {
                            locals.insert(id, D::Known(ty.clone()));
                        }
                    }
                    let result = D::Known(value.ty);
                    self.require_description(&result, expected, expr.span)?;
                    return Ok(result);
                }
                if binding.guard.is_some() || binding.otherwise.is_some() {
                    return Err(error(
                        expr.span,
                        "guarded binding on Any requires established pattern evidence",
                    ));
                }
                let ty = annotation.unwrap_or(actual);
                match &binding.pattern.kind {
                    hir::pattern::PatternKind::Binding(
                        hir::common::BindingMode::ByValue,
                        _,
                        None,
                    ) => {
                        let id = local(binding.pattern.hir_id);
                        if let D::Known(ty) = &ty {
                            self.locals.insert(id, ty.clone());
                        }
                        locals.insert(id, ty);
                        if !binding.mutable {
                            self.immutable.insert(id);
                        }
                    }
                    hir::pattern::PatternKind::Wild => {}
                    _ => {
                        return Err(error(
                            binding.pattern.span,
                            "destructuring an Any definition requires established pattern evidence",
                        ));
                    }
                }
                D::Known(p(P::Void))
            }
            H::If(condition, yes, no) if self.open_condition_has_type_evidence(condition, 0) => {
                let yes = Expr {
                    kind: H::Block(yes),
                    ..expr.clone()
                };
                let mut arms = vec![hir::expr::CondictionArm {
                    hir_id: expr.hir_id,
                    cond: Some(condition),
                    body: &yes,
                    span: expr.span,
                }];
                if let Some(no) = no {
                    arms.push(hir::expr::CondictionArm {
                        hir_id: no.hir_id,
                        cond: None,
                        body: no,
                        span: no.span,
                    });
                }
                self.describe_open_when(expr, &arms, locals, returns, expected, depth + 1)?
            }
            H::If(condition, yes, no) => {
                let condition_ty =
                    self.describe_open_expression(condition, locals, returns, None, depth + 1)?;
                let D::Known(condition_ty) = condition_ty else {
                    return Err(error(
                        condition.span,
                        "Any supplies no evidence for a boolean condition",
                    ));
                };
                self.require_expression(&condition_ty, &p(P::Bool), condition.span)?;
                let yes = self.describe_open_block(
                    yes,
                    locals,
                    returns,
                    if no.is_some() { expected } else { None },
                    depth + 1,
                )?;
                let yes = if no.is_none() && !matches!(&yes, D::Known(ty) if is_never(ty)) {
                    D::Known(p(P::Void))
                } else {
                    yes
                };
                let no = if let Some(no) = no {
                    self.describe_open_expression(
                        no,
                        &mut locals.clone(),
                        returns,
                        expected,
                        depth + 1,
                    )?
                } else {
                    D::Known(p(P::Void))
                };
                match (&yes, &no) {
                    (D::Known(ty), _) if is_never(ty) => no,
                    (_, D::Known(ty)) if is_never(ty) => yes,
                    (D::Known(a), D::Known(b)) => {
                        self.require_expression(a, b, expr.span)?;
                        yes
                    }
                    _ => D::Any,
                }
            }
            // Known subtrees retain the normal checker and its full protocols.
            // Unknown locals are deliberately absent from its Term map: they
            // cannot acquire projection, arithmetic, layout or trait evidence.
            _ => self.describe_known_region(expr, expected)?,
        };
        self.require_description(&result, expected, expr.span)?;
        Ok(result)
    }

    fn open_condition_has_type_evidence(&self, condition: &Expr<'_>, depth: usize) -> bool {
        if depth >= 128 {
            return false;
        }
        match &condition.kind {
            H::Binary(BinOp::And | BinOp::Or, left, right) => {
                self.open_condition_has_type_evidence(left, depth + 1)
                    || self.open_condition_has_type_evidence(right, depth + 1)
            }
            H::Unary(UnOp::Not, value) => self.open_condition_has_type_evidence(value, depth + 1),
            _ => self.extracted_refinement(condition),
        }
    }

    fn describe_open_when(
        &mut self,
        expr: &Expr<'_>,
        arms: &[hir::expr::CondictionArm<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let fallback = arms.iter().position(|arm| arm.cond.is_none());
        if fallback.is_some_and(|index| index + 1 != arms.len()) {
            return Err(error(expr.span, "when fallback must be the final arm"));
        }
        let saved_context = self.context.clone();
        let saved_locals = self.locals.clone();
        let outcome = (|| {
            let mut branch_type = if fallback.is_some() {
                expected.cloned()
            } else {
                None
            };
            let mut branches = Vec::new();
            for arm in arms {
                self.locals = saved_locals.clone();
                let condition = arm
                    .cond
                    .map(|condition| {
                        let checked = self.expr(condition, Some(&p(P::Bool)))?;
                        self.check_open_fragment(&checked, condition.span, false)?;
                        Ok::<_, Error>(checked)
                    })
                    .transpose()?;
                let mut branch_locals = locals.clone();
                let mut check = |this: &mut Self| {
                    this.describe_open_expression(
                        arm.body,
                        &mut branch_locals,
                        returns,
                        branch_type.as_ref(),
                        depth + 1,
                    )
                };
                let body = match &condition {
                    Some(condition) => {
                        self.under_type_condition(condition, true, arm.body.span, check)?
                    }
                    None => check(self)?,
                };
                if branch_type.is_none() && !matches!(&body, D::Known(ty) if is_never(ty)) {
                    branch_type = Some(body.clone());
                }
                if let Some(condition) = &condition {
                    self.assume_type_condition(condition, false, arm.body.span)?;
                }
                let body = if fallback.is_none() && !matches!(&body, D::Known(ty) if is_never(ty)) {
                    D::Known(p(P::Void))
                } else {
                    body
                };
                branches.push((condition, body));
            }
            let mut result = D::Known(p(P::Void));
            for (condition, mut body) in branches.into_iter().rev() {
                if fallback.is_some() && matches!(&body, D::Known(ty) if !is_never(ty)) {
                    if let Some(D::Known(ty)) = &branch_type {
                        body = D::Known(ty.clone());
                    }
                }
                result = match condition {
                    None => body,
                    Some(condition) if is_never(&condition.ty) => D::Known(p(P::Never)),
                    Some(_) => match (&body, &result) {
                        (D::Known(ty), _) if is_never(ty) => result,
                        (_, D::Known(ty)) if is_never(ty) => body,
                        (D::Known(a), D::Known(b)) => {
                            self.require_expression(a, b, expr.span)?;
                            body
                        }
                        _ => D::Any,
                    },
                };
            }
            Ok(result)
        })();
        self.context = saved_context;
        self.locals = saved_locals;
        outcome
    }

    fn describe_open_match(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let value = self.expr(value, None).map_err(|mut failure| {
            if failure.message == "local binding is unavailable in this scope" {
                failure.message = "matching Any requires definition-side pattern evidence".into();
            }
            failure
        })?;
        let saved = self.locals.clone();
        let outcome = (|| {
            let mut result = D::Known(p(P::Never));
            let mut patterns = Vec::new();
            for arm in arms {
                self.locals = saved.clone();
                let pattern = self.match_arm_pattern(arm, &value)?;
                let mut branch_locals = locals.clone();
                for (binding, ty) in &self.locals {
                    branch_locals.insert(*binding, D::Known(ty.clone()));
                }
                let context = expected.or_else(|| match &result {
                    D::Known(ty) if is_never(ty) => None,
                    _ => Some(&result),
                });
                let branch =
                    self.under_matched_pattern_type_facts(&pattern, &value, arm.span, |this| {
                        this.describe_open_expression(
                            arm.body,
                            &mut branch_locals,
                            returns,
                            context,
                            depth + 1,
                        )
                    })?;
                result = match (&result, &branch) {
                    (D::Known(ty), _) if is_never(ty) => branch,
                    (_, D::Known(ty)) if is_never(ty) => result,
                    (D::Known(a), D::Known(b)) => {
                        self.require_expression(b, a, arm.span)?;
                        result
                    }
                    _ => D::Any,
                };
                patterns.push(pattern);
            }
            let coverage =
                self.patterns_exhaustive(&patterns.iter().collect::<Vec<_>>(), &value.ty);
            if coverage != crate::coverage::Coverage::Exhaustive {
                return Err(error(
                    expr.span,
                    if coverage == crate::coverage::Coverage::Unknown {
                        "match coverage budget exhausted; add an unguarded fallback pattern"
                    } else {
                        "non-exhaustive match; add an unguarded fallback pattern"
                    },
                ));
            }
            // Validate the typed selector/pattern program separately from the
            // described bodies. Unit arms discard matches; no Any Type is made.
            let probe = n::Expr {
                source: source(expr.span),
                ty: p(P::Void),
                kind: K::Match {
                    value: Box::new(value.clone()),
                    arms: patterns
                        .into_iter()
                        .map(|pattern| n::MatchArm {
                            pattern,
                            guard: None,
                            body: n::Expr {
                                source: source(expr.span),
                                ty: p(P::Void),
                                kind: K::Unit,
                            },
                        })
                        .collect(),
                },
            };
            self.check_open_fragment(&probe, expr.span, false)?;
            Ok(if is_never(&value.ty) {
                D::Known(p(P::Never))
            } else {
                result
            })
        })();
        self.locals = saved;
        outcome
    }

    fn describe_known_region(&mut self, expr: &Expr<'_>, expected: Option<&D>) -> Result<D, Error> {
        let expected = match expected {
            Some(D::Known(ty)) => Some(ty),
            _ => None,
        };
        let value = self.expr(expr, expected).map_err(|mut failure| {
            if failure.message == "local binding is unavailable in this scope" {
                failure.message =
                    "operation on Any requires definition-side type or trait evidence".into();
            }
            failure
        })?;
        self.check_open_fragment(&value, expr.span, false)?;
        Ok(D::Known(value.ty))
    }

    fn describe_open_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let H::ResolvedDef { def_id, .. } = callee.kind else {
            return self.describe_known_region(expr, expected);
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .ok_or_else(|| error(callee.span, "missing callable declaration"))?;
        self.ensure_signature(owner, callee.span)?;
        self.ensure_contract(owner, callee.span)?;
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Err(error(
                callee.span,
                "ordinary deferred call requires a function declaration",
            ));
        };
        if sig.modifiers.is_comptime != self.in_staged_input {
            return Err(error(
                callee.span,
                "function is unavailable in this execution phase",
            ));
        }
        if !self.type_parameters[&owner].is_empty() {
            return self.describe_known_region(expr, expected);
        }
        let (parameters, result) = if let Some(definition) = self.open_functions.get(&owner) {
            (
                definition
                    .parameters
                    .iter()
                    .map(|(sort, annotation)| (*sort, annotation.description()))
                    .collect::<Vec<_>>(),
                definition.result.description(),
            )
        } else {
            let signature = self
                .signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
                .ok_or_else(|| error(callee.span, "missing callable signature"))?;
            let Term::Function { parameters, result } = signature.callable_signature() else {
                unreachable!()
            };
            (
                parameters
                    .iter()
                    .map(|parameter| (parameter.sort, D::Known(parameter.ty.clone())))
                    .collect(),
                D::Known((**result).clone()),
            )
        };
        let (routes, _) = route_arguments(args, Some(sig.params), sig.params.len(), expr.span)?;
        let mut symbols = vec![None; parameters.len()];
        let mut needed = BTreeSet::new();
        for requirement in &self.open_definition_obligations(owner) {
            for term in required::requirement_terms(requirement, expr.span)? {
                needed.extend(
                    term.substitution_parameters()
                        .map_err(|failure| error(expr.span, failure.to_string()))?
                        .keys()
                        .copied(),
                );
            }
        }
        for description in parameters
            .iter()
            .map(|(_, description)| description)
            .chain(std::iter::once(&result))
        {
            if let D::Known(ty) = description {
                needed.extend(
                    ty.substitution_parameters()
                        .map_err(|failure| error(expr.span, failure.to_string()))?
                        .keys()
                        .copied(),
                );
            }
        }
        for slot in needed {
            self.open_contract_argument(owner, slot, &parameters, &routes, expr, &mut symbols, 0)?;
        }
        // Known descriptions belong to the callee binder. Transport them before
        // comparing caller operands; coincident indices are not type evidence.
        for route in &routes {
            let (sort, ty) = &parameters[route.parameter];
            let expected = substitute_description(ty, &symbols, expr.span)?;
            let input = route.expression(expr);
            self.with_staged_input(self.in_staged_input || *sort != Sort::Runtime, |this| {
                this.describe_open_expression(&input, locals, returns, Some(&expected), depth + 1)
            })?;
        }
        self.admit_open_type_requirements(owner, &symbols, expr.span)?;
        substitute_description(&result, &symbols, expr.span)
    }

    // Resolve only inputs demanded by the type contract. An explicitly supplied
    // argument suppresses its default, including any nonterminating computation.
    fn open_contract_argument(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[(Sort, D)],
        routes: &[arguments::RoutedArgument<'_>],
        call: &Expr<'_>,
        symbols: &mut [Option<Term>],
        depth: usize,
    ) -> Result<(), Error> {
        if symbols.get(slot).is_some_and(Option::is_some) {
            return Ok(());
        }
        if depth >= 128 {
            return Err(error(
                call.span,
                "open contract input dependency depth exhausted",
            ));
        }
        let Some((sort, D::Known(domain))) = parameters.get(slot) else {
            return Err(error(call.span, "type contract input has no known domain"));
        };
        if *sort == Sort::Runtime {
            return Err(error(
                call.span,
                "type contract cannot read a runtime argument",
            ));
        }
        if let Some(route) = routes.iter().find(|route| route.parameter == slot) {
            for dependency in domain
                .substitution_parameters()
                .map_err(|failure| error(call.span, failure.to_string()))?
                .keys()
            {
                if *dependency >= slot {
                    return Err(error(
                        call.span,
                        "staged input domain depends on a later parameter",
                    ));
                }
                self.open_contract_argument(
                    owner,
                    *dependency,
                    parameters,
                    routes,
                    call,
                    symbols,
                    depth + 1,
                )?;
            }
            let domain = domain
                .substitute_partial(symbols)
                .map_err(|failure| error(call.span, failure.to_string()))?;
            symbols[slot] = Some(self.normal_value_argument(&route.expression(call), &domain)?);
            return Ok(());
        }
        let term = self.in_open_function(owner, |this| {
            let ItemKind::Fn(sig, _) = &this.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                return Err(error(call.span, "missing open contract argument"));
            };
            this.in_staged_input = true;
            for (parameter, (_, description)) in sig.params.iter().zip(parameters).take(slot) {
                if let D::Known(ty) = description {
                    this.locals.insert(local(parameter.hir_id), ty.clone());
                }
            }
            this.normal_value_argument(default, domain)
        })?;
        for dependency in term
            .substitution_parameters()
            .map_err(|failure| error(call.span, failure.to_string()))?
            .keys()
        {
            if *dependency >= slot {
                return Err(error(
                    call.span,
                    "default type contract input depends on a later parameter",
                ));
            }
            self.open_contract_argument(
                owner,
                *dependency,
                parameters,
                routes,
                call,
                symbols,
                depth + 1,
            )?;
        }
        symbols[slot] = Some(
            term.substitute_partial(symbols)
                .map_err(|failure| error(call.span, failure.to_string()))?,
        );
        Ok(())
    }

    fn open_definition_obligations(&self, owner: OwnerId) -> Vec<n::TypePredicate> {
        let mut obligations = self.definition_requirements(owner);
        obligations.extend(
            self.capabilities
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .map(n::TypePredicate::Trait),
        );
        obligations
    }

    fn specialized_open_capabilities(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Vec<n::TraitRequirement>, Error> {
        self.capabilities
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|requirement| {
                Ok(n::TraitRequirement {
                    interface: requirement.interface,
                    subject: requirement
                        .subject
                        .substitute_partial(symbols)
                        .map_err(|failure| error(span, failure.to_string()))?,
                    interface_arguments: requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| {
                            ty.substitute_partial(symbols)
                                .map_err(|failure| error(span, failure.to_string()))
                        })
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect()
    }

    fn specialized_open_requirements(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Vec<n::TypePredicate>, Error> {
        self.definition_requirements(owner)
            .iter()
            .map(|requirement| {
                required::map_requirement(
                    requirement,
                    &mut |ty| {
                        ty.substitute_partial(symbols).map_err(|failure| error(span,
                    format!("open type contract requires available comptime inputs: {failure}")))
                    },
                    span,
                    0,
                )
            })
            .collect()
    }

    fn admit_open_type_requirements(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<(), Error> {
        let mut obligations = self.specialized_open_requirements(owner, symbols, span)?;
        obligations.extend(
            self.specialized_open_capabilities(owner, symbols, span)?
                .into_iter()
                .map(n::TypePredicate::Trait),
        );
        for requirement in obligations {
            let mut outcome = self.context.prove(
                crate::ptc::Formula::from_predicate(&requirement),
                crate::ptc::Budget::default(),
            );
            if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
                let assumptions = self.capability_assumptions();
                outcome = crate::capabilities::World {
                    index: &self.implementation_index,
                    implementations: &self.checked_implementations,
                    traits: &self.checked_traits,
                    assumptions: &assumptions,
                    context: &self.context,
                }
                .check_predicate(&requirement, crate::ptc::Budget::default())
                .map_err(|message| error(span, message))?;
            }
            if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
                return Err(error(
                    span,
                    format!(
                        "open function type requirement is not established: {requirement}: {outcome}"
                    ),
                ));
            }
        }
        Ok(())
    }

    fn instantiate_open_annotation(
        &mut self,
        annotation: &Annotation,
        sig: &hir::item::FnSig<'_>,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Term, Error> {
        let ty = match annotation {
            Annotation::Known(ty) => ty.clone(),
            Annotation::Pending(computation) => {
                let mut values = Vec::new();
                for (binding, _, value) in &computation.inputs {
                    if value.is_some() {
                        continue;
                    }
                    let slot = sig
                        .params
                        .iter()
                        .position(|parameter| local(parameter.hir_id) == *binding)
                        .ok_or_else(|| {
                            error(span, "deferred annotation has an unknown source input")
                        })?;
                    let value = symbols.get(slot).and_then(Option::as_ref).ok_or_else(|| {
                        error(
                            span,
                            "deferred annotation needs a completed comptime argument",
                        )
                    })?;
                    values.push((*binding, value.clone()));
                }
                let mut invocation = computation.fork(span)?;
                for (_, ty, _) in &mut invocation.inputs {
                    *ty = ty
                        .substitute_partial(symbols)
                        .map_err(|failure| error(span, failure.to_string()))?;
                }
                let (value, _) =
                    invocation.invoke_with_inputs(values, &mut self.required_steps, span)?;
                let comptime_interpretor::Value::Type(ty) = value else {
                    return Err(error(span, "deferred annotation did not produce a Type"));
                };
                ty
            }
        };
        ty.substitute_partial(symbols)
            .map_err(|failure| error(span, failure.to_string()))
    }

    pub(super) fn open_function_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.check_open_definition(owner)?;
        let definition = self.open_functions[&owner].clone();
        let item = self.package.item(owner).unwrap();
        let ItemKind::Fn(sig, _) = &item.kind else {
            unreachable!()
        };
        let (routes, assigned) =
            route_arguments(args, Some(sig.params), sig.params.len(), expr.span)?;
        let mut symbols = vec![None; sig.params.len()];
        let mut values = vec![None; sig.params.len()];
        let mut parameters = Vec::new();
        // Type formation is declaration ordered. Runtime evaluation remains in
        // source order in the emitted call argument plan below.
        for (slot, (sort, annotation)) in definition.parameters.iter().enumerate() {
            if annotation.is_pending() {
                self.admit_open_type_requirements(owner, &symbols, expr.span)?;
            }
            let ty = self.instantiate_open_annotation(annotation, sig, &symbols, expr.span)?;
            parameters.push(Parameter {
                sort: *sort,
                ty: ty.clone(),
            });
            if *sort != Sort::Runtime {
                let (value, symbol) = if let Some(route) =
                    routes.iter().find(|route| route.parameter == slot)
                {
                    route.require_collection_type(self, &ty, expr.span)?;
                    let input = route.expression(expr);
                    let checked =
                        self.with_staged_input(true, |this| this.expr(&input, Some(&ty)))?;
                    self.freeze_dependency_input(&input, checked, vec![])?
                } else {
                    let (value, symbol) =
                        self.prepare_open_default(owner, slot, &parameters, &symbols, expr.span)?;
                    (
                        value,
                        symbol.expect("staged default produces an immutable symbol"),
                    )
                };
                symbol.check_scope(&[]).map_err(|failure| error(expr.span,
                    format!("open signature invocation requires completed comptime inputs: {failure}")))?;
                symbols[slot] = Some(symbol);
                values[slot] = Some(value);
            }
        }
        self.admit_open_type_requirements(owner, &symbols, expr.span)?;
        let result =
            self.instantiate_open_annotation(&definition.result, sig, &symbols, expr.span)?;
        let signature = Term::Function {
            parameters: parameters.clone(),
            result: Box::new(result.clone()),
        };
        let mut arguments = Vec::new();
        for route in &routes {
            let slot = route.parameter;
            route.require_collection_type(self, &parameters[slot].ty, expr.span)?;
            let value = if let Some(value) = values[slot].take() {
                value
            } else {
                let input = route.expression(expr);
                self.expr(&input, Some(&parameters[slot].ty))?
            };
            arguments.push(n::CallArgument {
                parameter: slot,
                value,
                is_default: false,
            });
        }
        for (slot, assigned) in assigned.iter().enumerate() {
            if *assigned {
                continue;
            }
            let value = if let Some(value) = values[slot].take() {
                value
            } else {
                self.prepare_open_default(owner, slot, &parameters, &symbols, expr.span)?
                    .0
            };
            arguments.push(n::CallArgument {
                parameter: slot,
                value,
                is_default: true,
            });
        }
        let result = if arguments
            .iter()
            .any(|argument| is_never(&argument.value.ty))
        {
            p(P::Never)
        } else {
            result
        };
        if let Some(expected) = expected {
            self.require_expression(&result, expected, expr.span)?;
        }
        let id = self.instantiate_open_function(owner, &symbols, signature.clone(), expr.span)?;
        let call = n::Expr {
            source: source(expr.span),
            ty: result,
            kind: K::Call {
                callee: Box::new(n::Expr {
                    source: source(expr.span),
                    ty: signature,
                    kind: K::Function(id),
                }),
                arguments,
                parameter_bindings: sig
                    .params
                    .iter()
                    .map(|parameter| local(parameter.hir_id))
                    .collect(),
            },
        };
        self.stage_collected_call(call, &routes, 0, expr.span)
    }

    fn require_stable_open_default(&mut self, value: &n::Expr, span: Span) -> Result<(), Error> {
        if !stable_default(value) && !self.stable_default_under_contract(value, span)? {
            return Err(error(
                span,
                "default expression requires P3 stability/termination evidence not yet implemented",
            ));
        }
        Ok(())
    }

    fn prepare_open_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[Parameter],
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<(n::Expr, Option<Term>), Error> {
        self.in_open_function(owner, |this| {
            let ItemKind::Fn(sig, _) = &this.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                return Err(error(
                    span,
                    "missing default for an unsupplied open parameter",
                ));
            };
            this.default_input_scope = Some(parameters.to_vec());
            for (index, parameter) in sig.params.iter().enumerate().take(slot) {
                let binding = local(parameter.hir_id);
                let domain = &parameters[index];
                this.locals.insert(binding, domain.ty.clone());
                if let Some(value) = &symbols[index] {
                    this.specialization_inputs
                        .push((binding, domain.ty.clone(), value.clone()));
                    if domain.sort == Sort::Type {
                        this.type_input_bindings.insert(binding, value.clone());
                    } else {
                        this.value_input_bindings
                            .insert(binding, (value.clone(), domain.ty.clone()));
                    }
                }
            }
            let staged = parameters[slot].sort != Sort::Runtime;
            this.in_staged_input = staged;
            let value = this.expr(default, Some(&parameters[slot].ty))?;
            if staged {
                let (value, symbol) = this.freeze_dependency_input(default, value, vec![])?;
                Ok((value, Some(symbol)))
            } else {
                this.require_stable_open_default(&value, default.span)?;
                Ok((value, None))
            }
        })
    }

    fn instantiate_open_function(
        &mut self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        signature: Term,
        span: Span,
    ) -> Result<DeclarationId, Error> {
        let key = (owner, symbols.iter().flatten().cloned().collect::<Vec<_>>());
        if let Some(id) = self.open_instances.get(&key) {
            return Ok(*id);
        }
        if self.open_instantiation_depth >= 64 {
            return Err(error(span, "open function instantiation depth exhausted"));
        }
        let serial = self.next_open_instance;
        self.next_open_instance = serial
            .checked_add(1)
            .ok_or_else(|| error(span, "open function instance limit exceeded"))?;
        let id = DeclarationId {
            package: u64::MAX - 1,
            local: serial,
        };
        let saved_instances = self.open_instances.clone();
        let saved_generated = self.generated_functions.len();
        self.open_instances.insert(key.clone(), id);
        self.open_instantiation_depth += 1;
        let saved_signature = self.signatures.insert(owner, signature.clone());
        let function = self.in_open_function(owner, |this| {
            let item = this.package.item(owner).unwrap();
            let ItemKind::Fn(sig, body) = &item.kind else {
                unreachable!()
            };
            let Term::Function { parameters, result } = &signature else {
                unreachable!()
            };
            for (slot, parameter) in sig.params.iter().enumerate() {
                let binding = local(parameter.hir_id);
                let ty = &parameters[slot].ty;
                this.locals.insert(binding, ty.clone());
                if let Some(value) = &symbols[slot] {
                    this.specialization_inputs
                        .push((binding, ty.clone(), value.clone()));
                    if parameters[slot].sort == Sort::Type {
                        this.type_input_bindings.insert(binding, value.clone());
                    } else {
                        this.value_input_bindings
                            .insert(binding, (value.clone(), ty.clone()));
                    }
                }
            }
            this.result = Some((**result).clone());
            let preconditions = this.program_preconditions(owner)?;
            let result_binding = sig.return_binding.map(local);
            let (postconditions, entry_snapshots) =
                this.program_postconditions(owner, result_binding, result)?;
            let body = this
                .package
                .body(*body)
                .ok_or_else(|| error(span, "missing open function body"))?;
            let value = this.computation_body(body.value, Some(result))?;
            Ok(n::Function {
                declaration: id,
                name: item.ident.name.as_str().into(),
                phase: n::FunctionPhase::Runtime,
                inline: sig.modifiers.is_inline,
                pure: sig.modifiers.is_pure,
                signature: signature.clone(),
                parameters: sig
                    .params
                    .iter()
                    .map(|parameter| local(parameter.hir_id))
                    .collect(),
                body: value,
                preconditions,
                postconditions,
                entry_snapshots,
                result_binding,
                type_parameters: vec![],
                requirements: this.specialized_open_requirements(owner, symbols, span)?,
                capabilities: this.specialized_open_capabilities(owner, symbols, span)?,
                defaults: vec![],
            })
        });
        match saved_signature {
            Some(signature) => {
                self.signatures.insert(owner, signature);
            }
            None => {
                self.signatures.remove(&owner);
            }
        }
        self.open_instantiation_depth -= 1;
        match function {
            Ok(function) => {
                self.generated_functions.push(function);
                Ok(id)
            }
            Err(failure) => {
                self.open_instances = saved_instances;
                self.generated_functions.truncate(saved_generated);
                Err(failure)
            }
        }
    }
}

fn open_contract_error(mut failure: Error) -> Error {
    if failure.message == "local binding is unavailable in this scope" {
        failure.message =
            "contract operation on Any requires definition-side type or trait evidence".into();
    }
    failure
}

fn substitute_description(
    description: &D,
    symbols: &[Option<Term>],
    span: Span,
) -> Result<D, Error> {
    match description {
        D::Any => Ok(D::Any),
        D::Known(ty) => ty
            .substitute_partial(symbols)
            .map(D::Known)
            .map_err(|failure| {
                error(
                    span,
                    format!("dependent call description requires available inputs: {failure}"),
                )
            }),
    }
}

fn source_always_exits(expr: &Expr<'_>, depth: usize) -> bool {
    if depth >= 128 {
        return false;
    }
    match &expr.kind {
        H::Return(_) | H::Break(_) | H::BreakTo(_) | H::Continue(_) | H::ContinueTo(_) => true,
        H::Semi(value) => source_always_exits(value, depth + 1),
        H::Block(block) => block
            .stmts
            .iter()
            .chain(block.expr)
            .any(|value| source_always_exits(value, depth + 1)),
        H::If(_, yes, Some(no)) => {
            yes.stmts
                .iter()
                .chain(yes.expr)
                .any(|value| source_always_exits(value, depth + 1))
                && source_always_exits(no, depth + 1)
        }
        _ => false,
    }
}
