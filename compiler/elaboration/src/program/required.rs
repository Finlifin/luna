//! Preparation and invocation of required computations during type formation.
//! Open signature captures retain unavailable staged inputs explicitly. Completed
//! invocation uses ordinary NIR comptime semantics without unfolding type heads.
use super::*;
use comptime_interpretor::{Budget, Interpreter, Value};
use std::collections::VecDeque;
pub(super) mod prepared;
use prepared::PreparedRequired;

impl Checker<'_, '_> {
    // A comptime parameter may acquire a Type domain from an explicit type
    // equality in this declaration. Only already available type binders and
    // positive equality conjuncts participate; program predicates never do.
    pub(super) fn signature_parameter_domain(
        &mut self,
        ty: Term,
        clauses: &[hir::clause::ClauseConstraint<'_>],
        span: Span,
    ) -> Result<Term, Error> {
        if ty == p(P::Type) {
            return Ok(ty);
        }
        fn collect<'h>(
            value: &Expr<'h>,
            out: &mut Vec<Expr<'h>>,
            depth: usize,
        ) -> Result<(), Error> {
            if depth >= 128 {
                return Err(error(
                    value.span,
                    "signature domain constraint depth exhausted",
                ));
            }
            match value.kind {
                H::Binary(BinOp::And, a, b) => {
                    collect(a, out, depth + 1)?;
                    collect(b, out, depth + 1)?;
                }
                H::Binary(BinOp::Eq, _, _) => out.push(value.clone()),
                _ => {}
            }
            Ok(())
        }
        let mut atoms = Vec::new();
        for clause in clauses {
            if let hir::clause::ClauseConstraintKind::Requires(value) = clause.kind {
                collect(value, &mut atoms, 0)?;
            }
        }
        let scope = self.call_inference_scope();
        let mut context = self
            .context
            .theory_in_scope(vec![scope.iter().map(|p| p.sort).collect()]);
        for atom in atoms {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "signature domain constraint budget exhausted"))?;
            if !signature_inputs_ready(&atom, 0, &|id| {
                self.type_input_bindings.contains_key(&local(id))
            })? || mentions_value_parameter(&atom, 0, &|id| {
                self.type_input_bindings.contains_key(&local(id))
            })? {
                continue;
            }
            context.assume(crate::ptc::Formula::from_predicate(
                &self.type_predicate(&atom)?,
            ));
        }
        if matches!(
            context.prove(
                crate::ptc::Formula::equal(ty.clone(), p(P::Type)),
                crate::ptc::Budget::default()
            ),
            crate::ptc::Outcome::Entailed(_)
        ) {
            Ok(p(P::Type))
        } else {
            Ok(ty)
        }
    }

    pub(super) fn ensure_contract(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        if self.requirements.contains_key(&owner) {
            return Ok(());
        }
        if self.active_contracts.len() >= 64 || !self.active_contracts.insert(owner) {
            return Err(error(
                span,
                "cyclic or excessively deep declaration contract dependency",
            ));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_checking = std::mem::replace(&mut self.checking_contract, true);
        let saved_pending = self.pending_type_uses.replace(Vec::new());
        let saved_capabilities = self.capabilities.get(&owner).cloned();
        self.context = self.binder_context(owner);
        let result = (|| {
            let clauses = match &self.package.item(owner).unwrap().kind {
                ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) | ItemKind::Effect(sig, _) => sig.clause_constraints,
                ItemKind::EffectGroup(group) => group.clause_constraints,
                ItemKind::TypeAlias(_, _, constraints) => *constraints,
                ItemKind::Struct(def) => def.clause_constraints,
                ItemKind::Enum(def) => def.clause_constraints,
                ItemKind::Impl(def) => def.clause_constraints,
                ItemKind::Trait(def) => def.clause_constraints,
                ItemKind::Associated(def) => def.clause_constraints,
                _ => return Err(error(span, "declaration does not have a type contract")),
            };
            let mut requirements = if let Some(parent) = self.effect_parents.get(&owner).copied() {
                self.ensure_contract(parent, span)?;
                self.requirements[&parent].clone()
            } else { Vec::new() };
            // Admission uses the same prerequisite discipline as signatures.
            // Only positive type atoms may become declaration assumptions.
            let mut pending_capabilities = Vec::new();
            for clause in clauses {
                if let hir::clause::ClauseConstraintKind::Requires(value) = &clause.kind {
                    if !mentions_value_parameter(value, 0, &|id| self.type_input_bindings.contains_key(&local(id)))? {
                        signature_capability_atoms(value, &mut pending_capabilities, 0)?;
                    }
                }
            }
            self.ready_signature_capabilities(owner, &mut pending_capabilities, true)?;
            let mut program_requirements = Vec::new();
            let mut postconditions = Vec::new();
            for clause in clauses {
                if let hir::clause::ClauseConstraintKind::Ensures(value) = &clause.kind {
                    if matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if !sig.modifiers.is_normal_form_head)
                    {
                        postconditions.push(*value);
                        continue;
                    }
                }
                let hir::clause::ClauseConstraintKind::Requires(value) = &clause.kind else {
                    return Err(error(
                        clause.span,
                        "program specification elaboration is not implemented",
                    ));
                };
                if mentions_value_parameter(value, 0, &|id| self.type_input_bindings.contains_key(&local(id)))? {
                    if !matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if !sig.modifiers.is_normal_form_head)
                    {
                        return Err(error(
                            clause.span,
                            "program specifications are not handled by PTC; this declaration cannot yet carry program preconditions",
                        ));
                    }
                    program_requirements.push(*value);
                } else {
                    self.positive_type_requirements(owner, value, &mut requirements, 0)?;
                }
            }
            self.program_requirements
                .insert(owner, program_requirements);
            self.program_postconditions.insert(owner, postconditions);
            Ok(requirements)
        })();
        let uses = std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.checking_contract = saved_checking;
        self.active_contracts.remove(&owner);
        if result.is_err() {
            match saved_capabilities {
                Some(capabilities) => { self.capabilities.insert(owner, capabilities); }
                None => { self.capabilities.remove(&owner); }
            }
        }
        let requirements = result?;
        self.signature_type_uses.extend(uses.iter().cloned());
        self.contract_uses.insert(owner, uses);
        if let Some(def) =
            self.structs
                .get_mut(&declaration(self.package, &self.package_identities, owner))
        {
            def.requirements = requirements.clone();
            def.capabilities = self.capabilities.get(&owner).cloned().unwrap_or_default();
        }
        if let Some(def) =
            self.enums
                .get_mut(&declaration(self.package, &self.package_identities, owner))
        {
            def.requirements = requirements.clone();
            def.capabilities = self.capabilities.get(&owner).cloned().unwrap_or_default();
        }
        self.requirements.insert(owner, requirements);
        Ok(())
    }

    pub(super) fn prepare_parameter_capabilities(&mut self, owner: OwnerId, parameters: &[hir::clause::ClauseParam<'_>]) -> Result<(), Error> {
        let subjects = parameter_capability_subjects(parameters)?;
        if subjects.is_empty() { return Ok(()); }
        let mut pending = Vec::new();
        for (subject, bound) in &subjects {
            let atom = Expr { kind: H::TraitBound(subject, bound), ..subject.clone() };
            signature_capability_atoms(&atom, &mut pending, 0)?;
        }
        let constraints = match &self.package.item(owner).unwrap().kind {
            ItemKind::TypeAlias(_, _, constraints) => *constraints,
            ItemKind::Struct(def) => def.clause_constraints,
            ItemKind::Enum(def) => def.clause_constraints,
            ItemKind::Impl(def) => def.clause_constraints,
            ItemKind::Trait(def) => def.clause_constraints,
            ItemKind::EffectGroup(group) => group.clause_constraints,
            _ => &[],
        };
        for constraint in constraints {
            if let hir::clause::ClauseConstraintKind::Requires(value) = &constraint.kind {
                signature_capability_atoms(value, &mut pending, 0)?;
            }
        }
        self.ready_signature_capabilities(owner, &mut pending, true)
    }

    // Positive capability conjuncts share the existing trait evidence path.
    // Never extract an atom from beneath not/or: that would strengthen the
    // declaration or turn failure to select into negative evidence.
    /// Publish each ready positive bound once, keeping later inputs pending.
    pub(super) fn ready_signature_capabilities<'h>(
        &mut self, owner: OwnerId, pending: &mut Vec<Expr<'h>>, complete: bool,
    ) -> Result<(), Error> {
        loop {
            let mut progress = false;
            let mut waiting = None;
            for value in std::mem::take(pending) {
                self.required_steps = self.required_steps.checked_sub(1)
                    .ok_or_else(|| error(value.span, "signature capability preparation budget exhausted"))?;
                let ready = complete || signature_inputs_ready(&value, 0, &|id| {
                    self.type_input_bindings.contains_key(&local(id))
                        || self.value_input_bindings.contains_key(&local(id))
                })?;
                if !ready { pending.push(value); continue; }
                // A failed atom must not publish partial assumptions or uses.
                // Completed declaration caches remain reusable across retries.
                let capabilities = self.capabilities.get(&owner).cloned();
                let context = self.context.clone();
                let pending_len = self.pending_type_uses.as_ref().map(Vec::len);
                match self.positive_type_requirements(owner, &value, &mut Vec::new(), 0) {
                    Ok(()) => progress = true,
                    Err(failure) => {
                        self.context = context;
                        match capabilities {
                            Some(capabilities) => { self.capabilities.insert(owner, capabilities); }
                            None => { self.capabilities.remove(&owner); }
                        }
                        if let (Some(uses), Some(len)) = (&mut self.pending_type_uses, pending_len) {
                            uses.truncate(len);
                        }
                        if !failure.needs_capability { return Err(failure); }
                        waiting.get_or_insert(failure);
                        pending.push(value);
                    }
                }
            }
            if pending.is_empty() { break; }
            if !progress {
                if complete {
                    if let Some(failure) = waiting { return Err(failure); }
                }
                break;
            }
        }
        Ok(())
    }

    fn positive_type_requirements(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        requirements: &mut Vec<n::TypePredicate>,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= 128 {
            return Err(error(value.span, "type requirement depth exhausted"));
        }
        match value.kind {
            H::Binary(BinOp::And, left, right) => {
                self.positive_type_requirements(owner, left, requirements, depth + 1)?;
                self.positive_type_requirements(owner, right, requirements, depth + 1)
            }
            H::TraitBound(subject, bound) => {
                let subject = self.type_expr(subject)?;
                for interface in self.parent_interfaces_for(bound, &subject)? {
                    let requirement = n::TraitRequirement {
                        interface: interface.declaration,
                        interface_arguments: interface.arguments,
                        subject: subject.clone(),
                    };
                    let capabilities = self.capabilities.entry(owner).or_default();
                    if !capabilities.contains(&requirement) {
                        capabilities.push(requirement);
                    }
                }
                Ok(())
            }
            _ => {
                requirements.push(self.type_predicate(value)?);
                Ok(())
            }
        }
    }

    // The predicate consumer needs the proved Boolean view, while the binding
    // and function signature retain their original dependent type identities.
    pub(super) fn boolean_predicate(&mut self, value: &Expr<'_>) -> Result<n::Expr, Error> {
        let mut expression = self.expr(value, Some(&p(P::Bool)))?;
        if !is_never(&expression.ty) {
            self.require(&expression.ty, &p(P::Bool), value.span)?;
            expression.ty = p(P::Bool);
        }
        Ok(expression)
    }

    pub(super) fn program_preconditions(&mut self, owner: OwnerId) -> Result<Vec<n::Expr>, Error> {
        let values = self
            .program_requirements
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        let mut result = Vec::new();
        for value in values {
            let expression = self.boolean_predicate(value)?;
            if !n::checkable_predicate(&expression) {
                return Err(error(
                    value.span,
                    "requires needs a supported effect-free program predicate",
                ));
            }
            result.push(expression);
        }
        Ok(result)
    }

    pub(super) fn program_postconditions(
        &mut self,
        owner: OwnerId,
        binding: Option<n::LocalId>,
        ty: &Term,
    ) -> Result<(Vec<n::Expr>, Vec<n::EntrySnapshot>), Error> {
        self.program_postconditions_with_result(owner, binding.map(|binding| (binding, ty)))
    }

    /// Ordinary open definitions may not know their result Type. An absent
    /// binding supplies no result evidence, while predicates over known inputs
    /// can still be checked without inventing a placeholder Type.
    pub(super) fn program_postconditions_with_result(
        &mut self, owner: OwnerId, result: Option<(n::LocalId, &Term)>,
    ) -> Result<(Vec<n::Expr>, Vec<n::EntrySnapshot>), Error> {
        let result=result.map(|(binding,ty)| {
            let ty=if let Term::Computation {result,..}=ty {result.as_ref()} else {ty};
            (binding,ty)
        });
        let old_scope = self.old_scope.replace(OldScope {
            owner,
            active: false,
            bindings: BTreeMap::new(),
        });
        let binding = result.map(|(binding, _)| binding);
        let previous = result.and_then(|(binding, ty)| self.locals.insert(binding, ty.clone()));
        let values = self
            .program_postconditions
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        let result: Result<Vec<n::Expr>, Error> = values
            .into_iter()
            .map(|value| {
                let expr = self.boolean_predicate(value)?;
                if !n::checkable_predicate(&expr) {
                    return Err(error(
                        value.span,
                        "ensures needs a supported effect-free program predicate",
                    ));
                }
                Ok(expr)
            })
            .collect();
        if let Some(binding) = binding {
            if let Some(previous) = previous {
                self.locals.insert(binding, previous);
            } else {
                self.locals.remove(&binding);
            }
        }
        let scope = std::mem::replace(&mut self.old_scope, old_scope).unwrap();
        let snapshots = scope
            .bindings
            .into_iter()
            .map(|(parameter, binding)| {
                self.locals.remove(&binding);
                n::EntrySnapshot { parameter, binding }
            })
            .collect();
        result.map(|predicates| (predicates, snapshots))
    }

    pub(super) fn copy_alias_type(&mut self, ty: &Term, span: Span) -> Result<Term, Error> {
        let mut budget = comptime_runtime::Budget {
            steps: self.required_steps as usize,
            max_limbs: 4096,
        };
        let start = budget.steps;
        let result = super::normal::clone_known_term(ty, &mut budget);
        self.required_steps -= (start - budget.steps) as u64;
        result.ok_or_else(|| error(span, "type alias expansion budget exhausted"))
    }

    pub(super) fn check_alias_definition(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        span: Span,
    ) -> Result<(), Error> {
        if self.checked_alias_definitions.contains(&owner) {
            return Ok(());
        }
        if self.aliases.len() >= 64 || !self.aliases.insert(owner) {
            return Err(error(
                span,
                "cyclic type alias or excessively deep expansion",
            ));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_substitution = std::mem::take(&mut self.alias_substitution);
        let saved_value_bindings = std::mem::take(&mut self.alias_value_bindings);
        let saved_scopes = std::mem::take(&mut self.alias_scopes);
        self.context = self.binder_context(owner);
        // Normalize/check the target as a Type-valued expression without
        // demanding concrete results of calls whose inputs are still symbolic.
        let result = self.expr(value, Some(&p(P::Type))).map(|_| ());
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.in_required_comptime = saved_mode;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_value_bindings;
        self.alias_scopes = saved_scopes;
        self.aliases.remove(&owner);
        if result.is_ok() {
            self.checked_alias_definitions.insert(owner);
        }
        result
    }

    pub(super) fn alias_type(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: &[Term],
        span: Span,
    ) -> Result<Term, Error> {
        let parameters = self
            .type_parameters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        if parameters.len() != arguments.len() {
            return Err(error(span, "type alias argument count mismatch"));
        }
        // Bound work before copying cache keys or complete expansion results.
        let arguments = arguments
            .iter()
            .map(|ty| self.copy_alias_type(ty, span))
            .collect::<Result<Vec<_>, _>>()?;
        let key = (owner, arguments.clone());
        if let Some(ty) = self.completed_aliases.get(&key).cloned() {
            return self.copy_alias_type(&ty, span);
        }
        let result = self.with_alias_arguments(owner, &parameters, &arguments, span, |checker| {
            checker.type_expr(value)
        });
        if let Ok(ty) = &result {
            let copy = self.copy_alias_type(ty, span)?;
            self.completed_aliases
                .insert(key, std::sync::Arc::new(copy));
        }
        result
    }

    // Keep a Type-valued computation residual while checking another alias.
    // This is transparent substitution, not a new nominal or normal-form head.
    pub(super) fn alias_expression(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: &[Term],
        span: Span,
    ) -> Result<n::Expr, Error> {
        let parameters = self.type_parameters[&owner].clone();
        if parameters.len() != arguments.len() {
            return Err(error(span, "type alias argument count mismatch"));
        }
        let arguments = arguments
            .iter()
            .map(|ty| self.copy_alias_type(ty, span))
            .collect::<Result<Vec<_>, _>>()?;
        self.with_alias_arguments(owner, &parameters, &arguments, span, |checker| {
            checker.expr(value, Some(&p(P::Type)))
        })
    }

    pub(super) fn alias_value_expression(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: Vec<n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        self.alias_expression_budget = self
            .alias_expression_budget
            .checked_sub(1)
            .ok_or_else(|| error(span, "type alias expression expansion budget exhausted"))?;
        let parameters = self.type_parameters[&owner].clone();
        let mut terms = Vec::new();
        for (argument, parameter) in arguments.iter().zip(&parameters) {
            terms.push(if let K::TypeValue(ty) = &argument.kind {
                self.copy_alias_type(ty, span)?
            } else {
                Term::Rigid {
                    declaration: *parameter,
                    sort: Sort::Type,
                }
            });
        }
        // Alias bounds still require evidence here. This path cannot defer a
        // contract without carrying its proof/check into the generated code.
        self.check_type_application(owner, &terms, span)?;
        if arguments
            .iter()
            .all(|argument| matches!(argument.kind, K::TypeValue(_)))
        {
            return self.alias_expression(owner, value, &terms, span);
        }
        let diverges = arguments.iter().any(|argument| is_never(&argument.ty));
        let saved_bindings = self.alias_value_bindings.clone();
        let result = self.with_alias_arguments(owner, &parameters, &terms, span, |checker| {
            let mut statements = Vec::new();
            for (argument, parameter) in arguments.into_iter().zip(&parameters) {
                if matches!(argument.kind, K::TypeValue(_)) {
                    continue;
                }
                let index = checker.next_temporary_binding;
                checker.next_temporary_binding = index
                    .checked_add(1)
                    .ok_or_else(|| error(span, "type alias binding budget exhausted"))?;
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index,
                };
                checker.alias_value_bindings.insert(*parameter, binding);
                checker.locals.insert(binding, p(P::Type));
                statements.push(n::Expr {
                    source: argument.source,
                    ty: if is_never(&argument.ty) {
                        p(P::Never)
                    } else {
                        p(P::Void)
                    },
                    kind: K::Let {
                        binding,
                        value: Box::new(argument),
                    },
                });
            }
            let saved_mode = std::mem::replace(&mut checker.in_required_comptime, true);
            let body = checker.expr(value, Some(&p(P::Type)));
            checker.in_required_comptime = saved_mode;
            let body = body?;
            Ok(n::Expr {
                source: source(span),
                ty: if diverges {
                    p(P::Never)
                } else {
                    body.ty.clone()
                },
                kind: K::Block {
                    statements,
                    result: Some(Box::new(body)),
                },
            })
        });
        self.alias_value_bindings = saved_bindings;
        result
    }

    fn with_alias_arguments<R>(
        &mut self,
        owner: OwnerId,
        parameters: &[DeclarationId],
        arguments: &[Term],
        span: Span,
        expand: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        if self.aliases.len() >= 64 || !self.aliases.insert(owner) {
            return Err(error(
                span,
                "cyclic type alias or excessively deep expansion",
            ));
        }
        let generic = !parameters.is_empty();
        let saved_owner = self.signature_owner.replace(owner);
        let caller = saved_owner.and_then(|scope| {
            self.alias_scopes
                .get(&scope)
                .copied()
                .unwrap_or(Some(scope))
        });
        let previous_scope = if generic {
            self.alias_scopes.insert(owner, caller)
        } else {
            None
        };
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_substitution = self.alias_substitution.clone();
        self.alias_substitution.extend(
            parameters
                .iter()
                .copied()
                .zip(arguments.iter().cloned().map(std::sync::Arc::new)),
        );
        if !generic {
            self.context = self.binder_context(owner);
        }
        let result = expand(self);
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.alias_substitution = saved_substitution;
        if generic {
            if let Some(scope) = previous_scope {
                self.alias_scopes.insert(owner, scope);
            } else {
                self.alias_scopes.remove(&owner);
            }
        }
        self.aliases.remove(&owner);
        result
    }

    pub(super) fn required_value(&mut self, expr: &Expr<'_>, target: P) -> Result<Value, Error> {
        self.required_expression(expr, Some(&p(target)))
    }

    pub(super) fn required_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Value, Error> {
        self.required_typed_expression(expr, expected)
            .map(|(value, _)| value)
    }

    pub(super) fn required_typed_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<(Value, Term), Error> {
        self.required_expression_with_inputs(expr,expected,None,Vec::new())
    }

    pub(super) fn required_normal_input(
        &mut self, source: &Expr<'_>, root: n::Expr,
        inputs: Vec<(n::LocalId,Term,Term)>,
    ) -> Result<(Value,Term),Error> {
        self.required_expression_with_inputs(source,None,Some(root),inputs)
    }

    fn required_expression_with_inputs(
        &mut self, expr: &Expr<'_>, expected: Option<&Term>,
        checked: Option<n::Expr>, inputs: Vec<(n::LocalId,Term,Term)>,
    ) -> Result<(Value,Term),Error> {
        let prepared = self.prepare_required_expression(expr, expected, checked, inputs, false)?;
        prepared.invoke(&mut self.required_steps, expr.span)
    }

    pub(super) fn prepare_required_expression(
        &mut self, expr: &Expr<'_>, expected: Option<&Term>,
        checked: Option<n::Expr>, inputs: Vec<(n::LocalId,Term,Term)>, defer_inputs: bool,
    ) -> Result<PreparedRequired,Error> {
        if self.active_required.len() >= 32 || !self.active_required.insert(local(expr.hir_id)) {
            return Err(error(
                expr.span,
                "cyclic or excessively deep required comptime dependency",
            ));
        }
        let saved_result = self.result.take();
        let saved_partial_return = self.partial_return_context.take();
        let saved_optional_returns = std::mem::take(&mut self.pending_optional_returns);
        let saved_returns = self.return_observations.take();
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let saved_pending = self.pending_type_uses.replace(Vec::new());
        let signature_start = self.signature_type_uses.len();
        let result = (|| {
            let mut root = checked.map(Ok).unwrap_or_else(|| self.expr(expr, expected)).map_err(|mut e| {
                if e.message == "local binding is unavailable in this scope" {
                    e.message =
                        "required comptime input is not yet available in this definition".into();
                }
                e
            })?;
            if let Some((parameters,arguments))=self.initializer_substitution() {
                root=root.instantiate_types(parameters,arguments)
                    .map_err(|e|error(expr.span,e.to_string()))?;
            }
            let result_type = root.ty.clone();
            let mut inputs = inputs;
            for input in &self.specialization_inputs {
                if !inputs.iter().any(|(id, _, _)| *id == input.0) { inputs.push(input.clone()); }
            }
            let mut scoped_parameters = Vec::new();
            let mut scoped_arguments = Vec::new();
            let input_frames = self.call_input_frames();
            // A Type input can be transported without knowing its concrete
            // description. Use invocation-local rigid identities, then restore
            // the original telescope slots before returning to normalization.
            let type_packet = match &result_type {
                Term::Tuple(fields) => fields.iter().all(|ty| *ty == p(P::Type)),
                Term::NormalForm { head, .. } if Some(*head) == self.protocols.tuple =>
                    matches!(self.structural_type(&result_type, expr.span)?, Term::Tuple(fields)
                        if fields.iter().all(|ty| *ty == p(P::Type))),
                _ => false,
            };
            if !defer_inputs && (result_type == p(P::Type) || result_type == p(P::Bool) || self.is_type_list_type(&result_type) || type_packet)
                && self.signature_owner.is_some()
                && input_frames.last().is_some_and(|frame|frame.contains(&Sort::Type)) {
                let frame = input_frames.last().cloned().unwrap_or_default();
                self.required_steps = self.required_steps.checked_sub(frame.len() as u64)
                    .ok_or_else(||error(expr.span,"scoped Type input preparation budget exhausted"))?;
                let mut mapping = frame.iter().enumerate().map(|(index,sort)|
                    Term::Bound {depth:0,index,sort:*sort}).collect::<Vec<_>>();
                let owner = self.signature_owner.unwrap();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let free = nir::free_bindings::collect(&root,&mut fuel)
                    .map_err(|message|error(expr.span,message));
                self.required_steps -= (before-fuel) as u64;
                let free = free?;
                for (binding, symbol) in &self.type_input_bindings {
                    if binding.owner != owner.def_id.raw() { continue; }
                    let Term::Bound {depth:0,index,sort:Sort::Type} = symbol else {continue;};
                    if frame.get(*index) != Some(&Sort::Type) {continue;}
                    let local = u32::try_from(*index).ok().filter(|index|*index!=u32::MAX)
                        .ok_or_else(||error(expr.span,"too many scoped Type inputs"))?;
                    let declaration = DeclarationId {package:u64::MAX,local};
                    mapping[*index] = Term::Rigid {declaration,sort:Sort::Type};
                    scoped_parameters.push(nir::DeclarationParameter {declaration,sort:Sort::Type});
                    scoped_arguments.push(symbol.clone());
                    if free.contains(binding) && !inputs.iter().any(|(id,_,_)|id==binding) {
                        inputs.push((*binding,p(P::Type),mapping[*index].clone()));
                    }
                }
                if !scoped_parameters.is_empty() {
                    let mut rebind=|ty:&Term| {
                        let mut fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                        let before=fuel;
                        let bounded=nir::type_computations::charge_term(ty,&mut fuel);
                        self.required_steps-=(before-fuel) as u64;
                        bounded.map_err(|e|error(expr.span,e.to_string()))?;
                        ty.rebind_scope(&mapping).map_err(|e|error(expr.span,e.to_string()))
                    };
                    root.rewrite_types(&mut rebind)?;
                    // Supplied environment values belong to the caller's frame
                    // too. Local-value binding and Type-slot binding are distinct:
                    // a supplied value must not replace the caller's Type identity.
                    for (_,ty,value) in &mut inputs {
                        *ty=rebind(ty)?;
                        *value=rebind(value)?;
                    }
                }
            }
            let mut functions = Vec::new();
            let mut runtime_function_identities = Vec::new();
            let mut members = BTreeSet::new();
            let mut expanded_members = BTreeSet::new();
            let mut associated = BTreeSet::new();
            let mut expanded_associated = BTreeSet::new();
            let mut queue = VecDeque::from_iter(references(&root, &mut members, &mut associated));
            let mut seen = BTreeSet::new();
            loop {
                // Abstract associated values can carry runtime code identities.
                // Retain their declaration metadata without checking or executing
                // unrelated runtime function bodies in this comptime snapshot.
                let pending:Vec<_>=associated.difference(&expanded_associated).copied().collect();
                for member in pending {
                    expanded_associated.insert(member);
                    // A generic comptime body may discover a member dependency
                    // that the enclosing initializer has not demanded yet.
                    let implementations:Vec<_>=self.associated_initializers.keys()
                        .filter(|(_,id)|*id==member).map(|(implementation,_)|*implementation).collect();
                    for implementation in implementations {
                        self.selected_associated_value(implementation,member,expr.span)?;
                    }
                    for implementation in &self.checked_implementations {
                        for (_,value) in implementation.associated.iter().filter(|(id,_)|*id==member) {
                            queue.extend(references(value,&mut members,&mut associated));
                        }
                    }
                }
                for member in members.difference(&expanded_members) {
                    if !self.interfaces_ready {
                        return Err(error(
                            expr.span,
                            "required comptime evaluation is waiting for the trait implementation world",
                        ));
                    }
                    let (interface, contract) = self
                        .checked_traits
                        .iter()
                        .find_map(|interface| {
                            interface
                                .methods
                                .iter()
                                .find(|method| method.declaration == *member)
                                .map(|method| (interface.declaration, method))
                        })
                        .ok_or_else(|| {
                            error(expr.span, "required comptime method contract is missing")
                        })?;
                    if !contract.signature.is_comptime_callable() && !contract.program.preconditions.is_empty() {
                        return Err(error(
                            expr.span,
                            "trait method value would erase program preconditions; callable program contracts are not implemented",
                        ));
                    }
                    // Checked implementation tables already contain the default
                    // body when it is inherited. An overridden default is not an
                    // execution dependency and may depend on this very query.
                    for implementation in &self.checked_implementations {
                        if implementation.interface == Some(interface) {
                            queue.extend(
                                implementation
                                    .methods
                                    .iter()
                                    .filter(|(name, _)| *name == contract.name)
                                    .map(|(_, body)| *body),
                            );
                        }
                    }
                }
                expanded_members.extend(members.iter().copied());
                let Some(id) = queue.pop_front() else {
                    break;
                };
                if !seen.insert(id) {
                    continue;
                }
                if seen.len() > 256 {
                    return Err(error(
                        expr.span,
                        "required comptime dependency budget exhausted",
                    ));
                }
                let owner = self
                    .package
                    .owners()
                    .find_map(|(o, _)| {
                        (declaration(self.package, &self.package_identities, o) == id).then_some(o)
                    })
                    .ok_or_else(|| {
                        error(
                            expr.span,
                            "required comptime function declaration is missing",
                        )
                    })?;
                if self.package.item(owner).is_some_and(|item| matches!(&item.kind,
                    ItemKind::Fn(sig, _) if !sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)) {
                    self.ensure_signature(owner, expr.span)?;
                    self.ensure_contract(owner, expr.span)?;
                    if self.program_requirements.get(&owner).is_some_and(|requirements| !requirements.is_empty()) {
                        return Err(error(expr.span, "function value would erase program preconditions; callable program contracts are not implemented"));
                    }
                    runtime_function_identities.push(n::RuntimeFunctionIdentity {
                        declaration: id,
                        type_parameters: self.type_parameters[&owner].clone(),
                        signature: self.signatures[&owner].clone(),
                    });
                    continue;
                }
                let function = self.required_function(owner, expr.span)?;
                queue.extend(references(&function.body, &mut members, &mut associated));
                functions.push(function);
            }
            // Signature formation may have produced obligations independently of
            // this expression's speculative queue. Settle both before execution.
            let uses: Vec<_> = self
                .pending_type_uses
                .as_ref()
                .unwrap()
                .iter()
                .cloned()
                .chain(self.signature_type_uses[signature_start..].iter().cloned())
                .collect();
            let saved_context = self.context.clone();
            let saved_owner = self.signature_owner;
            let checked = (|| {
                let mut queue = VecDeque::from(uses);
                let mut contracts = BTreeSet::new();
                let mut signature_cursor = self.signature_type_uses.len();
                while let Some((scope, assume_requirements, owner, arguments, span)) =
                    queue.pop_front()
                {
                    // Checking a contract operand uses only its binder facts.
                    // Its enclosing contract is still being formed and is not
                    // a prerequisite (nor an assumption available to itself).
                    let premise_owner = if assume_requirements { scope } else { None };
                    for dependency in [Some(owner), premise_owner].into_iter().flatten() {
                        if !matches!(
                            self.package.item(dependency).map(|i| &i.kind),
                            Some(ItemKind::Const(..))
                        ) {
                            self.ensure_contract(dependency, span)?;
                            if contracts.insert(dependency) {
                                queue.extend(
                                    self.contract_uses
                                        .get(&dependency)
                                        .into_iter()
                                        .flatten()
                                        .cloned(),
                                );
                            }
                        }
                    }
                    queue.extend(self.signature_type_uses[signature_cursor..].iter().cloned());
                    signature_cursor = self.signature_type_uses.len();
                    if !self.interfaces_ready
                        && self.capabilities.get(&owner).is_some_and(|c| !c.is_empty())
                    {
                        return Err(error(
                            span,
                            "required comptime evaluation is waiting for the trait implementation world",
                        ));
                    }
                    self.signature_owner = scope;
                    self.context = match scope {
                        Some(o) if assume_requirements => self.definition_context(o),
                        Some(o) => self.binder_context(o),
                        None => self.nominal_context(),
                    };
                    self.check_formation_requirements(owner, &arguments, span)?;
                }
                Ok(())
            })();
            self.context = saved_context;
            self.signature_owner = saved_owner;
            checked?;
            if !self.interfaces_ready && self.pending_refinements.iter().any(|pending| {
                let id=declaration(self.package,&self.package_identities,pending.owner);
                seen.contains(&id) || members.contains(&id)
            }) {
                return Err(error(expr.span,"required comptime evaluation is waiting for a signature type refinement"));
            }
            let entry = DeclarationId {
                package: u64::MAX,
                local: u32::MAX,
            };
            let mut entry_fuel=usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before=entry_fuel;
            // Retain the source telescope, including unused runtime slots:
            // type annotations refer to these original positions. Capture
            // compaction will reject a runtime read and prune unused inputs.
            // Completed caller values and pending staged inputs are distinct.
            let mut available = Vec::new();
            if let Some(owner) = self.signature_owner.filter(|owner| self.active_signatures.contains(owner)) {
                if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                    self.package.item(owner).map(|item| &item.kind) {
                    for (parameter, domain) in signature.params.iter().zip(self.signature_prefixes.get(&owner).into_iter().flatten()) {
                        let binding = local(parameter.hir_id);
                        let supplied = inputs.iter().position(|(id, _, _)| *id == binding)
                            .map(|index| inputs.remove(index));
                        let (ty, value) = match supplied {
                            Some((_, ty, value)) => (ty, Some(value)),
                            None => (domain.ty.clone(), None),
                        };
                        available.push((binding, Parameter { sort: domain.sort, ty }, value));
                    }
                }
            }
            for (binding, ty, value) in inputs {
                available.push((binding, Parameter {
                    sort: if ty == p(P::Type) { Sort::Type } else { Sort::ComptimeValue }, ty,
                }, Some(value)));
            }
            let captured=nir::type_computations::capture_entry(
                entry,"<required-comptime>".into(),root,
                &available.iter().map(|(binding, parameter, _)| (*binding, parameter.clone())).collect::<Vec<_>>(),
                &mut entry_fuel,
            );
            self.required_steps-=(before-entry_fuel) as u64;
            let captured=captured.map_err(|failure| error(expr.span, match failure {
                nir::type_computations::Error::MissingCapture(binding) if self.required_runtime_parameter(binding) =>
                    "required comptime computation cannot read a runtime parameter; defer does not change the parameter's phase".into(),
                nir::type_computations::Error::MissingCapture(binding) =>
                    format!("required comptime input {binding} is not available in this invocation"),
                other => other.to_string(),
            }))?;
            let selected=captured.slots.iter().copied().collect::<BTreeSet<_>>();
            let inputs=available.into_iter().enumerate().filter_map(|(slot,(binding,parameter,value))|
                selected.contains(&slot).then_some((binding,parameter.ty,value))).collect();
            functions.push(captured.function);
            // Preserve the checked implementation world, projecting only its
            // method tables to the members this invocation can reference. This
            // avoids requiring unrelated runtime bodies in a comptime snapshot.
            let mut traits = self.checked_traits.clone();
            for interface in &mut traits {
                interface
                    .methods
                    .retain(|method| members.contains(&method.declaration));
                for method in &mut interface.methods {
                    if method.default.is_some_and(|body| !seen.contains(&body)) {
                        // Project the unused fallback out of this execution
                        // snapshot only. The final package still checks and
                        // retains the original default declaration.
                        method.default = None;
                    }
                }
            }
            let mut implementations = self.checked_implementations.clone();
            for implementation in &mut implementations {
                implementation.methods.retain(|(name, _)| {
                    traits
                        .iter()
                        .find(|interface| Some(interface.declaration) == implementation.interface)
                        .is_some_and(|interface| {
                            interface.methods.iter().any(|method| method.name == *name)
                        })
                });
            }
            let mut package = n::Package {
                runtime_function_identities,
                traits,
                implementations,
                normal_forms: self.normal_form_metadata(),
                functions,
                structs: self.structs.values().cloned().collect(),
                enums: self.enums.values().cloned().collect(),
                ..Default::default()
            };
            crate::verification::contracts::inherit(&mut package, &mut self.next_temporary_binding)
                .map_err(|failure| error(expr.span, failure.to_string()))?;
            let invocation_assumptions=if let Some(implementation)=self.initializer_implementation() {
                crate::capabilities::capability_sources(&package.traits,&implementation.capabilities,crate::ptc::Budget::default())
                    .map_err(|message|error(expr.span,message))?
            } else {vec![]};
            // The snapshot contains demanded layouts only. Nominal identities
            // are admitted from source declarations even when no value/layout
            // of that type is needed by this comptime invocation.
            let type_context = if self.initializer_implementation().is_some() {
                self.context.clone()
            } else {
                self.nominal_context()
            };
            package.dispatch_instances =
                crate::materialization::dispatch_instances_in_context(
                    &package, &type_context, &invocation_assumptions,
                    crate::ptc::Budget::default(),
                )
                .map_err(|failure| error(
                    expr.span,
                    format!("required comptime dispatch preparation failed: {failure}"),
                ))?;
            let mut abstract_parameters=self.signature_owner.and_then(|owner|self.type_parameters.get(&owner))
                .cloned().unwrap_or_default();
            abstract_parameters.extend(scoped_parameters.iter().map(|parameter|parameter.declaration));
            // Equality observations of opaque caller Type inputs need their
            // declaration facts. Dispatch preparation above still uses its
            // own world; these facts are supplied only to this invocation.
            let invocation_type_context=if abstract_parameters.is_empty() {&type_context} else {&self.context};
            crate::verification::flow::validate_machine_operations(&package)
                .map_err(|failure| {
                    if matches!(failure.reason, crate::verification::flow::Reason::Phase("runtime function is unavailable during comptime execution")) {
                        error(expr.span, format!("required comptime evaluation cannot execute a runtime function or normal-form head: {failure}"))
                    } else { error(expr.span, failure.to_string()) }
                })?;
            let implementation_index = prepared::implementation_index(&package, expr.span)?;
            let prepared = PreparedRequired {
                package, implementation_index, entry, inputs, result_type, scoped_parameters, scoped_arguments,
                input_frames, invocation_assumptions, abstract_parameters,
                context: invocation_type_context.clone(),
                in_signature: self.signature_owner.is_some_and(|owner| self.active_signatures.contains(&owner)),
            };
            Ok(prepared)
        })();
        let pending = std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
        if let Some(uses) = &mut self.pending_type_uses {
            uses.extend(pending);
        }
        self.result = saved_result;
        self.partial_return_context = saved_partial_return;
        self.pending_optional_returns = saved_optional_returns;
        self.return_observations = saved_returns;
        self.loops = saved_loops;
        self.captures = saved_captures;
        self.in_required_comptime = saved_mode;
        self.numeric_defaults = saved_numeric;
        self.active_required.remove(&local(expr.hir_id));
        result
    }

    pub(super) fn required_function(&mut self, owner: OwnerId, span: Span) -> Result<n::Function, Error> {
        let item = self.package.item(owner).unwrap();
        let ItemKind::Fn(sig, body_id) = &item.kind else {
            return Err(error(span, "required comptime call needs a function body"));
        };
        if !sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head {
            return Err(error(
                span,
                "required comptime evaluation cannot execute a runtime function or normal-form head",
            ));
        }
        self.ensure_signature(owner, span)?;
        self.ensure_contract(owner, span)?;
        let signature = self.signatures[&owner].clone();
        let saved_owner = self.signature_owner.replace(owner);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_result = self.result.take();
        let saved_partial_return = self.partial_return_context.take();
        let saved_optional_returns = std::mem::take(&mut self.pending_optional_returns);
        let saved_returns = self.return_observations.take();
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_immutable = std::mem::take(&mut self.immutable);
        let saved_context = self.context.clone();
        let saved_contract = std::mem::replace(&mut self.checking_contract, false);
        let saved_defaults = std::mem::replace(&mut self.numeric_defaults, true);
        self.context = self.definition_context(owner);
        let result = (|| {
            let Term::Function { parameters, result } = &signature else {
                unreachable!()
            };
            for (param, ty) in sig.params.iter().zip(parameters) {
                self.locals.insert(local(param.hir_id), ty.ty.clone());
            }
            self.result = Some((**result).clone());
            let body = self
                .package
                .body(*body_id)
                .ok_or_else(|| error(span, "required comptime body is missing"))?;
            let preconditions = self.program_preconditions(owner)?;
            let result_binding = sig.return_binding.map(local);
            let (postconditions, entry_snapshots) =
                self.program_postconditions(owner, result_binding, result)?;
            let value = self.expr(body.value, Some(result))?;
            Ok(n::Function {
                declaration: declaration(self.package, &self.package_identities, owner),
                name: item.ident.name.as_str().into(),
                phase: n::FunctionPhase::Comptime,
                inline: sig.modifiers.is_inline, pure: sig.modifiers.is_pure,
                preconditions,
                postconditions,
                entry_snapshots,
                result_binding,
                type_parameters: self.type_parameters[&owner].clone(),
                requirements: self.requirements.get(&owner).cloned().unwrap_or_default(),
                capabilities: self.capabilities.get(&owner).cloned().unwrap_or_default(),
                signature,
                parameters: sig.params.iter().map(|p| local(p.hir_id)).collect(),
                // Call plans carry the defaults actually used, with the
                // appropriate substitutions and parameter frame. Template
                // defaults are checked later for the final package; forcing
                // them here adds dependencies to explicitly supplied inputs.
                defaults: vec![],
                body: value,
            })
        })();
        self.signature_owner = saved_owner;
        self.locals = saved_locals;
        self.captures = saved_captures;
        self.result = saved_result;
        self.partial_return_context = saved_partial_return;
        self.pending_optional_returns = saved_optional_returns;
        self.return_observations = saved_returns;
        self.loops = saved_loops;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.checking_contract = saved_contract;
        self.numeric_defaults = saved_defaults;
        result
    }
}

// The traversal includes guards and pattern computations; a function hidden in
// one of those positions must not escape phase/dependency checks.
pub(super) fn references(root: &n::Expr, members: &mut BTreeSet<DeclarationId>, associated: &mut BTreeSet<DeclarationId>) -> BTreeSet<DeclarationId> {
    enum Node<'a> {
        Expr(&'a n::Expr),
        Pattern(&'a n::Pattern),
    }
    let mut pending = vec![Node::Expr(root)];
    let mut output = BTreeSet::new();
    while let Some(node) = pending.pop() {
        match node {
            Node::Pattern(p) => match p {
                n::Pattern::Then { first, value, next } => {
                    pending.push(Node::Pattern(first));
                    pending.push(Node::Expr(value));
                    pending.push(Node::Pattern(next));
                }
                n::Pattern::Guard { pattern, condition } => {
                    pending.push(Node::Pattern(pattern));
                    pending.push(Node::Expr(condition));
                }
                n::Pattern::Remap { pattern, .. } | n::Pattern::Binding { inner: pattern, .. } => {
                    pending.push(Node::Pattern(pattern))
                }
                n::Pattern::Record { fields, .. } | n::Pattern::AnonymousRecord { fields, .. }
                | n::Pattern::Variant { fields, .. }
                | n::Pattern::NormalForm {
                    arguments: fields, ..
                }
                | n::Pattern::TypeTuple(fields)
                | n::Pattern::Tuple(fields)
                | n::Pattern::Or(fields) => pending.extend(fields.iter().map(Node::Pattern)),
                n::Pattern::Wild | n::Pattern::Unit | n::Pattern::Literal(_) => {}
            },
            Node::Expr(e) => match &e.kind {
                K::EffectHandle {computation, handlers, ..} => {
                    pending.push(Node::Expr(computation));
                    for handler in handlers {for arm in &handler.arms {
                        pending.push(Node::Pattern(&arm.pattern));
                        if let Some(g) = &arm.guard {pending.push(Node::Expr(g));}
                        pending.push(Node::Expr(&arm.body));
                    }}
                }
                K::EffectResume(body) | K::EffectComputation {body, ..} | K::EffectRun {computation:body, ..} => pending.push(Node::Expr(body)),
                K::EffectRequest {arguments, ..} => pending.extend(arguments.iter().map(Node::Expr)),
                K::Function(id)
                | K::FunctionInstance {
                    declaration: id, ..
                } => {
                    output.insert(*id);
                }
                K::Closure { captures, body, .. } => {
                    pending.push(Node::Expr(body));
                    pending.extend(
                        captures
                            .iter()
                            .filter_map(|c| c.value.as_deref())
                            .map(Node::Expr),
                    );
                }
                K::Call {
                    callee, arguments, ..
                } => {
                    pending.push(Node::Expr(callee));
                    pending.extend(arguments.iter().map(|a| Node::Expr(&a.value)));
                }
                K::NormalFormType { arguments, plan, .. } => {
                    pending.extend(plan.iter().flat_map(|p| &p.preparations).map(|p| Node::Expr(&p.expression)));
                    pending.extend(
                        arguments
                            .iter()
                            .flat_map(|a| a.patterns())
                            .map(Node::Pattern),
                    );
                    pending.extend(
                        arguments
                            .iter()
                            .flat_map(|a| a.expressions())
                            .map(Node::Expr),
                    );
                }
                K::ClosureType {
                    environment,
                    signature,
                } => {
                    pending.extend(environment.iter().map(Node::Expr));
                    pending.push(Node::Expr(signature));
                }
                K::NominalType {
                    arguments: fields, ..
                }
                | K::FunctionType {
                    parameters: fields, ..
                }
                | K::TupleType(fields)
                | K::Tuple(fields)
                | K::AnonymousRecord { fields } | K::Record { fields, .. }
                | K::Variant { fields, .. } => pending.extend(fields.iter().map(Node::Expr)),
                K::AnonymousField { value, .. } | K::Field { value, .. }
                | K::Let { value, .. }
                | K::Assign { value, .. }
                | K::BitAssign { value, .. }
                | K::Discard(value)
                | K::RequirePredicate(value) | K::EnsurePredicate(value) | K::CheckPredicate(value)
                | K::BitSlice { value, .. }
                | K::BitConvert(value)
                | K::Snapshot(value) | K::TakeBits(value) | K::SymbolName(value) | K::EnumName(value)
                | K::Not(value)
                | K::IntegerNegate(value)
                | K::FloatNegate(value)
                | K::AddressOf(value)
                | K::ReadPointer(value)
                | K::PointerAnonymousField { pointer: value, .. }
                | K::PointerField { pointer: value, .. }
                | K::PointerType(value)
                | K::BitVecType(value)
                | K::Loop { body: value, .. } => pending.push(Node::Expr(value)),
                K::ScalarLess(a, b)
                | K::ScalarGreater(a, b)
                | K::ScalarGreaterEqual(a, b)
                | K::ScalarLessEqual(a, b)
                | K::ScalarEqual(a, b)
                | K::TypeEqual(a, b)
                | K::TypeListIndex(a, b)
                | K::BitEqual(a, b)
                | K::BitConcat(a, b) | K::BitIndex(a, b)
                | K::And(a, b)
                | K::Or(a, b)
                | K::WritePointer {
                    pointer: a,
                    value: b,
                }
                | K::FunctionResultType {
                    signature: a,
                    result: b,
                }
                | K::IntegerBinary {
                    left: a, right: b, ..
                }
                | K::FloatBinary {
                    left: a, right: b, ..
                } => {
                    pending.push(Node::Expr(a));
                    pending.push(Node::Expr(b));
                }
                K::BitWritePointer {pointer,selections,value} => {
                        pending.push(Node::Expr(pointer));
                        pending.extend(selections.iter().flat_map(n::BitSelection::expressions).map(Node::Expr));
                        pending.push(Node::Expr(value));
                    }
                    K::BitUpdate {selections, value, ..} => {
                        pending.extend(selections.iter().flat_map(n::BitSelection::expressions).map(Node::Expr));
                        pending.push(Node::Expr(value));
                    }
                    K::BitWindow {value, start, end} => {
                        pending.push(Node::Expr(value));
                        pending.push(Node::Expr(start));
                        pending.push(Node::Expr(end));
                    }
                    K::Match { value, arms } => {
                    pending.push(Node::Expr(value));
                    for arm in arms {
                        pending.push(Node::Pattern(&arm.pattern));
                        if let Some(g) = &arm.guard {
                            pending.push(Node::Expr(g));
                        }
                        pending.push(Node::Expr(&arm.body));
                    }
                }
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    pending.extend(statements.iter().map(Node::Expr));
                    if let Some(r) = result {
                        pending.push(Node::Expr(r));
                    }
                }
                K::LetPattern {
                    pattern,
                    value,
                    guard,
                    otherwise,
                } => {
                    pending.push(Node::Pattern(pattern));
                    pending.push(Node::Expr(value));
                    if let Some(g) = guard {
                        pending.push(Node::Expr(g));
                    }
                    if let Some(e) = otherwise {
                        pending.push(Node::Expr(e));
                    }
                }
                K::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    pending.push(Node::Expr(condition));
                    pending.push(Node::Expr(then_branch));
                    if let Some(e) = else_branch {
                        pending.push(Node::Expr(e));
                    }
                }
                K::Return(value) => {
                    if let Some(e) = value {
                        pending.push(Node::Expr(e));
                    }
                }
                K::TraitMethod { member, .. } => {
                    members.insert(*member);
                }
                K::AssociatedValue {member,..} => {associated.insert(*member);}
                K::TypeValue(_) | K::TypeListValue(_)
                | K::Literal(_)
                | K::Local(_)
                | K::Unit
                | K::Break(_)
                | K::Continue(_) => {}
            },
        }
    }
    output
}

// Classification follows resolved value binders, never a failed PTC attempt.
// Unsupported mixed/specification forms retain their diagnostic rather than
// silently acquiring a different solver meaning.
pub(super) fn mentions_value_parameter(value: &Expr<'_>, depth: usize, is_type: &impl Fn(hir::HirId) -> bool) -> Result<bool, Error> {
    if depth >= 128 {
        return Err(error(
            value.span,
            "program precondition classification depth exhausted",
        ));
    }
    Ok(match &value.kind {
        H::ResolvedFnParam(_, id) => !is_type(*id),
        H::SelfValue => true,
        H::RefinedType {requirement,base} => {
            mentions_value_parameter(requirement,depth+1,is_type)? || mentions_value_parameter(base,depth+1,is_type)?
        }
        H::Binary(_, a, b) | H::Index(a, b) | H::Cast(a, b) | H::TraitBound(a, b) => {
            mentions_value_parameter(a, depth + 1, is_type)? || mentions_value_parameter(b, depth + 1, is_type)?
        }
        H::TakeOld(value)
        | H::Deref(value)
        | H::Ref(value)
        | H::MultiDeref(value)
        | H::TakeBitVec(value) | H::TakeTagName(value)
        | H::Defer(value)
        | H::Unary(_, value)
        | H::Projection(value, _)
        | H::LiteralConversion { literal: value, .. } => {
            mentions_value_parameter(value, depth + 1, is_type)?
        }
        _ => false,
    })
}

// Only conjunction exposes individual positive capabilities. Do not extract
// assumptions from beneath negation or disjunction.
pub(super) fn signature_capability_atoms<'h>(
    value: &Expr<'h>, pending: &mut Vec<Expr<'h>>, depth: usize,
) -> Result<(), Error> {
    if depth >= 128 { return Err(error(value.span, "signature capability depth exhausted")); }
    match value.kind {
        H::Binary(BinOp::And, left, right) => {
            signature_capability_atoms(left, pending, depth + 1)?;
            signature_capability_atoms(right, pending, depth + 1)?;
        }
        H::TraitBound(subject, bound) => {
            if let H::Binary(BinOp::Add, left, right) = bound.kind {
                for bound in [left, right] {
                    let atom = Expr { kind: H::TraitBound(subject, bound), ..value.clone() };
                    signature_capability_atoms(&atom, pending, depth + 1)?;
                }
            } else {
                pending.push(value.clone());
            }
        }
        _ => {}
    }
    Ok(())
}

// This is readiness, not type checking. Unknown expression forms wait for the
// final signature pass, where normal elaboration supplies their diagnostics.
fn signature_inputs_ready(
    value: &Expr<'_>, depth: usize, available: &impl Fn(hir::HirId) -> bool,
) -> Result<bool, Error> {
    if depth >= 128 { return Err(error(value.span, "signature input dependency depth exhausted")); }
    let ready = |value| signature_inputs_ready(value, depth + 1, available);
    Ok(match &value.kind {
        H::ResolvedFnParam(_, id) => available(*id),
        H::Lit(_) | H::ResolvedClauseParam(..) | H::ResolvedDef {..} | H::Builtin(_)
        | H::TyType | H::TyAny | H::TyVoid | H::TyNoReturn | H::TySelf | H::Unit => true,
        H::TraitBound(a,b) | H::Binary(_,a,b) | H::Cast(a,b) | H::Index(a,b)
        | H::TyFnArrow(a,b) | H::TyClosure(a,b) | H::EffectQualifiedType(a,b) => ready(a)? && ready(b)?,
        H::Projection(value,_) | H::TyPtr(value) | H::TyOptional(value)
        | H::Unary(_,value) | H::LiteralConversion {literal:value,..} | H::TypeOf(value) => ready(value)?,
        H::Tuple(values) | H::List(values) => {
            let mut all = true;
            for value in *values { all &= ready(value)?; }
            all
        }
        H::NFApplication(head,args) | H::Application(head,args) | H::ExtendedApplication(head,args) => {
            let mut all = ready(head)?;
            for arg in *args {
                let (Arg::Positional(value) | Arg::Named(_,value) | Arg::Expand(value)
                    | Arg::Implicit(value) | Arg::PostLambda(value)) = arg;
                all &= ready(value)?;
            }
            all
        }
        H::TyFn(parameters) | H::TyComptimeFn(parameters) | H::TyNFFn(parameters) => {
            let mut all = true;
            for parameter in *parameters {
                match parameter.kind {
                    TyParamKind::Optional(_,ty,default) => { all &= ready(ty)? && ready(default)?; }
                    TyParamKind::Positional(ty) | TyParamKind::PositionalDependencyCatched(_,ty)
                    | TyParamKind::Varadic(_,ty) => { all &= ready(ty)?; }
                    TyParamKind::Itself {..} => {}
                }
            }
            all
        }
        _ => false,
    })
}

/// Own the synthetic subjects while atoms borrow their declaration identities.
pub(super) fn parameter_capability_subjects<'h>(parameters: &[hir::clause::ClauseParam<'h>]) -> Result<Vec<(Expr<'h>, &'h Expr<'h>)>, Error> {
    let mut subjects = Vec::new();
    for parameter in parameters {
        if let hir::clause::ClauseParamKind::TraitBound(_, bound) = parameter.kind {
            let id = parameter.definition.ok_or_else(|| error(parameter.span, "missing clause parameter identity"))?;
            subjects.push((Expr { hir_id: parameter.hir_id, span: parameter.span,
                kind: H::ResolvedClauseParam(parameter.name.name, id) }, bound));
        }
    }
    Ok(subjects)
}

/// Predicate terms remain type propositions, including Type-valued ordinary inputs.
pub(super) fn requirement_terms<'a>(value: &'a n::TypePredicate, span: Span) -> Result<Vec<&'a Term>, Error> {
    let mut pending = vec![(value, 0usize)];
    let mut terms = Vec::new();
    let mut fuel = 10_000usize;
    while let Some((value, depth)) = pending.pop() {
        if depth >= 128 || fuel == 0 { return Err(error(span, "type requirement traversal budget exhausted")); }
        fuel -= 1;
        match value {
            n::TypePredicate::Bool(_) => {},
            n::TypePredicate::Trait(r) => { terms.push(&r.subject); terms.extend(&r.interface_arguments); },
            n::TypePredicate::Equal(a,b) => { terms.push(a); terms.push(b); },
            n::TypePredicate::Not(p) => pending.push((p,depth+1)),
            n::TypePredicate::And(ps) | n::TypePredicate::Or(ps) => pending.extend(ps.iter().map(|p|(p,depth+1))),
        }
    }
    Ok(terms)
}

pub(super) fn map_requirement(value: &n::TypePredicate, map: &mut impl FnMut(&Term)->Result<Term,Error>, span:Span, depth:usize) -> Result<n::TypePredicate,Error> {
    use n::TypePredicate as Q;
    if depth >= 128 { return Err(error(span,"type requirement mapping depth exhausted")); }
    Ok(match value {
        Q::Bool(v) => Q::Bool(*v),
        Q::Trait(r) => Q::Trait(n::TraitRequirement {interface:r.interface, subject:map(&r.subject)?,
            interface_arguments:r.interface_arguments.iter().map(&mut *map).collect::<Result<_,_>>()?}),
        Q::Equal(a,b) => Q::Equal(map(a)?,map(b)?),
        Q::Not(p) => Q::Not(Box::new(map_requirement(p,map,span,depth+1)?)),
        Q::And(ps) | Q::Or(ps) => {
            let parts=ps.iter().map(|p|map_requirement(p,map,span,depth+1)).collect::<Result<_,_>>()?;
            if matches!(value,Q::And(_)) {Q::And(parts)} else {Q::Or(parts)}
        }
    })
}
