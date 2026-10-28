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
        let mut context = self.context.theory_in_scope(self.call_input_frames());
        for atom in atoms {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "signature domain constraint budget exhausted"))?;
            if !signature_inputs_ready(&atom, 0, &|id| {
                self.type_input_bindings.contains_key(&local(id))
            })? || self.program_requirement(&atom)?
            {
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

    fn contract_parameter_is_type(&self, value: &Expr<'_>) -> bool {
        match value.kind {
            H::ResolvedFnParam(_, id) => self.type_input_bindings.contains_key(&local(id)),
            H::ResolvedClauseParam(_, id) => self
                .trait_input_domain(DeclarationId {
                    package: semantic_package(&self.package_identities, id.pkg),
                    local: id.index,
                })
                .is_none_or(|domain| domain == p(P::Type)),
            _ => false,
        }
    }

    pub(super) fn program_requirement(&mut self, value: &Expr<'_>) -> Result<bool, Error> {
        // Program predicates need not mention an ordinary parameter: a
        // completed associated Boolean can be the whole condition. Classify
        // before elaborating; a failed type proof must never trigger fallback
        // interpretation under a different meaning.
        fn type_shape(
            checker: &mut Checker<'_, '_>,
            value: &Expr<'_>,
            depth: usize,
        ) -> Result<bool, Error> {
            if depth >= 128 {
                return Err(error(
                    value.span,
                    "program precondition classification depth exhausted",
                ));
            }
            Ok(match &value.kind {
                H::Binary(BinOp::Eq | BinOp::Ne, left, right) => {
                    !matches!(left.kind, H::Lit(_))
                        && !matches!(right.kind, H::Lit(_))
                        && !checker
                            .contract_projection_domain(left)?
                            .is_some_and(|ty| ty != p(P::Type))
                        && !checker
                            .contract_projection_domain(right)?
                            .is_some_and(|ty| ty != p(P::Type))
                }
                H::TraitBound(..)
                | H::Lit(hir::Lit {
                    kind: hir::LitKind::Bool(_),
                    ..
                }) => true,
                H::Unary(UnOp::Not, inner) => type_shape(checker, inner, depth + 1)?,
                H::Binary(BinOp::And | BinOp::Or, left, right) => {
                    type_shape(checker, left, depth + 1)? && type_shape(checker, right, depth + 1)?
                }
                _ => false,
            })
        }
        Ok(
            mentions_value_parameter(value, 0, &|value| self.contract_parameter_is_type(value))?
                || !type_shape(self, value, 0)?,
        )
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
                ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) | ItemKind::Effect(sig, _) => {
                    sig.clause_constraints
                }
                ItemKind::EffectGroup(group) => group.clause_constraints,
                ItemKind::TypeAlias(_, _, constraints) | ItemKind::Newtype(_, _, constraints) => {
                    *constraints
                }
                ItemKind::Struct(def) => def.clause_constraints,
                ItemKind::Enum(def) => def.clause_constraints,
                ItemKind::Impl(def) => def.clause_constraints,
                ItemKind::Trait(def) => def.clause_constraints,
                ItemKind::Associated(def) => def.clause_constraints,
                // Lexical const initializers and explicit template-instance inputs
                // have no declaration clauses of their own. The selected
                // template contract is checked after preparing its inputs.
                ItemKind::Const(..) | ItemKind::TestTemplateInstance { .. } => &[],
                _ => return Err(error(span, "declaration does not have a type contract")),
            };
            let lexical_parent = matches!(
                self.package.item(owner).map(|item| &item.kind),
                Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(..))
            )
            .then(|| self.package.lexical_parents.get(&owner).copied())
            .flatten();
            let mut requirements =
                if let Some(parent) = self.effect_parents.get(&owner).copied().or(lexical_parent) {
                    self.ensure_contract(parent, span)?;
                    if lexical_parent == Some(parent) {
                        self.requirements[&parent]
                            .iter()
                            .map(|requirement| {
                                map_requirement(
                                    requirement,
                                    &mut |ty| self.lift_lexical_type(parent, ty, span),
                                    span,
                                    0,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?
                    } else {
                        self.requirements[&parent].clone()
                    }
                } else {
                    Vec::new()
                };
            // Admission uses the same prerequisite discipline as signatures.
            // Only positive type atoms may become declaration assumptions.
            let mut pending_capabilities = Vec::new();
            for clause in clauses {
                if let hir::clause::ClauseConstraintKind::Requires(value) = &clause.kind {
                    if !self.program_requirement(value)? {
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
                if self.program_requirement(value)? {
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
                Some(capabilities) => {
                    self.capabilities.insert(owner, capabilities);
                }
                None => {
                    self.capabilities.remove(&owner);
                }
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
        if let Some(def) =
            self.newtypes
                .get_mut(&declaration(self.package, &self.package_identities, owner))
        {
            def.requirements = requirements.clone();
            def.capabilities = self.capabilities.get(&owner).cloned().unwrap_or_default();
        }
        self.requirements.insert(owner, requirements);
        Ok(())
    }

    pub(super) fn prepare_parameter_capabilities(
        &mut self,
        owner: OwnerId,
        parameters: &[hir::clause::ClauseParam<'_>],
    ) -> Result<(), Error> {
        self.prepare_parameter_capabilities_prefix(owner, parameters, true)
    }

    pub(super) fn prepare_parameter_capabilities_prefix(
        &mut self,
        owner: OwnerId,
        parameters: &[hir::clause::ClauseParam<'_>],
        complete: bool,
    ) -> Result<(), Error> {
        let subjects = parameter_capability_subjects(parameters)?;
        if subjects.is_empty() {
            return Ok(());
        }
        let mut pending = Vec::new();
        for (subject, bound) in &subjects {
            let atom = Expr {
                kind: H::TraitBound(subject, bound),
                ..subject.clone()
            };
            signature_capability_atoms(&atom, &mut pending, 0)?;
        }
        let constraints = match &self.package.item(owner).unwrap().kind {
            ItemKind::TypeAlias(_, _, constraints) | ItemKind::Newtype(_, _, constraints) => {
                *constraints
            }
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
        self.ready_signature_capabilities_with_clauses(owner, &mut pending, complete, true)
    }

    // Positive capability conjuncts share the existing trait evidence path.
    // Never extract an atom from beneath not/or: that would strengthen the
    // declaration or turn failure to select into negative evidence.
    /// Publish each ready positive bound once, keeping later inputs pending.
    pub(super) fn ready_signature_capabilities<'h>(
        &mut self,
        owner: OwnerId,
        pending: &mut Vec<Expr<'h>>,
        complete: bool,
    ) -> Result<(), Error> {
        // During signature construction clause domains are published as a
        // telescope prefix. Do not inspect bounds that read later inputs.
        let check_clauses = self.normal_input_prefixes.contains_key(&owner);
        self.ready_signature_capabilities_with_clauses(owner, pending, complete, check_clauses)
    }

    pub(super) fn ready_signature_capabilities_with_clauses<'h>(
        &mut self,
        owner: OwnerId,
        pending: &mut Vec<Expr<'h>>,
        complete: bool,
        check_clauses: bool,
    ) -> Result<(), Error> {
        loop {
            let mut progress = false;
            let mut waiting = None;
            for value in std::mem::take(pending) {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(
                        value.span,
                        "signature capability preparation budget exhausted",
                    )
                })?;
                let ready = complete
                    || signature_inputs_ready_with_clauses(
                        &value,
                        0,
                        &|id| {
                            self.type_input_bindings.contains_key(&local(id))
                                || self.value_input_bindings.contains_key(&local(id))
                        },
                        &|id| {
                            !check_clauses
                                || self.type_clause_params(owner).iter().any(|parameter| {
                                    parameter.definition == Some(id)
                                        && matches!(
                                            parameter.kind,
                                            hir::clause::ClauseParamKind::Type(_)
                                                | hir::clause::ClauseParamKind::TraitBound(..)
                                        )
                                })
                                || self
                                    .trait_input_domain(DeclarationId {
                                        package: semantic_package(&self.package_identities, id.pkg),
                                        local: id.index,
                                    })
                                    .is_some()
                        },
                    )?;
                if !ready {
                    pending.push(value);
                    continue;
                }
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
                            Some(capabilities) => {
                                self.capabilities.insert(owner, capabilities);
                            }
                            None => {
                                self.capabilities.remove(&owner);
                            }
                        }
                        if let (Some(uses), Some(len)) = (&mut self.pending_type_uses, pending_len)
                        {
                            uses.truncate(len);
                        }
                        if !failure.needs_capability {
                            return Err(failure);
                        }
                        waiting.get_or_insert(failure);
                        pending.push(value);
                    }
                }
            }
            if pending.is_empty() {
                break;
            }
            if !progress {
                if complete {
                    if let Some(failure) = waiting {
                        return Err(failure);
                    }
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
        &mut self,
        owner: OwnerId,
        result: Option<(n::LocalId, &Term)>,
    ) -> Result<(Vec<n::Expr>, Vec<n::EntrySnapshot>), Error> {
        let values = self
            .program_postconditions
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        self.program_postcondition_values(owner, result, values)
    }

    pub(super) fn program_postcondition_values(
        &mut self,
        owner: OwnerId,
        result: Option<(n::LocalId, &Term)>,
        values: Vec<&Expr<'_>>,
    ) -> Result<(Vec<n::Expr>, Vec<n::EntrySnapshot>), Error> {
        let result = result.map(|(binding, ty)| {
            let ty = if let Term::Computation { result, .. } = ty {
                result.as_ref()
            } else {
                ty
            };
            (binding, ty)
        });
        let old_scope = self.old_scope.replace(OldScope {
            owner,
            active: false,
            bindings: BTreeMap::new(),
        });
        let binding = result.map(|(binding, _)| binding);
        let previous = result.and_then(|(binding, ty)| self.locals.insert(binding, ty.clone()));
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
        let saved_normal_bindings = self.normal_default_bindings.clone();
        let saved_scopes = std::mem::take(&mut self.alias_scopes);
        self.context = self.binder_context(owner);
        // Normalize/check the target as a Type-valued expression without
        // demanding concrete results of calls whose inputs are still symbolic.
        let result = (|| {
            let mut parameters = self.nominal_captures(owner, span)?;
            let mut domains = parameters
                .iter()
                .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                .collect::<Vec<_>>();
            parameters.extend(self.type_parameters[&owner].iter().copied());
            domains.extend(self.trait_domains(owner)?);
            for (parameter, domain) in parameters.into_iter().zip(domains) {
                if domain != p(P::Type) {
                    let binding = self.spread_binding(span)?;
                    self.locals.insert(binding, domain.clone());
                    self.normal_default_bindings
                        .insert(parameter, (binding, domain));
                }
            }
            self.expr(value, Some(&p(P::Type))).map(|_| ())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.in_required_comptime = saved_mode;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_value_bindings;
        self.alias_scopes = saved_scopes;
        self.normal_default_bindings = saved_normal_bindings;
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
        let captured: Vec<_> = self
            .alias_substitution
            .iter()
            .map(|(id, value)| (*id, value.clone()))
            .collect();
        let environment = captured
            .into_iter()
            .map(|(id, value)| {
                self.copy_alias_type(&value, span)
                    .map(|value| (id, std::sync::Arc::new(value)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let key = (owner, arguments.clone(), environment);
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

    pub(super) fn ensure_alias_formation_contract(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<DeclarationId>, Error> {
        let declaration = declaration(self.package, &self.package_identities, owner);
        if let Some(contract) = self.alias_contracts.get(&declaration) {
            return Ok(contract.parameters.clone());
        }
        self.ensure_contract(owner, span)?;
        let mut requirements = self.requirements[&owner].clone();
        let mut capabilities = self.capabilities.get(&owner).cloned().unwrap_or_default();
        let mut container = self.self_containers.get(&owner).copied();
        let mut depth = 0;
        while let Some(parent) = container {
            depth += 1;
            if depth > 64 {
                return Err(error(
                    span,
                    "alias formation environment is excessively deep",
                ));
            }
            self.ensure_contract(parent, span)?;
            for premise in &self.requirements[&parent] {
                if !requirements.contains(premise) {
                    requirements.push(premise.clone());
                }
            }
            for capability in self.capabilities.get(&parent).into_iter().flatten() {
                if !capabilities.contains(capability) {
                    capabilities.push(capability.clone());
                }
            }
            container = self.self_containers.get(&parent).copied();
        }
        let mut parameters = self.type_parameters[&owner].clone();
        let mut domains = self.trait_domains(owner)?;
        if domains.is_empty() {
            domains.resize(parameters.len(), p(P::Type));
        }
        if requirements.is_empty()
            && capabilities.is_empty()
            && domains.iter().all(|ty| *ty == p(P::Type))
        {
            return Ok(parameters);
        }
        // Captures precede the alias's own telescope, so an own value domain
        // can refer to a parent Type input. Close over dependencies of captured
        // domains as well as references in predicates.
        let mut pending = domains.clone();
        let predicates: Vec<_> = requirements
            .iter()
            .cloned()
            .chain(capabilities.iter().cloned().map(n::TypePredicate::Trait))
            .collect();
        for predicate in &predicates {
            for term in requirement_terms(predicate, span)? {
                pending.push(self.copy_alias_type(term, span)?);
            }
        }
        let mut captures = BTreeMap::new();
        while let Some(term) = pending.pop() {
            let term = self.copy_alias_type(&term, span)?;
            for (id, sort) in term
                .rigid_parameters()
                .map_err(|e| error(span, e.to_string()))?
            {
                if parameters.contains(&id) || captures.contains_key(&id) {
                    continue;
                }
                let domain = self.trait_input_domain(id).unwrap_or_else(|| p(P::Type));
                let expected_sort = if domain == p(P::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                };
                if sort != expected_sort {
                    return Err(error(span, "alias capture domain has incompatible sort"));
                }
                pending.push(self.copy_alias_type(&domain, span)?);
                captures.insert(id, domain);
            }
        }
        let mut captured_parameters = Vec::new();
        let mut captured_domains = Vec::new();
        while !captures.is_empty() {
            self.required_steps = self
                .required_steps
                .checked_sub(captures.len() as u64)
                .ok_or_else(|| error(span, "alias capture ordering budget exhausted"))?;
            let next = captures
                .iter()
                .find_map(|(id, domain)| {
                    domain
                        .rigid_parameters()
                        .ok()
                        .filter(|dependencies| {
                            dependencies
                                .iter()
                                .all(|(dependency, _)| captured_parameters.contains(dependency))
                        })
                        .map(|_| *id)
                })
                .ok_or_else(|| error(span, "cyclic or unavailable alias capture domain"))?;
            captured_parameters.push(next);
            captured_domains.push(captures.remove(&next).unwrap());
        }
        captured_parameters.append(&mut parameters);
        captured_domains.append(&mut domains);
        parameters = captured_parameters;
        domains = captured_domains;
        self.alias_contracts.insert(
            declaration,
            nir::type_aliases::Contract {
                declaration,
                parameters: parameters.clone(),
                parameter_types: domains,
                requirements,
                capabilities,
            },
        );
        Ok(parameters)
    }

    pub(super) fn alias_value_expression(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: Vec<n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        self.alias_value_expression_with_guard(owner, value, arguments, span, true)
    }

    pub(super) fn alias_value_expression_with_guard(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: Vec<n::Expr>,
        span: Span,
        guarded: bool,
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
                    sort: self.input_binders(&[*parameter])[0].sort,
                }
            });
        }
        if arguments
            .iter()
            .all(|argument| matches!(argument.kind, K::TypeValue(_)))
        {
            self.check_type_application(owner, &terms, span)?;
            return self.alias_expression(owner, value, &terms, span);
        }
        let contract_parameters = self.ensure_alias_formation_contract(owner, span)?;
        let mut deferred: BTreeSet<_> = parameters
            .iter()
            .zip(&arguments)
            .filter(|(_, argument)| !matches!(argument.kind, K::TypeValue(_)))
            .map(|(id, _)| *id)
            .collect();
        deferred.extend(
            contract_parameters
                .iter()
                .filter(|id| {
                    self.alias_value_bindings.contains_key(id)
                        || self.normal_default_bindings.contains_key(id)
                })
                .copied(),
        );
        self.check_formation_application(
            owner,
            FormationInputs {
                terms: terms.clone(),
                deferred,
                environment: BTreeMap::new(),
            },
            span,
        )?;
        let diverges = arguments.iter().any(|argument| is_never(&argument.ty));
        let saved_bindings = self.alias_value_bindings.clone();
        let saved_normal_bindings = self.normal_default_bindings.clone();
        let result = self.with_alias_arguments(owner, &parameters, &terms, span, |checker| {
            let mut statements = Vec::new();
            let mut guard_inputs = BTreeMap::new();
            for (argument, parameter) in arguments.into_iter().zip(&parameters) {
                if matches!(argument.kind, K::TypeValue(_)) {
                    guard_inputs.insert(*parameter, argument);
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
                // A diverging input never initializes this local. Preserve the
                // declared domain for the unreachable alias body and guard.
                let binding_ty = if is_never(&argument.ty) {
                    checker
                        .trait_input_domain(*parameter)
                        .unwrap_or_else(|| p(P::Type))
                } else {
                    argument.ty.clone()
                };
                if binding_ty == p(P::Type) {
                    checker.alias_value_bindings.insert(*parameter, binding);
                } else {
                    checker
                        .normal_default_bindings
                        .insert(*parameter, (binding, binding_ty.clone()));
                }
                checker.locals.insert(binding, binding_ty.clone());
                guard_inputs.insert(
                    *parameter,
                    n::Expr {
                        source: argument.source,
                        ty: binding_ty,
                        kind: K::Local(binding),
                    },
                );
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
            let mut guard_arguments = Vec::new();
            for parameter in &contract_parameters {
                let input = if let Some(input) = guard_inputs.remove(parameter) {
                    input
                } else {
                    checker.captured_input_expression(*parameter, span)?
                };
                guard_arguments.push(input);
            }
            let saved_mode = std::mem::replace(&mut checker.in_required_comptime, true);
            let body = checker.expr(value, Some(&p(P::Type)));
            checker.in_required_comptime = saved_mode;
            let body = body?;
            let body = if guarded
                && checker.alias_contracts.contains_key(&declaration(
                    checker.package,
                    &checker.package_identities,
                    owner,
                )) {
                n::Expr {
                    source: source(span),
                    ty: body.ty.clone(),
                    kind: K::CheckTypeAlias {
                        declaration: declaration(
                            checker.package,
                            &checker.package_identities,
                            owner,
                        ),
                        arguments: guard_arguments
                            .into_iter()
                            .map(n::NormalFormArgument::Value)
                            .collect(),
                        preparations: vec![],
                        bindings: None,
                        order: None,
                        body: Some(Box::new(body)),
                    },
                }
            } else {
                body
            };
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
        self.normal_default_bindings = saved_normal_bindings;
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
        self.required_expression_with_inputs(expr, expected, None, Vec::new())
    }

    pub(super) fn required_normal_input(
        &mut self,
        source: &Expr<'_>,
        root: n::Expr,
        inputs: Vec<(n::LocalId, Term, Term)>,
    ) -> Result<(Value, Term), Error> {
        self.required_live_input(source, root, inputs, Vec::new())
    }

    fn required_expression_with_inputs(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        checked: Option<n::Expr>,
        inputs: Vec<(n::LocalId, Term, Term)>,
    ) -> Result<(Value, Term), Error> {
        let prepared = self.prepare_required_expression(expr, expected, checked, inputs, false)?;
        self.invoke_required_publishing(prepared, Vec::new(), expr.hir_id.owner, expr.span)
    }

    pub(super) fn invoke_required_publishing(
        &mut self,
        prepared: PreparedRequired,
        values: Vec<(n::LocalId, Value)>,
        owner: OwnerId,
        span: Span,
    ) -> Result<(Value, Term), Error> {
        let identity = declaration(self.package, &self.package_identities, owner).package;
        let (result, definitions) = prepared.invoke_publishing(
            values,
            &mut self.required_steps,
            span,
            identity,
            &mut self.next_definition_identity,
        )?;
        if definitions.structs.is_empty() && definitions.enums.is_empty() {
            return Ok(result);
        }
        for batch in definitions.structs {
            for definition in batch.iter() {
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let copied = (|| {
                    fuel = fuel
                        .checked_sub(
                            definition
                                .name
                                .len()
                                .saturating_add(definition.fields.len()),
                        )
                        .ok_or(())?;
                    for (name, ty) in &definition.fields {
                        fuel = fuel.checked_sub(name.len()).ok_or(())?;
                        nir::type_computations::charge_term(ty, &mut fuel).map_err(|_| ())?;
                    }
                    Ok::<_, ()>(())
                })();
                self.required_steps -= (before - fuel) as u64;
                copied
                    .map_err(|_| error(span, "generated struct registration budget exhausted"))?;
                self.context
                    .admit_nominal_constructor(definition.declaration);
                self.structs
                    .insert(definition.declaration, definition.clone());
            }
        }
        for batch in definitions.enums {
            for definition in batch.iter() {
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let copied = comptime_interpretor::EnumBuildRequest::charge_descriptor(
                    &definition.name,
                    &definition.groups,
                    &definition.variants,
                    definition.encoding.as_ref(),
                    definition.symbol_encoding.as_ref(),
                    &mut fuel,
                );
                self.required_steps -= (before - fuel) as u64;
                copied.map_err(|_| error(span, "generated enum registration budget exhausted"))?;
                self.context
                    .admit_nominal_constructor(definition.declaration);
                self.enums
                    .insert(definition.declaration, definition.clone());
            }
        }
        self.implementation_index.sync_nominal_layouts(
            self.structs.values(),
            self.enums.values(),
            self.newtypes.values(),
        );
        Ok(result)
    }

    pub(super) fn prepare_required_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        checked: Option<n::Expr>,
        inputs: Vec<(n::LocalId, Term, Term)>,
        defer_inputs: bool,
    ) -> Result<PreparedRequired, Error> {
        self.prepare_required_expression_with_live_inputs(
            expr,
            expected,
            checked,
            inputs,
            defer_inputs,
            &[],
        )
    }

    /// Complete an already checked source computation with invocation-local
    /// graph inputs. Preparation captures domains and dependencies, never values.
    pub(super) fn required_live_input(
        &mut self,
        source: &Expr<'_>,
        root: n::Expr,
        inputs: Vec<(n::LocalId, Term, Term)>,
        live: Vec<(n::LocalId, Term, Value)>,
    ) -> Result<(Value, Term), Error> {
        let prepared = self.prepare_required_expression_with_live_inputs(
            source,
            None,
            Some(root),
            inputs,
            false,
            &live,
        )?;
        let selected: BTreeSet<_> = prepared
            .inputs
            .iter()
            .map(|(binding, _, _)| *binding)
            .collect();
        self.invoke_required_publishing(
            prepared,
            live.into_iter()
                .filter(|(binding, _, _)| selected.contains(binding))
                .map(|(binding, _, value)| (binding, value))
                .collect(),
            source.hir_id.owner,
            source.span,
        )
    }

    pub(super) fn required_expression_with_live_inputs(
        &mut self,
        source: &Expr<'_>,
        expected: Option<&Term>,
        live: Vec<(n::LocalId, Term, Value)>,
    ) -> Result<(Value, Term), Error> {
        let prepared = self.prepare_required_expression_with_live_inputs(
            source,
            expected,
            None,
            Vec::new(),
            false,
            &live,
        )?;
        let selected: BTreeSet<_> = prepared
            .inputs
            .iter()
            .map(|(binding, _, _)| *binding)
            .collect();
        self.invoke_required_publishing(
            prepared,
            live.into_iter()
                .filter(|(binding, _, _)| selected.contains(binding))
                .map(|(binding, _, value)| (binding, value))
                .collect(),
            source.hir_id.owner,
            source.span,
        )
    }

    pub(super) fn prepare_required_expression_with_live_inputs(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        checked: Option<n::Expr>,
        inputs: Vec<(n::LocalId, Term, Term)>,
        defer_inputs: bool,
        live: &[(n::LocalId, Term, Value)],
    ) -> Result<PreparedRequired, Error> {
        let mut supplied: BTreeSet<_> = inputs.iter().map(|(binding, _, _)| *binding).collect();
        for (binding, _, _) in live {
            if !supplied.insert(*binding) {
                return Err(error(expr.span, "duplicate required comptime input"));
            }
        }
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
        let saved_value_locals = self.locals.clone();
        let saved_value_bindings = self.normal_default_bindings.clone();
        let result = (|| {
            let mut inputs = inputs;
            let mut abstract_value_parameters = Vec::new();
            let visible = self.visible_comptime_parameters(expr.span)?;
            for id in visible {
                let Some(domain) = self
                    .trait_input_domain(id)
                    .filter(|domain| *domain != p(P::Type))
                else {
                    continue;
                };
                // Generator clauses already have local identities. Reuse the
                // live binding when capturing an invocation instead of
                // treating its presence as evidence that a value was supplied.
                // Other prepared defaults may still be pending and must not
                // acquire an invented abstract value here.
                let existing = self
                    .normal_default_bindings
                    .get(&id)
                    .map(|(binding, _)| *binding);
                if let Some(binding) = existing {
                    if !self.generator_data.contains_key(&binding)
                        || !self.locals.contains_key(&binding)
                        || supplied.contains(&binding)
                    {
                        continue;
                    }
                }
                let domain = self.substituted_trait_domain(&domain, expr.span)?;
                let value = self
                    .alias_substitution
                    .get(&id)
                    .map(|value| (**value).clone())
                    .unwrap_or(Term::Rigid {
                        declaration: id,
                        sort: Sort::ComptimeValue,
                    });
                let binding = match existing {
                    Some(binding) => binding,
                    None => self.spread_binding(expr.span)?,
                };
                self.locals.insert(binding, domain.clone());
                self.normal_default_bindings
                    .insert(id, (binding, domain.clone()));
                if value
                    == (Term::Rigid {
                        declaration: id,
                        sort: Sort::ComptimeValue,
                    })
                {
                    abstract_value_parameters.push((id, domain.clone()));
                }
                inputs.push((binding, domain, value));
            }
            let mut root = checked
                .map(Ok)
                .unwrap_or_else(|| self.expr(expr, expected))
                .map_err(|mut e| {
                    if e.message == "local binding is unavailable in this scope" {
                        e.message =
                            "required comptime input is not yet available in this definition"
                                .into();
                    }
                    e
                })?;
            if let Some((parameters, arguments)) = self.initializer_substitution() {
                root = root
                    .instantiate_parameters(&self.input_binders(parameters), arguments)
                    .map_err(|e| error(expr.span, e.to_string()))?;
            }
            let result_type = root.ty.clone();
            for input in &self.specialization_inputs {
                if !inputs.iter().any(|(id, _, _)| *id == input.0) {
                    inputs.push(input.clone());
                }
            }
            let mut scoped_parameters = Vec::new();
            let mut scoped_arguments = Vec::new();
            let input_frames = self.call_input_frames();
            // A Type input can be transported without knowing its concrete
            // description. Use invocation-local rigid identities, then restore
            // the original telescope slots before returning to normalization.
            let type_packet = match &result_type {
                Term::Tuple(fields) => fields.iter().all(|ty| *ty == p(P::Type)),
                Term::NormalForm { head, .. } if Some(*head) == self.protocols.tuple => {
                    matches!(self.structural_type(&result_type, expr.span)?, Term::Tuple(fields)
                        if fields.iter().all(|ty| *ty == p(P::Type)))
                }
                _ => false,
            };
            if !defer_inputs
                && (result_type == p(P::Type)
                    || result_type == p(P::Bool)
                    || self.is_type_list_type(&result_type)
                    || type_packet)
                && self.signature_owner.is_some()
                && input_frames
                    .last()
                    .is_some_and(|frame| frame.contains(&Sort::Type))
            {
                let frame = input_frames.last().cloned().unwrap_or_default();
                self.required_steps = self
                    .required_steps
                    .checked_sub(frame.len() as u64)
                    .ok_or_else(|| {
                        error(expr.span, "scoped Type input preparation budget exhausted")
                    })?;
                let mut mapping = frame
                    .iter()
                    .enumerate()
                    .map(|(index, sort)| Term::Bound {
                        depth: 0,
                        index,
                        sort: *sort,
                    })
                    .collect::<Vec<_>>();
                let owner = self.default_input_owner.or(self.signature_owner).unwrap();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let free = nir::free_bindings::collect(&root, &mut fuel)
                    .map_err(|message| error(expr.span, message));
                self.required_steps -= (before - fuel) as u64;
                let free = free?;
                for (binding, symbol) in &self.type_input_bindings {
                    if binding.owner != owner.def_id.raw() {
                        continue;
                    }
                    let Term::Bound {
                        depth: 0,
                        index,
                        sort: Sort::Type,
                    } = symbol
                    else {
                        continue;
                    };
                    if frame.get(*index) != Some(&Sort::Type) {
                        continue;
                    }
                    let local = u32::try_from(*index)
                        .ok()
                        .filter(|index| *index != u32::MAX)
                        .ok_or_else(|| error(expr.span, "too many scoped Type inputs"))?;
                    let declaration = DeclarationId {
                        package: u64::MAX,
                        local,
                    };
                    mapping[*index] = Term::Rigid {
                        declaration,
                        sort: Sort::Type,
                    };
                    scoped_parameters.push(nir::DeclarationParameter {
                        declaration,
                        sort: Sort::Type,
                    });
                    scoped_arguments.push(symbol.clone());
                    if free.contains(binding) && !inputs.iter().any(|(id, _, _)| id == binding) {
                        inputs.push((*binding, p(P::Type), mapping[*index].clone()));
                    }
                }
                if !scoped_parameters.is_empty() {
                    let mut rebind = |ty: &Term| {
                        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                        let before = fuel;
                        let bounded = nir::type_computations::charge_term(ty, &mut fuel);
                        self.required_steps -= (before - fuel) as u64;
                        bounded.map_err(|e| error(expr.span, e.to_string()))?;
                        ty.rebind_scope(&mapping)
                            .map_err(|e| error(expr.span, e.to_string()))
                    };
                    root.rewrite_types(&mut rebind)?;
                    // Supplied environment values belong to the caller's frame
                    // too. Local-value binding and Type-slot binding are distinct:
                    // a supplied value must not replace the caller's Type identity.
                    for (_, ty, value) in &mut inputs {
                        *ty = rebind(ty)?;
                        *value = rebind(value)?;
                    }
                }
            }
            let mut functions = Vec::new();
            let mut runtime_function_identities = Vec::new();
            let mut live_methods = Vec::new();
            let mut members = BTreeSet::new();
            let mut expanded_members = BTreeSet::new();
            let mut associated = BTreeSet::new();
            let mut expanded_associated = BTreeSet::new();
            let mut queue = VecDeque::from_iter(references(&root, &mut members, &mut associated));
            // A frozen callable input is an execution dependency even when the
            // source computation refers to it through a local parameter.
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let free = nir::free_bindings::collect(&root, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            let free = free.map_err(|message| error(expr.span, message))?;
            queue.extend(frozen_references(
                inputs
                    .iter()
                    .filter(|(binding, _, _)| free.contains(binding))
                    .map(|(_, _, value)| value),
                &mut self.required_steps,
                expr.span,
            )?);
            for (binding, _, value) in live {
                if !free.contains(binding) {
                    continue;
                }
                let dependencies = live_references(
                    value,
                    &mut members,
                    &mut associated,
                    &mut live_methods,
                    &mut self.required_steps,
                    expr.span,
                )?;
                queue.extend(dependencies);
            }
            let mut seen = BTreeSet::new();
            loop {
                // Abstract associated values can carry runtime code identities.
                // Retain their declaration metadata without checking or executing
                // unrelated runtime function bodies in this comptime snapshot.
                let pending: Vec<_> = associated
                    .difference(&expanded_associated)
                    .copied()
                    .collect();
                for member in pending {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(
                            expr.span,
                            "required comptime associated dependency budget exhausted",
                        )
                    })?;
                    expanded_associated.insert(member);
                    // A generic comptime body may discover a member dependency
                    // that the enclosing initializer has not demanded yet.
                    let implementations: Vec<_> = self
                        .associated_initializers
                        .keys()
                        .filter(|(_, id)| *id == member)
                        .map(|(implementation, _)| *implementation)
                        .collect();
                    for implementation in implementations {
                        self.selected_associated_value(implementation, member, expr.span)?;
                    }
                    for implementation in &self.checked_implementations {
                        for (_, value) in implementation
                            .associated
                            .iter()
                            .filter(|(id, _)| *id == member)
                        {
                            queue.extend(references(value, &mut members, &mut associated));
                        }
                        for (_, initializer) in implementation
                            .associated_initializers
                            .iter()
                            .filter(|(id, _)| *id == member)
                        {
                            queue.extend(references(
                                initializer.expression(),
                                &mut members,
                                &mut associated,
                            ));
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
                    if !contract.signature.is_comptime_callable()
                        && !contract.program.preconditions.is_empty()
                        && !matches!(contract.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                            if parameters.iter().any(|p| p.sort != Sort::Runtime))
                    {
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
                    // An initializer can refer to another associated value
                    // without introducing a function edge in this iteration.
                    // Close both dependency sets before finishing the snapshot.
                    if !associated.is_subset(&expanded_associated) {
                        continue;
                    }
                    break;
                };
                if let Some(member) = self.checked_traits.iter().find_map(|interface| {
                    interface
                        .methods
                        .iter()
                        .find(|member| member.declaration == id)
                }) {
                    // Repeated packet references still name a contract, not a
                    // function body. Only an inherited default can reuse that
                    // identity as an executable dependency after expansion.
                    if !expanded_members.contains(&id) || member.default != Some(id) {
                        members.insert(id);
                        continue;
                    }
                }
                if !seen.insert(id) {
                    continue;
                }
                if seen.len() > 256 {
                    return Err(error(
                        expr.span,
                        "required comptime dependency budget exhausted",
                    ));
                }
                if let Some(function) = self
                    .generated_functions
                    .iter()
                    .find(|function| function.declaration == id)
                    .cloned()
                {
                    if function.phase == n::FunctionPhase::Runtime {
                        if !function.preconditions.is_empty()
                            && !matches!(&function.signature, Term::Function { parameters, .. }
                                if parameters.iter().any(|p| p.sort != Sort::Runtime))
                        {
                            return Err(error(
                                expr.span,
                                "function value would erase program preconditions; callable program contracts are not implemented",
                            ));
                        }
                        runtime_function_identities.push(n::RuntimeFunctionIdentity {
                            declaration: id,
                            type_parameters: function.type_parameters,
                            signature: function
                                .signature
                                .with_unsafe_call_requirement(function.is_unsafe),
                        });
                    } else {
                        queue.extend(function_references(
                            &function,
                            &mut members,
                            &mut associated,
                        ));
                        functions.push(function);
                    }
                    continue;
                }
                let owner = self
                    .package
                    .owners()
                    .find_map(|(o, _)| {
                        (matches_declaration(self.package, &self.package_identities, o, id))
                            .then_some(o)
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
                    let signature = self.known_callable_signature(owner)
                        .ok_or_else(|| error(expr.span, "runtime function identity requires a known signature"))?;
                    if self.program_requirements.get(&owner).is_some_and(|requirements| !requirements.is_empty())
                        && !matches!(&signature, Term::Function { parameters, .. }
                            if parameters.iter().any(|p| p.sort != Sort::Runtime)) {
                        return Err(error(expr.span, "function value would erase program preconditions; callable program contracts are not implemented"));
                    }
                    runtime_function_identities.push(n::RuntimeFunctionIdentity {
                        declaration: id,
                        type_parameters: self.type_parameters[&owner].clone(),
                        signature: self.callable_type(owner, signature),
                    });
                    continue;
                }
                let function = self.required_function(owner, expr.span)?;
                queue.extend(function_references(
                    &function,
                    &mut members,
                    &mut associated,
                ));
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
                    // Later staged binders may classify requires operands as
                    // type propositions. Do not cache the enclosing contract
                    // before those binders have entered its source telescope.
                    let staged_pending = scope.is_some_and(|scope| {
                        self.active_signatures.contains(&scope)
                            && self
                                .package
                                .item(scope)
                                .is_some_and(|item| match &item.kind {
                                    ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => {
                                        let ready =
                                            self.signature_domains.get(&scope).map_or(0, Vec::len);
                                        sig.params
                                            .iter()
                                            .skip(ready)
                                            .any(|parameter| parameter.ty.is_comptime())
                                    }
                                    _ => false,
                                })
                    });
                    let premise_owner = if assume_requirements && !staged_pending {
                        scope
                    } else {
                        None
                    };
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
                        Some(o) if premise_owner == Some(o) => self.definition_context(o),
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
            if !self.interfaces_ready
                && self.pending_refinements.iter().any(|pending| {
                    let id = declaration(self.package, &self.package_identities, pending.owner);
                    seen.contains(&id) || members.contains(&id)
                })
            {
                return Err(error(
                    expr.span,
                    "required comptime evaluation is waiting for a signature type refinement",
                ));
            }
            let entry = DeclarationId {
                package: u64::MAX,
                local: u32::MAX,
            };
            let mut entry_fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = entry_fuel;
            // Retain the source telescope, including unused runtime slots:
            // type annotations refer to these original positions. Capture
            // compaction will reject a runtime read and prune unused inputs.
            // Completed caller values and pending staged inputs are distinct.
            let mut available = Vec::new();
            if let Some(owner) = self
                .signature_owner
                .filter(|owner| self.active_signatures.contains(owner))
            {
                if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                    self.package.item(owner).map(|item| &item.kind)
                {
                    for (parameter, domain) in signature
                        .params
                        .iter()
                        .zip(self.signature_domains.get(&owner).into_iter().flatten())
                    {
                        let binding = local(parameter.hir_id);
                        let supplied = inputs
                            .iter()
                            .position(|(id, _, _)| *id == binding)
                            .map(|index| inputs.remove(index));
                        let (ty, value) = match supplied {
                            Some((_, ty, value)) => (Some(ty), Some(value)),
                            None => (domain.1.clone(), None),
                        };
                        let sort = if signature.modifiers.is_comptime && domain.0 == Sort::Runtime {
                            if ty.as_ref() == Some(&p(P::Type)) {
                                Sort::Type
                            } else {
                                Sort::ComptimeValue
                            }
                        } else {
                            domain.0
                        };
                        available.push((binding, (sort, ty), value));
                    }
                }
            }
            for (binding, ty, value) in inputs {
                available.push((
                    binding,
                    (
                        if ty == p(P::Type) {
                            Sort::Type
                        } else {
                            Sort::ComptimeValue
                        },
                        Some(ty),
                    ),
                    Some(value),
                ));
            }
            for (binding, ty, _) in live {
                if let Some((_, (_, domain), value)) =
                    available.iter_mut().find(|(id, _, _)| id == binding)
                {
                    if value.is_some() {
                        return Err(error(expr.span, "duplicate required comptime input"));
                    }
                    if let Some(expected) = domain.as_ref() {
                        self.require_expression(ty, expected, expr.span)?;
                    }
                    *domain = Some(ty.clone());
                } else {
                    available.push((
                        *binding,
                        (
                            if *ty == p(P::Type) {
                                Sort::Type
                            } else {
                                Sort::ComptimeValue
                            },
                            Some(ty.clone()),
                        ),
                        None,
                    ));
                }
            }
            let captured = nir::type_computations::capture_sparse_entry(
                entry,
                "<required-comptime>".into(),
                root,
                &available
                    .iter()
                    .map(|(binding, parameter, _)| (*binding, parameter.0, parameter.1.clone()))
                    .collect::<Vec<_>>(),
                &mut entry_fuel,
            );
            self.required_steps -= (before - entry_fuel) as u64;
            let captured=captured.map_err(|failure| error(expr.span, match failure {
                nir::type_computations::Error::MissingCapture(binding) if self.required_runtime_parameter(binding) =>
                    "required comptime computation cannot read a runtime parameter; defer does not change the parameter's phase".into(),
                nir::type_computations::Error::MissingCapture(binding) =>
                    format!("required comptime input {binding} is not available in this invocation"),
                other => other.to_string(),
            }))?;
            let selected = captured.slots.iter().copied().collect::<BTreeSet<_>>();
            let inputs: Vec<(n::LocalId, Term, Option<Term>)> = available
                .into_iter()
                .enumerate()
                .filter_map(|(slot, (binding, parameter, value))| {
                    selected.contains(&slot).then(|| {
                        (
                            binding,
                            parameter.1.expect("captured domain checked"),
                            value,
                        )
                    })
                })
                .collect();
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
                definition_identity_cursor: self.next_definition_identity,
                target: self.target,
                runtime_function_identities,
                traits,
                implementations,
                normal_forms: self.normal_form_metadata(),
                function_parameters: self.function_parameter_metadata(),
                functions,
                structs: self.structs.values().cloned().collect(),
                newtypes: self.newtypes.values().cloned().collect(),
                alias_contracts: self.alias_contracts.values().cloned().collect(),
                enums: self.enums.values().cloned().collect(),
                ..Default::default()
            };
            crate::verification::contracts::inherit(&mut package, &mut self.next_temporary_binding)
                .map_err(|mut failure| {
                    failure.source = failure
                        .source
                        .original(&self.expansion_sources)
                        .unwrap_or(failure.source);
                    error(expr.span, failure.to_string())
                })?;
            let invocation_assumptions =
                if let Some(implementation) = self.initializer_implementation() {
                    crate::capabilities::capability_sources(
                        &package.traits,
                        &implementation.capabilities,
                        crate::ptc::Budget::default(),
                    )
                    .map_err(|message| error(expr.span, message))?
                } else {
                    vec![]
                };
            // The snapshot contains demanded layouts only. Nominal identities
            // are admitted from source declarations even when no value/layout
            // of that type is needed by this comptime invocation.
            let type_context = if self.initializer_implementation().is_some() {
                self.context.clone()
            } else {
                self.nominal_context()
            };
            package.dispatch_instances = crate::materialization::dispatch_instances_in_context(
                &package,
                &type_context,
                &invocation_assumptions,
                &live_methods,
                crate::ptc::Budget::default(),
            )
            .map_err(|failure| {
                error(
                    expr.span,
                    format!("required comptime dispatch preparation failed: {failure}"),
                )
            })?;
            let mut abstract_parameters = self
                .signature_owner
                .and_then(|owner| self.type_parameters.get(&owner))
                .cloned()
                .unwrap_or_default();
            abstract_parameters.retain(|id| {
                self.trait_input_domain(*id)
                    .is_none_or(|domain| domain == p(P::Type))
            });
            abstract_value_parameters.retain(|(id, _)| {
                inputs.iter().any(|(_, _, value)| {
                    value
                        .as_ref()
                        .is_some_and(|value| value.mentions_rigid(*id))
                })
            });
            abstract_parameters.extend(
                scoped_parameters
                    .iter()
                    .map(|parameter| parameter.declaration),
            );
            // A specialized declaration can receive an opaque Type from its
            // generic caller. Retain that caller binder during invocation;
            // the callee's original binder no longer names the supplied Type.
            // Captured invocation values can carry these same opaque binders
            // inside callable instances or closure snapshot arguments.
            for argument in self
                .alias_substitution
                .values()
                .map(std::sync::Arc::as_ref)
                .chain(
                    inputs
                        .iter()
                        .flat_map(|(_, ty, value)| std::iter::once(ty).chain(value.iter())),
                )
            {
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let bounded = nir::type_computations::charge_term(argument, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                bounded.map_err(|failure| error(expr.span, failure.to_string()))?;
                for (id, sort) in argument
                    .rigid_parameters()
                    .map_err(|failure| error(expr.span, failure.to_string()))?
                {
                    if sort == Sort::Type && !abstract_parameters.contains(&id) {
                        abstract_parameters.push(id);
                    } else if sort == Sort::ComptimeValue
                        && !abstract_value_parameters
                            .iter()
                            .any(|(known, _)| *known == id)
                    {
                        // Substitution can forward a caller's value binder
                        // under a different callee declaration. Its domain
                        // evidence belongs to that caller, not to the replaced
                        // binder. Keep it opaque during required invocation.
                        let domain = self.trait_input_domain(id).ok_or_else(|| {
                            error(
                                expr.span,
                                "forwarded value input lacks declaration domain evidence",
                            )
                        })?;
                        abstract_value_parameters.push((id, domain));
                    }
                }
            }
            for (binding, _, value) in live {
                if !free.contains(binding) {
                    continue;
                }
                value
                    .visit_owned_graph(&mut self.required_steps, &mut |value, fuel| {
                        if let Value::TypeParameter(id) = value {
                            if !abstract_parameters.contains(id) {
                                abstract_parameters.push(*id);
                            }
                        }
                        let mut types: Vec<&Term> = match value {
                            Value::Type(ty)
                            | Value::List { ty, .. }
                            | Value::Newtype { ty, .. }
                            | Value::Optional { ty, .. }
                            | Value::ErrorResult { ty, .. } => vec![ty],
                            Value::TypeList(types) => types.iter().collect(),
                            Value::Closure(closure) => {
                                let mut types = vec![closure.ty()];
                                if let Some((_, arguments)) = closure.declaration_instance() {
                                    types.extend(arguments);
                                }
                                types
                            }
                            _ => vec![],
                        };
                        for ty in types.drain(..) {
                            let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
                            let before = remaining;
                            let bounded = nir::type_computations::charge_term(ty, &mut remaining);
                            *fuel -= (before - remaining) as u64;
                            bounded.map_err(|_| comptime_interpretor::ErrorKind::StepsExhausted)?;
                            for (id, sort) in ty
                                .rigid_parameters()
                                .map_err(|_| comptime_interpretor::ErrorKind::Type)?
                            {
                                if sort == Sort::Type && !abstract_parameters.contains(&id) {
                                    abstract_parameters.push(id);
                                }
                            }
                        }
                        Ok(())
                    })
                    .map_err(|failure| {
                        error(
                            expr.span,
                            format!("live input Type scope failed: {failure:?}"),
                        )
                    })?;
            }
            // Equality observations of opaque caller Type inputs need their
            // declaration facts. Dispatch preparation above still uses its
            // own world; these facts are supplied only to this invocation.
            let invocation_type_context = if abstract_parameters.is_empty() {
                &type_context
            } else {
                &self.context
            };
            crate::verification::flow::validate_machine_operations(&package)
                .map_err(|mut failure| {
                    failure.source = failure.source.original(&self.expansion_sources).unwrap_or(failure.source);
                    if matches!(failure.reason, crate::verification::flow::Reason::Phase("runtime function is unavailable during comptime execution")) {
                        error(expr.span, format!("required comptime evaluation cannot execute a runtime function or normal-form head: {failure}"))
                    } else { error(expr.span, failure.to_string()) }
                })?;
            let implementation_index = prepared::implementation_index(&package, expr.span)?;
            let prepared =
                PreparedRequired {
                    copy_authority: self.protocols.copy.zip(self.protocols.drop).map(
                        |(copy, drop)| {
                            let resources = n::Package {
                                target: self.target,
                                traits: self.checked_traits.clone(),
                                implementations: self.checked_implementations.clone(),
                                normal_forms: package.normal_forms.clone(),
                                structs: package.structs.clone(),
                                newtypes: package.newtypes.clone(),
                                alias_contracts: package.alias_contracts.clone(),
                                enums: package.enums.clone(),
                                ..Default::default()
                            };
                            (copy, drop, std::sync::Arc::new(resources))
                        },
                    ),
                    package,
                    implementation_index,
                    entry,
                    inputs,
                    result_type,
                    scoped_parameters,
                    scoped_arguments,
                    input_frames,
                    invocation_assumptions,
                    abstract_parameters,
                    abstract_value_parameters,
                    context: invocation_type_context.clone(),
                    in_signature: self
                        .signature_owner
                        .is_some_and(|owner| self.active_signatures.contains(&owner)),
                };
            Ok(prepared)
        })();
        let pending = std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
        if let Some(uses) = &mut self.pending_type_uses {
            uses.extend(pending);
        }
        self.locals = saved_value_locals;
        self.normal_default_bindings = saved_value_bindings;
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

    pub(super) fn required_function(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<n::Function, Error> {
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
                inline: sig.modifiers.is_inline,
                pure: sig.modifiers.is_pure,
                is_unsafe: sig.modifiers.is_unsafe,
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

// Callable identities are execution dependencies wherever they occur in a
// completed aggregate. Data types are not unfolded and no factory is invoked.
pub(super) fn frozen_references<'a>(
    values: impl IntoIterator<Item = &'a Term>,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    let mut declarations = BTreeSet::new();
    for value in values {
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let bounded = nir::type_computations::charge_term(value, &mut fuel);
        *steps -= (before - fuel) as u64;
        bounded.map_err(|failure| error(span, failure.to_string()))?;
        let mut pending = vec![value];
        while let Some(value) = pending.pop() {
            match value {
                Term::FunctionValue {
                    declaration,
                    arguments,
                } => {
                    declarations.insert(*declaration);
                    pending.extend(arguments);
                }
                Term::ClosureSnapshot { factory, arguments } => {
                    declarations.insert(*factory);
                    pending.extend(arguments);
                }
                Term::TupleValue(fields) => pending.extend(fields),
                Term::RecordValue(fields) => pending.extend(fields.iter().map(|(_, value)| value)),
                Term::OptionalValue { ty, value } => {
                    pending.push(ty);
                    if let Some(value) = value {
                        pending.push(value);
                    }
                }
                Term::ErrorValue { ty, label, value } => {
                    pending.push(ty);
                    if let Some(label) = label {
                        pending.push(label);
                    }
                    pending.push(value);
                }
                Term::NominalValue { fields, .. } => pending.extend(fields),
                _ => {}
            }
        }
    }
    Ok(declarations)
}

// The traversal includes guards and pattern computations; a function hidden in
// one of those positions must not escape phase/dependency checks.
fn live_references(
    value: &Value,
    members: &mut BTreeSet<DeclarationId>,
    associated: &mut BTreeSet<DeclarationId>,
    live_methods: &mut Vec<comptime_interpretor::TraitMethodIdentity>,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    let mut found = BTreeSet::new();
    value
        .visit_owned_graph(steps, &mut |value, fuel| {
            match value {
                Value::Function(id)
                | Value::RuntimeFunction {
                    declaration: id, ..
                } => {
                    found.insert(*id);
                }
                Value::RuntimeTraitMethod(method) => {
                    members.insert(method.member);
                    for term in std::iter::once(&method.subject)
                        .chain(&method.interface.arguments)
                        .chain(&method.method_arguments)
                    {
                        let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
                        let mut remaining = before;
                        let checked = nir::type_computations::charge_term(term, &mut remaining);
                        *fuel -= (before - remaining) as u64;
                        checked.map_err(|_| comptime_interpretor::ErrorKind::StepsExhausted)?;
                    }
                    live_methods.push((**method).clone());
                }
                Value::Closure(closure) => {
                    if let Some((declaration, _)) = closure.declaration_instance() {
                        found.insert(declaration);
                    }
                    constants::charge_copy(closure.body(), fuel, span)
                        .map_err(|_| comptime_interpretor::ErrorKind::StepsExhausted)?;
                    found.extend(references(closure.body(), members, associated));
                }
                _ => {}
            }
            Ok(())
        })
        .map_err(|failure| {
            error(
                span,
                format!("required comptime graph dependency inspection failed: {failure:?}"),
            )
        })?;
    Ok(found)
}

fn function_references(
    function: &n::Function,
    members: &mut BTreeSet<DeclarationId>,
    associated: &mut BTreeSet<DeclarationId>,
) -> BTreeSet<DeclarationId> {
    // Every expression retained in the snapshot needs its dependencies, even
    // when it occurs only in a contract. Source function defaults are already
    // moved into the call plans that use them by required_function.
    std::iter::once(&function.body)
        .chain(&function.preconditions)
        .chain(&function.postconditions)
        .chain(function.defaults.iter().map(|(_, value)| value))
        .flat_map(|value| references(value, members, associated))
        .collect()
}

pub(super) fn references(
    root: &n::Expr,
    members: &mut BTreeSet<DeclarationId>,
    associated: &mut BTreeSet<DeclarationId>,
) -> BTreeSet<DeclarationId> {
    enum Node<'a> {
        Expr(&'a n::Expr),
        Pattern(&'a n::Pattern),
    }
    let mut pending = vec![Node::Expr(root)];
    let mut output = BTreeSet::new();
    while let Some(node) = pending.pop() {
        match node {
            Node::Pattern(p) => match p {
                n::Pattern::CodeEvidence { .. } => {}
                n::Pattern::Then { first, value, next } => {
                    pending.push(Node::Pattern(first));
                    pending.push(Node::Expr(value));
                    pending.push(Node::Pattern(next));
                }
                n::Pattern::Guard { pattern, condition } => {
                    pending.push(Node::Pattern(pattern));
                    pending.push(Node::Expr(condition));
                }
                n::Pattern::OptionalPresent(pattern)
                | n::Pattern::ErrorResult { inner: pattern, .. }
                | n::Pattern::Observe { inner: pattern, .. }
                | n::Pattern::Remap { pattern, .. }
                | n::Pattern::Binding { inner: pattern, .. } => {
                    pending.push(Node::Pattern(pattern))
                }
                n::Pattern::NominalType { fields, .. }
                | n::Pattern::Record { fields, .. }
                | n::Pattern::AnonymousRecord { fields, .. }
                | n::Pattern::Variant { fields, .. }
                | n::Pattern::NormalForm {
                    arguments: fields, ..
                }
                | n::Pattern::TypeTuple(fields)
                | n::Pattern::Tuple(fields)
                | n::Pattern::Or(fields) => pending.extend(fields.iter().map(Node::Pattern)),
                n::Pattern::OptionalAbsent
                | n::Pattern::Wild
                | n::Pattern::Unit
                | n::Pattern::Literal(_) => {}
            },
            Node::Expr(e) => match &e.kind {
                K::EffectHandle {
                    computation,
                    handlers,
                    ..
                } => {
                    pending.push(Node::Expr(computation));
                    for handler in handlers {
                        for arm in &handler.arms {
                            pending.push(Node::Pattern(&arm.pattern));
                            if let Some(g) = &arm.guard {
                                pending.push(Node::Expr(g));
                            }
                            pending.push(Node::Expr(&arm.body));
                        }
                    }
                }
                K::EffectResume(body)
                | K::EffectComputation { body, .. }
                | K::EffectRun {
                    computation: body, ..
                } => pending.push(Node::Expr(body)),
                K::EffectRequest { arguments, .. } => {
                    pending.extend(arguments.iter().map(Node::Expr))
                }
                K::ComptimeFunctionInstance {
                    declaration,
                    subject,
                } => {
                    output.insert(*declaration);
                    pending.push(Node::Expr(subject));
                }
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
                K::NormalFormType {
                    arguments, plan, ..
                } => {
                    pending.extend(
                        plan.iter()
                            .flat_map(|p| &p.preparations)
                            .map(|p| Node::Expr(&p.expression)),
                    );
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
                K::NominalType {
                    arguments,
                    preparations,
                    ..
                } => {
                    pending.extend(preparations.iter().map(|p| Node::Expr(&p.expression)));
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
                K::CheckTypeAlias {
                    arguments,
                    body,
                    preparations,
                    ..
                } => {
                    pending.extend(preparations.iter().map(|p| Node::Expr(&p.expression)));
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
                    if let Some(body) = body {
                        pending.push(Node::Expr(body));
                    }
                }
                K::FunctionType {
                    parameters: fields, ..
                }
                | K::TypeList(fields)
                | K::TupleType(fields)
                | K::Intrinsic { inputs: fields, .. }
                | K::Tuple(fields)
                | K::Object { fields, .. }
                | K::AnonymousRecord { fields }
                | K::Record { fields, .. }
                | K::Variant { fields, .. } => pending.extend(fields.iter().map(Node::Expr)),
                K::AnonymousField { value, .. }
                | K::Field { value, .. }
                | K::Let { value, .. }
                | K::Assign { value, .. }
                | K::BitAssign { value, .. }
                | K::UnsafeScope(value)
                | K::Discard(value)
                | K::RequirePredicate(value)
                | K::EnsurePredicate(value)
                | K::CheckPredicate(value)
                | K::BitSlice { value, .. }
                | K::BitConvert(value)
                | K::ErrorConvert { value: value, .. }
                | K::NewtypeConvert(value)
                | K::Snapshot(value)
                | K::TakeBits(value)
                | K::TextQuery { subject: value, .. }
                | K::IntegerRepresentation(value)
                | K::SymbolName(value)
                | K::EnumName(value)
                | K::Not(value)
                | K::IntegerNegate(value)
                | K::FloatNegate(value)
                | K::AddressOf(value)
                | K::ReadPointer(value)
                | K::PointerAnonymousField { pointer: value, .. }
                | K::PointerField { pointer: value, .. }
                | K::OptionalPresent(value)
                | K::OptionalType(value)
                | K::PointerType(value)
                | K::ListLength(value)
                | K::StructFieldTypes(value)
                | K::BuildStruct(value)
                | K::BuildMetadata {
                    selector: value, ..
                }
                | K::BuildEnum(value)
                | K::EnumStructure { subject: value, .. }
                | K::ObjectQuery { subject: value, .. }
                | K::ObjectEdit { subject: value, .. }
                | K::StructFieldNames(value)
                | K::BitVecType(value)
                | K::Loop { body: value, .. } => pending.push(Node::Expr(value)),
                K::ScalarLess(a, b)
                | K::ScalarGreater(a, b)
                | K::ScalarGreaterEqual(a, b)
                | K::ScalarLessEqual(a, b)
                | K::TemplateEqual(a, b)
                | K::ScalarEqual(a, b)
                | K::TypeEqual(a, b)
                | K::ListConcat(a, b)
                | K::TypeListIndex(a, b)
                | K::BitEqual(a, b)
                | K::BitConcat(a, b)
                | K::BitIndex(a, b)
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
                K::BitWritePointer {
                    pointer,
                    selections,
                    value,
                } => {
                    pending.push(Node::Expr(pointer));
                    pending.extend(
                        selections
                            .iter()
                            .flat_map(n::BitSelection::expressions)
                            .map(Node::Expr),
                    );
                    pending.push(Node::Expr(value));
                }
                K::BitUpdate {
                    selections, value, ..
                } => {
                    pending.extend(
                        selections
                            .iter()
                            .flat_map(n::BitSelection::expressions)
                            .map(Node::Expr),
                    );
                    pending.push(Node::Expr(value));
                }
                K::BitWindow { value, start, end } => {
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
                K::AssociatedValue { member, .. } => {
                    associated.insert(*member);
                }
                K::ComptimeValue(_)
                | K::TypeValue(_)
                | K::TypeListValue(_)
                | K::Literal(_)
                | K::Local(_)
                | K::OptionalAbsent
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
pub(super) fn mentions_value_parameter(
    value: &Expr<'_>,
    depth: usize,
    is_type: &impl Fn(&Expr<'_>) -> bool,
) -> Result<bool, Error> {
    if depth >= 128 {
        return Err(error(
            value.span,
            "program precondition classification depth exhausted",
        ));
    }
    Ok(match &value.kind {
        H::ResolvedFnParam(..) | H::ResolvedClauseParam(..) => !is_type(value),
        H::SelfValue => true,
        // Normal-form applications retain an opaque type identity even when
        // indexed by comptime values. Their formation is checked on the type
        // path; those indices do not turn a trait bound into a program test.
        H::NFApplication(..) => false,
        H::Application(callee, arguments) | H::ExtendedApplication(callee, arguments) => {
            let mut found = mentions_value_parameter(callee, depth + 1, is_type)?;
            for argument in *arguments {
                let (Arg::Positional(value)
                | Arg::Named(_, value)
                | Arg::Expand(value)
                | Arg::Implicit(value)
                | Arg::PostLambda(value)) = argument;
                found |= mentions_value_parameter(value, depth + 1, is_type)?;
            }
            found
        }
        H::Tuple(elements) | H::List(elements) => {
            let mut found = false;
            for element in *elements {
                found |= mentions_value_parameter(element, depth + 1, is_type)?;
            }
            found
        }
        H::Block(block) | H::UnsafeBlock(block) => {
            let mut found = false;
            for expression in block.stmts.iter().chain(block.expr) {
                found |= mentions_value_parameter(expression, depth + 1, is_type)?;
            }
            found
        }
        H::If(condition, block, otherwise) => {
            let mut found = mentions_value_parameter(condition, depth + 1, is_type)?;
            for expression in block.stmts.iter().chain(block.expr).chain(*otherwise) {
                found |= mentions_value_parameter(expression, depth + 1, is_type)?;
            }
            found
        }
        H::When(arms) => {
            let mut found = false;
            for arm in *arms {
                if let Some(condition) = arm.cond {
                    found |= mentions_value_parameter(condition, depth + 1, is_type)?;
                }
                found |= mentions_value_parameter(arm.body, depth + 1, is_type)?;
            }
            found
        }
        H::Match(subject, arms) => {
            let mut found = mentions_value_parameter(subject, depth + 1, is_type)?;
            for arm in *arms {
                found |= pattern_mentions_value_parameter(&arm.pat, depth + 1, is_type)?;
                if let Some(guard) = arm.guard {
                    found |= mentions_value_parameter(guard, depth + 1, is_type)?;
                }
                found |= mentions_value_parameter(arm.body, depth + 1, is_type)?;
            }
            found
        }
        H::Let(binding) => {
            let mut found = pattern_mentions_value_parameter(&binding.pattern, depth + 1, is_type)?;
            for expression in binding
                .init
                .into_iter()
                .chain(binding.guard)
                .chain(binding.otherwise)
            {
                found |= mentions_value_parameter(expression, depth + 1, is_type)?;
            }
            found
        }
        H::RefinedType { requirement, base } => {
            mentions_value_parameter(requirement, depth + 1, is_type)?
                || mentions_value_parameter(base, depth + 1, is_type)?
        }
        H::Binary(_, a, b) | H::Index(a, b) | H::Cast(a, b) | H::TraitBound(a, b) => {
            mentions_value_parameter(a, depth + 1, is_type)?
                || mentions_value_parameter(b, depth + 1, is_type)?
        }
        H::TakeOld(value)
        | H::Deref(value)
        | H::Ref(value)
        | H::MultiDeref(value)
        | H::TakeBitVec(value)
        | H::TakeTagName(value)
        | H::TakeIntrinsic(value)
        | H::TakeTupleEnumerate(value)
        | H::Defer(value)
        | H::InlineControl(value)
        | H::Unary(_, value)
        | H::Projection(value, _)
        | H::LiteralConversion { literal: value, .. } => {
            mentions_value_parameter(value, depth + 1, is_type)?
        }
        _ => false,
    })
}

fn pattern_mentions_value_parameter(
    pattern: &hir::pattern::Pattern<'_>,
    depth: usize,
    is_type: &impl Fn(&Expr<'_>) -> bool,
) -> Result<bool, Error> {
    if depth >= 128 {
        return Err(error(
            pattern.span,
            "program precondition classification depth exhausted",
        ));
    }
    use hir::pattern::PatternKind as P;
    let mut patterns = Vec::new();
    let mut expressions = Vec::new();
    match &pattern.kind {
        P::Then(first, value, next) => {
            patterns.extend([*first, *next]);
            expressions.push(*value);
        }
        P::Guard(inner, condition) => {
            patterns.push(*inner);
            expressions.push(*condition);
        }
        P::Const(value) | P::Comptime(value) => expressions.push(*value),
        P::Binding(_, _, inner) => patterns.extend(*inner),
        P::Projection(inner, _)
        | P::OptionSome(inner)
        | P::ErrorOk(inner)
        | P::ErrorErr(inner)
        | P::Not(inner)
        | P::Ref(inner) => patterns.push(*inner),
        P::Tuple(fields) | P::Or(fields) => patterns.extend(fields.iter()),
        P::List(fields, rest) => {
            patterns.extend(fields.iter());
            patterns.extend(*rest);
        }
        P::AppTuple(head, fields) => {
            patterns.push(*head);
            patterns.extend(fields.iter());
        }
        P::Struct(head, fields, _) | P::AppStruct(head, fields) => {
            patterns.push(*head);
            patterns.extend(fields.iter().map(|field| &field.pat));
        }
        P::NormalForm(head, arguments) => {
            expressions.push(*head);
            patterns.extend(arguments.iter().map(|argument| &argument.pattern));
        }
        P::Range(start, end, _) => {
            expressions.extend(*start);
            expressions.extend(*end);
        }
        P::BitVec { segments, .. } => {
            for segment in segments {
                match segment {
                    hir::pattern::BitSegment::Typed { pattern, ty } => {
                        patterns.push(*pattern);
                        expressions.push(*ty);
                    }
                    hir::pattern::BitSegment::Computed(value) => expressions.push(*value),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    for value in expressions {
        if mentions_value_parameter(value, depth + 1, is_type)? {
            return Ok(true);
        }
    }
    for pattern in patterns {
        if pattern_mentions_value_parameter(pattern, depth + 1, is_type)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// Only conjunction exposes individual positive capabilities. Do not extract
// assumptions from beneath negation or disjunction.
pub(super) fn signature_capability_atoms<'h>(
    value: &Expr<'h>,
    pending: &mut Vec<Expr<'h>>,
    depth: usize,
) -> Result<(), Error> {
    if depth >= 128 {
        return Err(error(value.span, "signature capability depth exhausted"));
    }
    match value.kind {
        H::Binary(BinOp::And, left, right) => {
            signature_capability_atoms(left, pending, depth + 1)?;
            signature_capability_atoms(right, pending, depth + 1)?;
        }
        H::TraitBound(subject, bound) => {
            if let H::Binary(BinOp::Add, left, right) = bound.kind {
                for bound in [left, right] {
                    let atom = Expr {
                        kind: H::TraitBound(subject, bound),
                        ..value.clone()
                    };
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
pub(super) fn signature_inputs_ready(
    value: &Expr<'_>,
    depth: usize,
    available: &impl Fn(hir::HirId) -> bool,
) -> Result<bool, Error> {
    signature_inputs_ready_with_clauses(value, depth, available, &|_| true)
}

pub(super) fn signature_inputs_ready_with_clauses(
    value: &Expr<'_>,
    depth: usize,
    available: &impl Fn(hir::HirId) -> bool,
    clause_available: &impl Fn(hir::common::DefId) -> bool,
) -> Result<bool, Error> {
    if depth >= 128 {
        return Err(error(
            value.span,
            "signature input dependency depth exhausted",
        ));
    }
    let ready =
        |value| signature_inputs_ready_with_clauses(value, depth + 1, available, clause_available);
    Ok(match &value.kind {
        H::ResolvedFnParam(_, id) => available(*id),
        H::ResolvedClauseParam(_, id) => clause_available(*id),
        H::Lit(_)
        | H::ResolvedDef { .. }
        | H::Builtin(_)
        | H::TyType
        | H::TyAny
        | H::TyVoid
        | H::TyNoReturn
        | H::TySelf
        | H::Unit => true,
        H::TraitBound(a, b)
        | H::Assign(a, b)
        | H::Binary(_, a, b)
        | H::Cast(a, b)
        | H::Index(a, b)
        | H::TyFnArrow(a, b)
        | H::TyClosure(a, b)
        | H::EffectQualifiedType(a, b)
        | H::ErrorQualifiedType(a, b) => ready(a)? & ready(b)?,
        H::Projection(value, _)
        | H::TyPtr(value)
        | H::TyOptional(value)
        | H::Unary(_, value)
        | H::LiteralConversion { literal: value, .. }
        | H::TypeOf(value) => ready(value)?,
        H::Semi(value) => ready(value)?,
        H::Let(binding) => {
            let mut all = signature_pattern_inputs_ready(
                &binding.pattern,
                depth + 1,
                available,
                clause_available,
            )?;
            for value in [binding.ty, binding.init, binding.guard, binding.otherwise]
                .into_iter()
                .flatten()
            {
                all &= ready(value)?;
            }
            all
        }
        H::Block(block) | H::UnsafeBlock(block) | H::Loop(block) => {
            let mut all = true;
            for statement in block.stmts {
                all &= ready(statement)?;
            }
            if let Some(tail) = block.expr {
                all &= ready(tail)?;
            }
            all
        }
        H::If(condition, yes, no) => {
            let mut all = ready(condition)?;
            for statement in yes.stmts {
                all &= ready(statement)?;
            }
            if let Some(tail) = yes.expr {
                all &= ready(tail)?;
            }
            if let Some(no) = no {
                all &= ready(no)?;
            }
            all
        }
        H::When(arms) => {
            let mut all = true;
            for arm in *arms {
                if let Some(condition) = arm.cond {
                    all &= ready(condition)?;
                }
                all &= ready(arm.body)?;
            }
            all
        }
        H::Match(subject, arms) => {
            let mut all = ready(subject)?;
            for arm in *arms {
                all &= signature_pattern_inputs_ready(
                    &arm.pat,
                    depth + 1,
                    available,
                    clause_available,
                )?;
                if let Some(guard) = arm.guard {
                    all &= ready(guard)?;
                }
                all &= ready(arm.body)?;
            }
            all
        }
        H::Tuple(values) | H::List(values) => {
            let mut all = true;
            for value in *values {
                all &= ready(value)?;
            }
            all
        }
        H::NFApplication(head, args)
        | H::Application(head, args)
        | H::ExtendedApplication(head, args) => {
            let mut all = ready(head)?;
            for arg in *args {
                let (Arg::Positional(value)
                | Arg::Named(_, value)
                | Arg::Expand(value)
                | Arg::Implicit(value)
                | Arg::PostLambda(value)) = arg;
                all &= ready(value)?;
            }
            all
        }
        H::TyFn(parameters)
        | H::TyComptimeFn(parameters)
        | H::TyModifiedFn(_, parameters)
        | H::TyNFFn(parameters) => {
            let mut all = true;
            for parameter in *parameters {
                match parameter.kind {
                    TyParamKind::Optional(_, ty, default) => {
                        all &= ready(ty)? & ready(default)?;
                    }
                    TyParamKind::Positional(ty)
                    | TyParamKind::PositionalDependencyCatched(_, ty)
                    | TyParamKind::Varadic(_, ty) => {
                        all &= ready(ty)?;
                    }
                    TyParamKind::Itself { .. } => {}
                }
            }
            all
        }
        _ => false,
    })
}

fn signature_pattern_inputs_ready(
    pattern: &hir::pattern::Pattern<'_>,
    depth: usize,
    available: &impl Fn(hir::HirId) -> bool,
    clause_available: &impl Fn(hir::common::DefId) -> bool,
) -> Result<bool, Error> {
    use hir::pattern::PatternKind as K;
    if depth >= 128 {
        return Err(error(
            pattern.span,
            "signature pattern dependency depth exhausted",
        ));
    }
    let pat = |value| signature_pattern_inputs_ready(value, depth + 1, available, clause_available);
    let expr =
        |value| signature_inputs_ready_with_clauses(value, depth + 1, available, clause_available);
    Ok(match &pattern.kind {
        K::Wild | K::OptionNull | K::Binding(_, _, None) => true,
        K::Const(value) | K::Comptime(value) => expr(value)?,
        K::Binding(_, _, Some(inner))
        | K::Projection(inner, _)
        | K::OptionSome(inner)
        | K::ErrorOk(inner)
        | K::ErrorErr(inner)
        | K::Not(inner)
        | K::Ref(inner) => pat(inner)?,
        K::Guard(inner, guard) => pat(inner)? & expr(guard)?,
        K::Then(first, view, second) => pat(first)? & expr(view)? & pat(second)?,
        K::Tuple(fields) | K::Or(fields) => {
            let mut all = true;
            for field in *fields {
                all &= pat(field)?;
            }
            all
        }
        K::List(fields, tail) => {
            let mut all = true;
            for field in *fields {
                all &= pat(field)?;
            }
            if let Some(tail) = tail {
                all &= pat(tail)?;
            }
            all
        }
        K::AppTuple(head, fields) => {
            let mut all = pat(head)?;
            for field in *fields {
                all &= pat(field)?;
            }
            all
        }
        K::Struct(head, fields, _) | K::AppStruct(head, fields) => {
            let mut all = pat(head)?;
            for field in *fields {
                all &= pat(&field.pat)?;
            }
            all
        }
        K::NormalForm(head, args) => {
            let mut all = expr(head)?;
            for argument in *args {
                all &= pat(&argument.pattern)?;
            }
            all
        }
        K::BitVec { segments, .. } => {
            let mut all = true;
            for segment in segments {
                match segment {
                    hir::pattern::BitSegment::Literal { .. } => {}
                    hir::pattern::BitSegment::Typed { pattern, ty } => {
                        all &= pat(pattern)? & expr(ty)?;
                    }
                    hir::pattern::BitSegment::Computed(value) => {
                        all &= expr(value)?;
                    }
                }
            }
            all
        }
        K::Range(start, end, _) => {
            let mut all = true;
            for value in [*start, *end].into_iter().flatten() {
                all &= expr(value)?;
            }
            all
        }
        K::CodeSplice(operand, _) => expr(operand)?,
        K::Async | K::Invalid => false,
    })
}

/// Own the synthetic subjects while atoms borrow their declaration identities.
pub(super) fn parameter_capability_subjects<'h>(
    parameters: &[hir::clause::ClauseParam<'h>],
) -> Result<Vec<(Expr<'h>, &'h Expr<'h>)>, Error> {
    let mut subjects = Vec::new();
    for parameter in parameters {
        if let hir::clause::ClauseParamKind::TraitBound(_, bound) = parameter.kind {
            let id = parameter
                .definition
                .ok_or_else(|| error(parameter.span, "missing clause parameter identity"))?;
            subjects.push((
                Expr {
                    hir_id: parameter.hir_id,
                    span: parameter.span,
                    kind: H::ResolvedClauseParam(parameter.name.name, id),
                },
                bound,
            ));
        }
    }
    Ok(subjects)
}

/// Predicate terms remain type propositions, including Type-valued ordinary inputs.
pub(super) fn requirement_terms<'a>(
    value: &'a n::TypePredicate,
    span: Span,
) -> Result<Vec<&'a Term>, Error> {
    let mut pending = vec![(value, 0usize)];
    let mut terms = Vec::new();
    let mut fuel = 10_000usize;
    while let Some((value, depth)) = pending.pop() {
        if depth >= 128 || fuel == 0 {
            return Err(error(span, "type requirement traversal budget exhausted"));
        }
        fuel -= 1;
        match value {
            n::TypePredicate::Bool(_) => {}
            n::TypePredicate::Trait(r) => {
                terms.push(&r.subject);
                terms.extend(&r.interface_arguments);
            }
            n::TypePredicate::Equal(a, b) => {
                terms.push(a);
                terms.push(b);
            }
            n::TypePredicate::Not(p) => pending.push((p, depth + 1)),
            n::TypePredicate::And(ps) | n::TypePredicate::Or(ps) => {
                pending.extend(ps.iter().map(|p| (p, depth + 1)))
            }
        }
    }
    Ok(terms)
}

pub(super) fn map_requirement(
    value: &n::TypePredicate,
    map: &mut impl FnMut(&Term) -> Result<Term, Error>,
    span: Span,
    depth: usize,
) -> Result<n::TypePredicate, Error> {
    use n::TypePredicate as Q;
    if depth >= 128 {
        return Err(error(span, "type requirement mapping depth exhausted"));
    }
    Ok(match value {
        Q::Bool(v) => Q::Bool(*v),
        Q::Trait(r) => Q::Trait(n::TraitRequirement {
            interface: r.interface,
            subject: map(&r.subject)?,
            interface_arguments: r
                .interface_arguments
                .iter()
                .map(&mut *map)
                .collect::<Result<_, _>>()?,
        }),
        Q::Equal(a, b) => Q::Equal(map(a)?, map(b)?),
        Q::Not(p) => Q::Not(Box::new(map_requirement(p, map, span, depth + 1)?)),
        Q::And(ps) | Q::Or(ps) => {
            let parts = ps
                .iter()
                .map(|p| map_requirement(p, map, span, depth + 1))
                .collect::<Result<_, _>>()?;
            if matches!(value, Q::And(_)) {
                Q::And(parts)
            } else {
                Q::Or(parts)
            }
        }
    })
}
