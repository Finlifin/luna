//! Preparation and invocation of required computations during type formation.
//! Open signature captures retain unavailable staged inputs explicitly. Completed
//! invocation uses ordinary NIR comptime semantics without unfolding type heads.
use super::*;
use comptime_executor::{Cache, Context, KeyProtocol, QueryId, Scope};
use comptime_interpretor::{Budget, Interpreter, Value};
use std::cell::RefCell;
use std::collections::VecDeque;

mod capture;
mod data;

pub(super) use data::{MemberSources, independent_member_data};

mod dependencies;
pub(super) mod inputs;
pub(super) mod prepared;
mod publication;
mod restore;
mod scope;
mod source_closure;
mod trait_world;

pub(super) use dependencies::{Dependency, Discovery, Member};
use prepared::PreparedRequired;

/// Required expression evaluation can prepare a distinct clause packet at the
/// same source node. Query identity includes the operation, not just its site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RootKind {
    AssociatedType,
    Expression,
    CallableInputs,
}

// Preparing code for the VM also checks unentered branches and callable bodies.
// Only carry entry evidence along edges reached before any preceding computation.
fn entry_call(mut expr: &Expr<'_>, steps: &mut u64) -> Result<Option<(hir::HirId, Span)>, Error> {
    loop {
        *steps = steps
            .checked_sub(1)
            .ok_or_else(|| error(expr.span, "required entry observation budget exhausted"))?;
        expr = match expr.kind {
            H::Application(..) | H::ExtendedApplication(..) => {
                return Ok(Some((expr.hir_id, expr.span)));
            }
            H::Block(block) | H::UnsafeBlock(block) => match block.stmts.first().or(block.expr) {
                Some(first) => first,
                None => return Ok(None),
            },
            H::Let(decl) => match decl.init {
                Some(value) => value,
                None => return Ok(None),
            },
            H::If(condition, _, _)
            | H::Match(condition, _)
            | H::Binary(_, condition, _)
            | H::Unary(_, condition) => condition,
            _ => return Ok(None),
        };
    }
}

/// A prepared operation owns its selection world and live inputs. Its source
/// owner forms requested declarations in their original lexical environments.
pub(super) struct Invocation {
    prepared: PreparedRequired,
    values: Vec<(n::LocalId, Value)>,
    package: u64,
    identities: crate::definitions::DefinitionIdentities,
    span: Span,
}

pub(super) struct CompletedInvocation {
    result: (Value, Term),
    definitions: crate::definitions::PublishedDefinitions,
    span: Span,
}

impl Invocation {
    pub(super) fn execute(
        self,
        checker: &mut Checker<'_, '_>,
    ) -> Result<CompletedInvocation, Error> {
        let mut steps = std::mem::take(&mut checker.required_steps);
        let span = self.span;
        let owner = checker.member_sources.owner.clone();
        let supply_attributes = checker.attribute_demands.owns(
            &checker.definition_attributes,
            &self.prepared.world.package().definition_attributes,
        );
        let prepared = match try_memoize_pure_invocation(
            self.prepared,
            &self.values,
            supply_attributes,
            &checker.protocols,
            &mut steps,
            span,
            self.package,
            &self.identities,
        )? {
            Ok((cached, definitions)) => {
                checker.required_steps = steps;
                return Ok(CompletedInvocation {
                    result: cached,
                    definitions,
                    span: self.span,
                });
            }
            Err(prepared) => prepared,
        };
        let checker = std::cell::RefCell::new(checker);
        let source_failure = std::cell::Cell::new(None);
        let lookup = |implementation, member, remaining: &mut u64| {
            let mut checker = checker.borrow_mut();
            std::mem::swap(remaining, &mut checker.required_steps);
            let value = checker.required_member_data(implementation, member);
            std::mem::swap(remaining, &mut checker.required_steps);
            value.map_err(|failure| {
                let source = Some(failure.source);
                source_failure.set(Some(failure));
                comptime_interpretor::Error {
                    source,
                    kind: comptime_interpretor::ErrorKind::AssociatedSourceFailure,
                }
            })
        };
        let attributes = |target, known: &dyn Fn(DeclarationId) -> bool, remaining: &mut u64| {
            let mut checker = checker.borrow_mut();
            std::mem::swap(remaining, &mut checker.required_steps);
            let result = checker
                .demand_definition_attributes(target)
                .and_then(|value| {
                    let checker = &mut **checker;
                    checker.attribute_demands.layouts.deliver(
                        value,
                        known,
                        &mut checker.required_steps,
                        span,
                    )
                });
            std::mem::swap(remaining, &mut checker.required_steps);
            result.map_err(|failure| {
                let source = Some(failure.source);
                source_failure.set(Some(failure));
                comptime_interpretor::Error {
                    source,
                    kind: comptime_interpretor::ErrorKind::PendingDefinitionAttributes(target),
                }
            })
        };
        let result = prepared.invoke_publishing(
            self.values,
            &mut steps,
            span,
            self.package,
            self.identities,
            Some(data::Reader {
                owner: &owner,
                attributes: supply_attributes.then_some(&attributes),
                form: &lookup,
            }),
        );
        checker.into_inner().required_steps = steps;
        if let Some(failure) = source_failure.take() {
            return Err(failure);
        }
        let (result, definitions) = result?;
        Ok(CompletedInvocation {
            result,
            definitions,
            span: self.span,
        })
    }
}

impl std::fmt::Display for Invocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(required-invocation (package {}) (source {}) (live-inputs",
            self.package,
            source(self.span)
        )?;
        for (binding, _) in &self.values {
            write!(f, " {binding}")?;
        }
        write!(f, ") {})", self.prepared)
    }
}

impl CompletedInvocation {
    /// Publish generated definitions in the receiving source world before the
    /// returned value or type can refer to them.
    pub(super) fn publish(self, checker: &mut Checker<'_, '_>) -> Result<(Value, Term), Error> {
        if checker.attribute_demands.tracks_publications() {
            checker.attribute_demands.layouts.charge_registration(
                &self.definitions,
                &mut checker.required_steps,
                self.span,
            )?;
        }
        publication::Receiver {
            context: &mut checker.context,
            declared_nominals: &mut checker.declared_theory,
            structs: &mut checker.structs,
            enums: &mut checker.enums,
            newtypes: &mut checker.newtypes,
            implementation_index: &mut checker.implementation_index,
        }
        .register(&self.definitions, &mut checker.required_steps, self.span)?;
        if checker.attribute_demands.tracks_publications() {
            checker
                .attribute_demands
                .layouts
                .register(&self.definitions);
        }
        Ok(self.result)
    }
}

impl Checker<'_, '_> {
    // A comptime parameter may acquire a Type domain from an explicit type
    // equality in this declaration. Only already available type binders and
    // positive equality conjuncts participate; program predicates never do.
    pub(super) fn signature_parameter_domain(
        &mut self,
        owner: OwnerId,
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
            let inputs = self.source_expression_inputs(owner, &atom)?;
            if !inputs
                .locals
                .iter()
                .all(|id| self.input_facts.type_identity(id).is_some())
                || self.program_requirement(&atom)?
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
            H::ResolvedFnParam(_, id) | H::ResolvedLocal(_, id) => {
                self.input_facts.type_identity(&local(id)).is_some()
            }
            H::ResolvedClauseParam(_, id) => self
                .trait_input_domain(DeclarationId {
                    package: semantic_package(&self.package_identities, id.pkg),
                    local: id.index,
                })
                .is_none_or(|domain| domain == p(P::Type)),
            _ => false,
        }
    }

    fn contract_parameter_is_ptc_integer(&self, value: &Expr<'_>) -> bool {
        match value.kind {
            H::ResolvedClauseParam(_, id) => {
                self.trait_input_domain(DeclarationId {
                    package: semantic_package(&self.package_identities, id.pkg),
                    local: id.index,
                }) == Some(p(P::Integer))
            }
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
                _ if checker.ptc_integer_comparison(value).is_some() => true,
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
                _ if checker.type_predicate_not_sized_call(value)?.is_some() => true,
                _ if checker.type_predicate_kind_call(value)?.is_some() => true,
                _ => false,
            })
        }
        Ok(mentions_value_parameter(value, 0, &|value| {
            self.contract_parameter_is_type(value) || self.contract_parameter_is_ptc_integer(value)
        })? || !type_shape(self, value, 0)?)
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
        let checked = self.in_declaration_scope(owner, |this| this.form_contract(owner, span));
        self.active_contracts.remove(&owner);
        checked
    }

    fn form_contract(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        self.checking_contract = true;
        let saved_pending = self.pending_type_uses.replace(Vec::new());
        let saved_capabilities = self.capabilities.get(&owner).cloned();
        // A required invocation can request this contract after a nested
        // signature check restored its caller's input environment. Rebuild
        // declaration inputs from checked domains, never from that caller.
        let domains = self
            .open_functions
            .get(&owner)
            .map(|definition| definition.input_domains())
            .or_else(|| {
                self.signatures
                    .get(&owner)
                    .or_else(|| self.interface_signatures.get(&owner))
                    .and_then(|signature| match signature.callable_signature() {
                        Term::Function { parameters, .. } => Some(
                            parameters
                                .iter()
                                .map(|p| (p.sort, Some(p.ty.clone())))
                                .collect(),
                        ),
                        _ => None,
                    })
            })
            .or_else(|| self.signature_domains.get(&owner).cloned());
        if let Some(domains) = domains {
            if let ItemKind::Fn(sig, _)
            | ItemKind::Case(sig, _)
            | ItemKind::FnDeclaration(sig)
            | ItemKind::Effect(sig, _) = &self.package.item(owner).unwrap().kind
            {
                for (index, (parameter, (sort, domain))) in
                    sig.params.iter().zip(domains).enumerate()
                {
                    if let Some(domain) = domain {
                        let binding = local(parameter.hir_id);
                        self.input_facts
                            .parameters
                            .bind(binding, index, sort, &domain);
                        self.locals.insert(binding, domain);
                    }
                }
            }
        }
        self.context = self.binder_context(owner);
        let result = (|| {
            let clauses = match &self.package.item(owner).unwrap().kind {
                ItemKind::Fn(sig, _)
                | ItemKind::Case(sig, _)
                | ItemKind::FnDeclaration(sig)
                | ItemKind::Effect(sig, _) => sig.clause_constraints,
                ItemKind::EffectGroup(group) => group.clause_constraints,
                ItemKind::TypeAlias(_, _, constraints) | ItemKind::Newtype(_, _, constraints) => {
                    *constraints
                }
                ItemKind::Struct(def) => def.clause_constraints,
                ItemKind::Enum(def) => def.clause_constraints,
                ItemKind::Impl(def) => def.clause_constraints,
                ItemKind::Deriver(def) => def.clause_constraints,
                ItemKind::Derive(req) => req.clause_constraints,
                ItemKind::Trait(def) => def.clause_constraints,
                ItemKind::Associated(def) => def.clause_constraints,
                // Lexical const initializers and explicit template-instance inputs
                // have no declaration clauses of their own. The selected
                // template contract is checked after preparing its inputs.
                ItemKind::Const(..)
                | ItemKind::Static(..)
                | ItemKind::TestTemplateInstance { .. } => &[],
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
            let type_result_binding = match &self.package.item(owner).unwrap().kind {
                ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig)
                    if !sig.modifiers.is_normal_form_head =>
                {
                    sig.return_binding.and_then(|binding| {
                        let returns_type = match self.signatures.get(&owner) {
                            Some(signature) => matches!(
                                signature.callable_signature(),
                                Term::Function { result, .. } if **result == p(P::Type)
                            ),
                            None => match sig.return_ty {
                                Some(ret_ty) if !matches!(ret_ty.kind, H::Defer(_)) => {
                                    self.completed_type(ret_ty)
                                        .ok()
                                        .and_then(|ct| ct.identity(ret_ty.span).ok())
                                        == Some(p(P::Type))
                                }
                                _ => false,
                            },
                        };
                        returns_type.then_some((binding, sig.span))
                    })
                }
                _ => None,
            };
            let mut program_requirements = Vec::new();
            let mut postconditions = Vec::new();
            let mut decreases_clauses = Vec::new();
            let mut result_binder: Option<(n::LocalId, DeclarationId)> = None;
            let mut result_guarantees = Vec::new();
            let clause_result = (|| -> Result<(), Error> {
                for clause in clauses {
                    if let hir::clause::ClauseConstraintKind::Ensures(value) = &clause.kind {
                        if matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig) if !sig.modifiers.is_normal_form_head)
                        {
                            if let Some((ret_hir, ret_span)) = type_result_binding {
                                let ret_local = local(ret_hir);
                                if result_binder.is_none() {
                                    let binder = self.fresh_type_binder(owner, ret_span)?;
                                    self.input_facts.types.insert(
                                        ret_local,
                                        Term::Rigid {
                                            declaration: binder.declaration,
                                            sort: binder.sort,
                                        },
                                    );
                                    self.input_facts.derived.insert(ret_local);
                                    self.locals.insert(ret_local, p(P::Type));
                                    result_binder = Some((ret_local, binder.declaration));
                                }
                                if !self.program_requirement(value)? {
                                    let predicate = self.type_predicate(value)?;
                                    result_guarantees.push(predicate);
                                    continue;
                                }
                            }
                            postconditions.push(*value);
                            continue;
                        }
                    }
                    if let hir::clause::ClauseConstraintKind::Decreases(value) = &clause.kind {
                        if !matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig) if !sig.modifiers.is_normal_form_head)
                        {
                            return Err(error(
                                clause.span,
                                "decreases clauses are only supported on functions",
                            ));
                        }
                        decreases_clauses.push(*value);
                        continue;
                    }
                    let hir::clause::ClauseConstraintKind::Requires(value) = &clause.kind else {
                        return Err(error(
                            clause.span,
                            "program specification elaboration is not implemented",
                        ));
                    };
                    if self.program_requirement(value)? {
                        if !matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig) if !sig.modifiers.is_normal_form_head)
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
                Ok(())
            })();
            if let Some((ret_local, binder_decl)) = result_binder {
                self.input_facts.types.remove(&ret_local);
                self.input_facts.derived.remove(&ret_local);
                self.locals.remove(&ret_local);
                if clause_result.is_ok() && !result_guarantees.is_empty() {
                    self.result_guarantees
                        .insert(owner, (binder_decl, result_guarantees));
                }
            }
            clause_result?;
            self.program_requirements
                .insert(owner, program_requirements);
            self.program_postconditions.insert(owner, postconditions);
            self.program_decreases.insert(owner, decreases_clauses);
            Ok(requirements)
        })();
        let uses = std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
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
            ItemKind::Deriver(def) => def.clause_constraints,
            ItemKind::Derive(req) => req.clause_constraints,
            ItemKind::Trait(def) => def.clause_constraints,
            ItemKind::EffectGroup(group) => group.clause_constraints,
            _ => &[],
        };
        for constraint in constraints {
            if let hir::clause::ClauseConstraintKind::Requires(value) = &constraint.kind {
                signature_capability_atoms(value, &mut pending, 0)?;
            }
        }
        if pending.is_empty() {
            return Ok(());
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
                let ready = if complete {
                    true
                } else {
                    let inputs = self.source_expression_inputs(owner, &value)?;
                    inputs
                        .locals
                        .iter()
                        .all(|id| self.input_facts.has_identity(id))
                        && inputs.declarations.iter().all(|id| {
                            !check_clauses
                                || self.type_clause_params(owner).iter().any(|parameter| {
                                    parameter.definition.is_some_and(|definition| {
                                        DeclarationId {
                                            package: semantic_package(
                                                &self.package_identities,
                                                definition.pkg,
                                            ),
                                            local: definition.index,
                                        } == *id
                                    }) && matches!(
                                        parameter.kind.unquoted(),
                                        hir::clause::ClauseParamKind::Type(_)
                                            | hir::clause::ClauseParamKind::TraitBound(..)
                                    )
                                })
                                || self.trait_input_domain(*id).is_some()
                        })
                };
                if !ready {
                    pending.push(value);
                    continue;
                }
                // A failed atom must not publish partial assumptions or uses.
                // Completed declaration caches remain reusable across retries.
                let capabilities = self.capabilities.get(&owner).cloned();
                let context = self.context.clone();
                let pending_len = self.pending_type_uses.as_ref().map(Vec::len);
                // This atom is still being formed. Its operands may use prior
                // binder evidence, but cannot demand the enclosing contract as
                // a premise and recursively assume the atom they are computing.
                let saved_checking = std::mem::replace(&mut self.checking_contract, true);
                let checked = self.positive_type_requirements(owner, &value, &mut Vec::new(), 0);
                self.checking_contract = saved_checking;
                match checked {
                    Ok(()) => {
                        /*
                         * Unknown remains an error in the current world. Only
                         * newly established premises justify another attempt
                         * at the deferred atoms; a duplicate or a successfully
                         * parsed non-capability predicate supplies no progress.
                         */
                        progress |= self.capabilities.get(&owner).map_or(0, Vec::len)
                            > capabilities.as_ref().map_or(0, Vec::len);
                    }
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
                        if !failure.retry_after_capability_progress {
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

    pub(super) fn program_predicate_shape(
        &mut self,
        expression: &n::Expr,
        calls: bool,
        span: Span,
    ) -> Result<bool, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = if calls {
            n::predicate_call_candidate(expression, &mut fuel)
        } else {
            n::checkable_predicate(expression, &mut fuel)
        };
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|reason| error(span, reason.to_string()))
    }

    pub(super) fn program_preconditions(&mut self, owner: OwnerId) -> Result<Vec<n::Expr>, Error> {
        let values = self
            .program_requirements
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        self.with_program_predicate(|this| {
            let mut result = Vec::new();
            for value in values {
                let expression = this.boolean_predicate(value)?;
                if !this.program_predicate_shape(&expression, false, value.span)? {
                    return Err(error(
                        value.span,
                        "requires needs a supported effect-free program predicate",
                    ));
                }
                result.push(expression);
            }
            Ok(result)
        })
    }

    pub(super) fn program_decreases(&mut self, owner: OwnerId) -> Result<Vec<n::Expr>, Error> {
        let values = self
            .program_decreases
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        self.with_program_predicate(|this| {
            let mut result = Vec::new();
            for value in values {
                let metric = this.integer_metric_expression(value)?;
                result.push(metric);
            }
            Ok(result)
        })
    }

    pub(super) fn integer_metric_expression(&mut self, value: &Expr<'_>) -> Result<n::Expr, Error> {
        let metric = self.expr(value, None)?;
        let is_integer = matches!(
            &metric.ty,
            Term::Primitive(primitive)
                if primitive.fixed_integer_layout().is_some()
                    || matches!(primitive, P::Usize | P::Isize | P::Integer)
        );
        if !is_integer {
            return Err(error(
                value.span,
                "decreases metric must be an integer expression",
            ));
        }
        let check = n::Expr {
            source: metric.source,
            ty: p(P::Bool),
            kind: K::ScalarGreaterEqual(Box::new(metric.clone()), Box::new(metric.clone())),
        };
        if !self.program_predicate_shape(&check, false, value.span)? {
            return Err(error(
                value.span,
                "decreases metric must be an effect-free integer expression",
            ));
        }
        Ok(metric)
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
        let result: Result<Vec<n::Expr>, Error> = self.with_program_predicate(|this| {
            values
                .into_iter()
                .map(|value| {
                    let expr = this.boolean_predicate(value)?;
                    if !this.program_predicate_shape(&expr, false, value.span)? {
                        return Err(error(
                            value.span,
                            "ensures needs a supported effect-free program predicate",
                        ));
                    }
                    Ok(expr)
                })
                .collect()
        });
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
        if self.data_consumer_call(expr)?.is_some() {
            return self.required_data_consumer(expr, expected);
        }
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
        self.required_invocation(prepared, values, owner, span)
            .execute(self)?
            .publish(self)
    }

    pub(super) fn required_invocation(
        &self,
        prepared: PreparedRequired,
        values: Vec<(n::LocalId, Value)>,
        owner: OwnerId,
        span: Span,
    ) -> Invocation {
        Invocation {
            prepared,
            values,
            package: declaration(self.package, &self.package_identities, owner).package,
            identities: self.definition_identities.clone(),
            span,
        }
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
        self.invoke_required_with_live_inputs(prepared, live, source)
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
        self.invoke_required_with_live_inputs(prepared, live, source)
    }

    pub(super) fn invoke_required_with_live_inputs(
        &mut self,
        prepared: PreparedRequired,
        live: Vec<(n::LocalId, Term, Value)>,
        source: &Expr<'_>,
    ) -> Result<(Value, Term), Error> {
        if live.is_empty() {
            return self.invoke_required_publishing(
                prepared,
                Vec::new(),
                source.hir_id.owner,
                source.span,
            );
        }
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
        self.prepare_required_root_with_live_inputs(
            expr,
            RootKind::Expression,
            inputs,
            defer_inputs,
            live,
            |checker| {
                checked
                    .map(Ok)
                    .unwrap_or_else(|| checker.expr(expr, expected))
                    .map(|root| (root, ()))
            },
        )
        .map(|(prepared, ())| prepared)
    }

    /// Construct a required root inside the same isolated checking context as
    /// source expressions. Callers must not precheck staged producers in the
    /// runtime caller's return, loop, or capture context.
    /// The consumer travels with the checked root until its world is prepared.
    /// It belongs to the caller and does not enter the frozen invocation world.
    pub(super) fn prepare_required_root_with_live_inputs<C>(
        &mut self,
        expr: &Expr<'_>,
        kind: RootKind,
        inputs: Vec<(n::LocalId, Term, Term)>,
        defer_inputs: bool,
        live: &[(n::LocalId, Term, Value)],
        build: impl FnOnce(&mut Self) -> Result<(n::Expr, C), Error>,
    ) -> Result<(PreparedRequired, C), Error> {
        let mut supplied: BTreeSet<_> = inputs.iter().map(|(binding, _, _)| *binding).collect();
        for (binding, _, _) in live {
            if !supplied.insert(*binding) {
                return Err(error(expr.span, "duplicate required comptime input"));
            }
        }
        let entry = if kind == RootKind::AssociatedType {
            None
        } else {
            entry_call(expr, &mut self.required_steps)?
        };
        let query = (local(expr.hir_id), kind);
        if self.active_required.len() >= 32 || !self.active_required.insert(query) {
            return Err(error(
                expr.span,
                "cyclic or excessively deep required comptime dependency",
            ));
        }
        let saved_entry = std::mem::replace(&mut self.required_entry_call, entry);
        let saved_control = std::mem::take(&mut self.control);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_code = std::mem::replace(&mut self.in_code_body, false);
        let saved_unsafe = std::mem::replace(&mut self.unsafe_depth, 0);
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let saved_pending = self.pending_type_uses.replace(Vec::new());
        let signature_start = self.signature_type_uses.len();
        let saved_value_locals = self.locals.clone();
        let saved_value_bindings = self.normal_default_bindings.clone();
        let saved_input_scope = self.input_facts.supplied.clone();
        let saved_input_immutable = self.immutable.clone();
        let result = (|| {
            let mut inputs = inputs;
            // Nested checks may use supplied input evidence from this region,
            // while pending defaults and live storage remain unavailable.
            self.input_facts.supplied = self.input_facts.supplied.child(
                &inputs,
                live,
                &mut self.required_steps,
                expr.span,
            )?;
            let mut abstract_value_parameters = Vec::new();
            let visible = self.visible_comptime_parameters(expr.span)?;
            for id in visible {
                let Some(domain) = self.trait_input_domain(id) else {
                    continue;
                };
                if Sort::of_comptime_domain(&domain) == Sort::Runtime {
                    if self.alias_substitution.get(&id).is_none() {
                        let domain = self.substituted_trait_domain(&domain, expr.span)?;
                        abstract_value_parameters.push((id, domain));
                    }
                    continue;
                }
                if Sort::of_comptime_domain(&domain) != Sort::ComptimeValue {
                    continue;
                }
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
                    .cloned()
                    .unwrap_or(Term::Rigid {
                        declaration: id,
                        sort: Sort::ComptimeValue,
                    });
                let binding = match existing {
                    Some(binding) => binding,
                    None => self.spread_binding(expr.span)?,
                };
                self.locals.insert(binding, domain.clone());
                self.immutable.insert(binding);
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
                // Publish the completed prefix before forming the next domain.
                self.input_facts.supplied.supply(
                    binding,
                    &domain,
                    &value,
                    &mut self.required_steps,
                    expr.span,
                )?;
                supplied.insert(binding);
                inputs.push((binding, domain, value));
            }
            let (mut root, consumer) = build(self).map_err(|mut e| {
                if e.message == "local binding is unavailable in this scope" {
                    e.message =
                        "required comptime input is not yet available in this definition".into();
                }
                e
            })?;
            if let Some((parameters, arguments)) = self.initializer_substitution() {
                // The selected initializer specializes its entire invocation
                // frame, including lexical value captures. Substituting only
                // the body leaves those inputs abstract or in the wrong domain.
                let domains = parameters
                    .iter()
                    .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                    .collect::<Vec<_>>();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let specialized = (|| {
                    let substitution = nir::DeclarationSubstitution::bounded(
                        parameters,
                        &domains,
                        arguments,
                        &[],
                        &mut fuel,
                    )
                    .map_err(|e| error(expr.span, e.to_string()))?;
                    root = substitution
                        .instantiate_expression_bounded(&root, &mut fuel)
                        .map_err(|e| error(expr.span, e.to_string()))?;
                    for (_, domain, value) in &mut inputs {
                        *domain = substitution
                            .instantiate_bounded(domain, &mut fuel)
                            .map_err(|e| error(expr.span, e.to_string()))?;
                        *value = substitution
                            .instantiate_bounded(value, &mut fuel)
                            .map_err(|e| error(expr.span, e.to_string()))?;
                    }
                    for (_, domain) in &mut abstract_value_parameters {
                        *domain = substitution
                            .instantiate_bounded(domain, &mut fuel)
                            .map_err(|e| error(expr.span, e.to_string()))?;
                    }
                    Ok::<_, Error>(())
                })();
                self.required_steps -= (before - fuel) as u64;
                specialized?;
            }
            let result_type = root.ty.clone();
            // The root is structurally stable after initializer substitution.
            // Later staged-slot rebinding changes Bound terms to rigid identities,
            // not lexical LocalIds. These reads locate outer bindings. The entry
            // telescope below additionally closes over their dependent domains.
            let mut capture_fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = capture_fuel;
            let capture_body =
                nir::type_computations::CaptureBody::analyze(root, &mut capture_fuel);
            self.required_steps -= (before - capture_fuel) as u64;
            let mut root = capture_body.map_err(|message| error(expr.span, message))?;
            let free = root.reads();
            for input in &self.input_facts.specialized {
                if supplied.insert(input.0) {
                    inputs.push(input.clone());
                }
            }
            if !self.checking_code_generator
                && !self.generator_data.is_empty()
                && !self.input_facts.partial.is_empty()
            {
                for binding in free.iter().copied() {
                    if supplied.contains(&binding) {
                        continue;
                    }
                    let captured = self.with_observation_budget(expr.span, |this, budget| {
                        this.observed_generator_input(binding)
                            .map(|(domain, value)| {
                                Ok((
                                    nir::copy_budget::clone_term(domain, &mut budget.steps)?,
                                    nir::copy_budget::clone_term(value, &mut budget.steps)?,
                                ))
                            })
                            .transpose()
                    })?;
                    if let Some((domain, value)) = captured {
                        supplied.insert(binding);
                        inputs.push((binding, domain, value));
                    }
                }
            }
            // Retain only referenced outer inputs, including dependencies in
            // type annotations. This transports existing evidence; it does not
            // evaluate a producer to fill a missing capture.
            if !self.input_facts.supplied.is_empty() {
                for binding in free.iter().copied() {
                    if supplied.contains(&binding)
                        || !self.locals.contains_key(&binding)
                        || !self.immutable.contains(&binding)
                    {
                        continue;
                    }
                    if let Some((domain, Some(value))) = self.input_facts.supplied.lookup(
                        binding,
                        &mut self.required_steps,
                        expr.span,
                    )? {
                        let domain = inputs::copy(domain, &mut self.required_steps, expr.span)?;
                        let value = inputs::copy(value, &mut self.required_steps, expr.span)?;
                        supplied.insert(binding);
                        inputs.push((binding, domain, value));
                    }
                }
            }
            let mut scoped_parameters = Vec::new();
            let mut scoped_arguments = Vec::new();
            let input_frames = self.call_input_frames();
            let mut caller_mapping = None;
            // Reify the caller's staged slots for this invocation. Their
            // identities can occur in a type without computing their values.
            if let Some(owner) = self.default_input_owner.or(self.signature_owner)
                && !defer_inputs
                && let Some(frame) = input_frames.last()
                && frame.iter().any(|sort| *sort != Sort::Runtime)
            {
                self.required_steps = self
                    .required_steps
                    .checked_sub(frame.len() as u64)
                    .ok_or_else(|| error(expr.span, "scoped input preparation budget exhausted"))?;
                let mut mapping = frame
                    .iter()
                    .enumerate()
                    .map(|(index, sort)| Term::Bound {
                        depth: 0,
                        index,
                        sort: *sort,
                    })
                    .collect::<Vec<_>>();
                let type_domain = p(P::Type);
                let owner = owner.def_id.raw();
                let bindings = n::LocalId { owner, index: 0 }..=n::LocalId {
                    owner,
                    index: u32::MAX,
                };
                let staged = self
                    .input_facts
                    .types
                    .range(bindings.clone())
                    .map(|(binding, symbol)| (binding, symbol, &type_domain))
                    .chain(
                        self.input_facts
                            .values
                            .range(bindings)
                            .map(|(binding, (symbol, domain))| (binding, symbol, domain)),
                    )
                    .chain(self.input_facts.parameters.identities().filter_map(
                        |(binding, (symbol, domain))| {
                            (binding.owner == owner
                                && !self.input_facts.types.contains_key(binding)
                                && !self.input_facts.values.contains_key(binding))
                            .then_some((binding, symbol, domain))
                        },
                    ));
                for (binding, symbol, domain) in staged {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(expr.span, "scoped input preparation budget exhausted")
                    })?;
                    let Term::Bound {
                        depth: 0,
                        index,
                        sort,
                    } = symbol
                    else {
                        continue;
                    };
                    if frame.get(*index) != Some(sort) {
                        continue;
                    }
                    let local = u32::try_from(*index)
                        .ok()
                        .filter(|i| *i != u32::MAX)
                        .ok_or_else(|| error(expr.span, "too many scoped inputs"))?;
                    let declaration = DeclarationId {
                        package: u64::MAX,
                        local,
                    };
                    if matches!(mapping[*index], Term::Bound { .. }) {
                        mapping[*index] = Term::Rigid {
                            declaration,
                            sort: *sort,
                        };
                        scoped_parameters.push(nir::DeclarationParameter {
                            declaration,
                            sort: *sort,
                        });
                        scoped_arguments.push(symbol.clone());
                        if matches!(*sort, Sort::ComptimeValue | Sort::Runtime) {
                            abstract_value_parameters.push((
                                declaration,
                                inputs::copy(domain, &mut self.required_steps, expr.span)?,
                            ));
                        }
                    }
                    if free.contains(binding) && supplied.insert(*binding) {
                        inputs.push((
                            *binding,
                            inputs::copy(domain, &mut self.required_steps, expr.span)?,
                            mapping[*index].clone(),
                        ));
                    }
                }
                if !scoped_parameters.is_empty() {
                    let mut rebind = |ty: &Term| {
                        scope::rebind_input(ty, &mapping, &mut self.required_steps, expr.span)
                    };
                    root = root.rewrite_types(&mut rebind)?;
                    for (_, ty, value) in &mut inputs {
                        *ty = rebind(ty)?;
                        *value = rebind(value)?;
                    }
                    caller_mapping = Some(mapping);
                }
            }
            let capture = self.capture_required_entry(root, inputs, live, expr.span)?;
            let mut observed = capture.observe(
                Discovery::Required(&self.implementation_index, &self.member_sources),
                &mut self.required_steps,
                expr.span,
            )?;
            let mut closure_roots = std::mem::take(&mut observed.code);
            let mut used_runtime_ids = BTreeSet::new();
            for (_, ty, value) in &capture.inputs {
                used_runtime_ids.extend(ty.runtime_dependencies());
                if let Some(val) = value {
                    used_runtime_ids.extend(val.runtime_dependencies());
                }
            }
            used_runtime_ids.extend(capture.function.signature.runtime_dependencies());
            let mut collect_expr_runtimes = |target_expr: &n::Expr,
                                             ids: &mut BTreeSet<nir::RuntimeId>,
                                             steps: &mut u64|
             -> Result<(), Error> {
                let before = usize::try_from(*steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let res = nir::visit::expression_checked(target_expr, &mut fuel, |sub, _| {
                    ids.extend(sub.ty.runtime_dependencies());
                    match &sub.kind {
                        K::TypeValue(t) | K::ComptimeValue(t) | K::StagedValue(t) => {
                            ids.extend(t.runtime_dependencies());
                        }
                        K::FunctionInstance { arguments, .. } => {
                            for arg in arguments {
                                ids.extend(arg.runtime_dependencies());
                            }
                        }
                        K::EffectRequest { signature, .. } => {
                            ids.extend(signature.runtime_dependencies());
                        }
                        K::BitUpdate { storage_type, .. } | K::BitAssign { storage_type, .. } => {
                            ids.extend(storage_type.runtime_dependencies());
                        }
                        _ => {}
                    }
                    Ok::<(), nir::visit::Error>(())
                });
                *steps = steps.saturating_sub((before - fuel) as u64);
                res.map_err(|err| error(expr.span, err.to_string()))
            };
            collect_expr_runtimes(
                &capture.function.body,
                &mut used_runtime_ids,
                &mut self.required_steps,
            )?;
            let source_closure::SourceClosure {
                mut functions,
                mut runtime_function_identities,
                mut retained,
            } = self.required_source_closure(closure_roots.clone(), expr.span)?;
            // Include transitive global runtime delivery origins referenced by the
            // captured invocation and fold any dependencies they reference into closure_roots.
            let mut seen_deliveries = BTreeSet::new();
            let mut extra_closure_added = false;
            loop {
                for f in &functions {
                    used_runtime_ids.extend(f.signature.runtime_dependencies());
                    for (&rid, delivery) in &f.runtime_deliveries {
                        used_runtime_ids.insert(rid);
                        collect_expr_runtimes(
                            delivery,
                            &mut used_runtime_ids,
                            &mut self.required_steps,
                        )?;
                    }
                    collect_expr_runtimes(
                        &f.body,
                        &mut used_runtime_ids,
                        &mut self.required_steps,
                    )?;
                }
                let pending_deliveries: Vec<nir::RuntimeId> = used_runtime_ids
                    .difference(&seen_deliveries)
                    .copied()
                    .collect();
                if pending_deliveries.is_empty() {
                    break;
                }
                let mut new_roots = false;
                for rid in pending_deliveries {
                    seen_deliveries.insert(rid);
                    if let Some(delivery) = self.global_runtime_deliveries.get(&rid).cloned() {
                        collect_expr_runtimes(
                            &delivery,
                            &mut used_runtime_ids,
                            &mut self.required_steps,
                        )?;
                        let refs =
                            Discovery::Required(&self.implementation_index, &self.member_sources)
                                .references(&delivery, &mut self.required_steps)
                                .map_err(|failure| error(expr.span, failure.to_string()))?;
                        for dep in refs {
                            if closure_roots.insert(dep) {
                                new_roots = true;
                            }
                        }
                    }
                }
                if !new_roots {
                    break;
                }
                extra_closure_added = true;
                let refreshed = self.required_source_closure(closure_roots.clone(), expr.span)?;
                functions = refreshed.functions;
                runtime_function_identities = refreshed.runtime_function_identities;
                retained = refreshed.retained;
            }
            let _ = extra_closure_added;
            let capture::Entry {
                function,
                inputs,
                parameter_slots,
                ..
            } = capture;
            let entry = function.declaration;
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
                                    ItemKind::Fn(sig, _)
                                    | ItemKind::Case(sig, _)
                                    | ItemKind::FnDeclaration(sig) => {
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
                        && premise_owner.is_none()
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
                    self.validate_bit_packed_type_application(owner, &arguments.terms, span)?;
                }
                Ok(())
            })();
            self.context = saved_context;
            self.signature_owner = saved_owner;
            checked?;
            if !self.interfaces_ready
                && self.pending_refinements.iter().any(|pending| {
                    let id = declaration(self.package, &self.package_identities, pending.owner);
                    retained.contains(&Dependency::Definition(id))
                        || retained.contains(&Dependency::InterfaceMember(Member::Method(id)))
                })
            {
                return Err(error(
                    expr.span,
                    "required comptime evaluation is waiting for a signature type refinement",
                ));
            }
            functions.push(function);
            let trait_world::TraitWorld {
                traits,
                implementations,
                method_links,
            } = self.required_trait_world(&retained, expr.span)?;
            let mut package = n::Package {
                formed_nullary_nominals: self.declared_theory.formed_nullary_nominals().clone(),
                builtin_implementation_packages: self
                    .protocols
                    .builtin_implementation_packages
                    .clone(),
                package_metadata: self.package_metadata.clone(),
                declaration_paths: self.declaration_paths.clone(),
                refinements: self.refinements.clone(),
                definition_identity_cursor: self.definition_identities.next(),
                target: self.target,
                method_links,
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
                statics: self.statics.values().cloned().collect(),
                definition_attributes: self.definition_attributes.clone(),
                runtime_domains: self.runtime_domains.clone(),
                global_runtime_deliveries: self
                    .global_runtime_deliveries
                    .iter()
                    .filter(|(id, _)| used_runtime_ids.contains(id))
                    .map(|(id, expr)| (*id, expr.clone()))
                    .collect(),
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
            let world = prepared::RequiredWorld::capture(
                package,
                &mut self.required_steps,
                expr.span,
                &self.expansion_sources,
                &self.member_sources.owner,
            )?;
            // The snapshot contains demanded layouts only. Nominal identities
            // are admitted from source declarations even when no value/layout
            // of that type is needed by this comptime invocation.
            let type_context = if self.initializer_implementation().is_some() {
                self.context.clone()
            } else {
                self.nominal_context()
            };
            // A binder in the enclosing declaration is available to the
            // invocation only if its code, captures or their domains need it.
            // An unrelated open binder must not turn closed work abstract.
            let mut available_types: BTreeSet<_> = self
                .signature_owner
                .and_then(|owner| self.type_parameters.get(&owner))
                .into_iter()
                .flatten()
                .copied()
                .filter(|id| {
                    !self.alias_substitution.contains_key(id)
                        && self
                            .trait_input_domain(*id)
                            .is_none_or(|domain| Sort::of_comptime_domain(&domain) == Sort::Type)
                })
                .collect();
            available_types.extend(
                scoped_parameters
                    .iter()
                    .filter(|p| p.sort == Sort::Type)
                    .map(|p| p.declaration),
            );
            // Substitution RHSs can introduce caller binders. They supply a
            // lexical directory; only retained inputs are live roots.
            let mut forwarded = BTreeSet::new();
            for (term, retained) in self
                .alias_substitution
                .values()
                .map(|ty| (ty, false))
                .chain(inputs.iter().flat_map(|(_, domain, value)| {
                    std::iter::once((domain, true)).chain(value.iter().map(|v| (v, true)))
                }))
            {
                let uses =
                    input_observation::term_inputs(term, &mut self.required_steps, expr.span)?;
                for (id, sort) in uses.declarations {
                    match sort {
                        Sort::Type => {
                            available_types.insert(id);
                            if retained {
                                observed.types.insert(id);
                            }
                        }
                        Sort::ComptimeValue | Sort::Runtime => {
                            forwarded.insert(id);
                            if retained {
                                observed.values.insert(id);
                            }
                        }
                    }
                }
            }
            if !forwarded.is_empty() {
                let local: BTreeSet<_> = abstract_value_parameters
                    .iter()
                    .map(|(id, _)| *id)
                    .chain(
                        self.callable_type_inputs
                            .values()
                            .flatten()
                            .map(|slot| slot.declaration),
                    )
                    .collect();
                for &id in forwarded.difference(&local) {
                    let domain = self.trait_input_domain(id).ok_or_else(|| {
                        error(
                            expr.span,
                            "forwarded value input lacks declaration domain evidence",
                        )
                    })?;
                    abstract_value_parameters.push((id, domain));
                }
            }
            abstract_value_parameters
                .extend(available_types.into_iter().map(|id| (id, p(P::Type))));
            let (lexical_types, abstract_value_parameters) = scope::select_abstract_inputs(
                &abstract_value_parameters,
                &self.callable_type_inputs,
                caller_mapping.as_deref(),
                &world,
                &observed,
                &mut self.required_steps,
                expr.span,
            )?;
            observed.types.extend(lexical_types);
            let abstract_parameters: Vec<_> = observed.types.into_iter().collect();
            let mut raw_assumptions = Vec::new();
            if let Some(implementation) = self.initializer_implementation() {
                raw_assumptions.extend(implementation.capabilities.iter().cloned());
            }
            if !abstract_parameters.is_empty() {
                let abstract_type_set: BTreeSet<DeclarationId> =
                    abstract_parameters.iter().copied().collect();
                let abstract_value_set: BTreeSet<DeclarationId> = abstract_value_parameters
                    .iter()
                    .map(|(id, _)| *id)
                    .collect();
                for cap in self.capability_assumptions() {
                    if !world.package().traits.iter().any(|t| {
                        t.declaration == cap.interface
                            && t.type_parameters.len() == cap.interface_arguments.len()
                    }) {
                        continue;
                    }
                    let in_scope = std::iter::once(&cap.subject)
                        .chain(&cap.interface_arguments)
                        .all(|term| {
                            term.check_scope(&[]).is_ok()
                                && term.rigid_parameters().is_ok_and(|rigids| {
                                    rigids.into_iter().all(|(id, sort)| match sort {
                                        Sort::Type => abstract_type_set.contains(&id),
                                        Sort::ComptimeValue => abstract_value_set.contains(&id),
                                        Sort::Runtime => false,
                                    })
                                })
                        });
                    if in_scope && !raw_assumptions.contains(&cap) {
                        raw_assumptions.push(cap);
                    }
                }
            }
            let invocation_assumptions = if raw_assumptions.is_empty() {
                vec![]
            } else {
                crate::capabilities::capability_sources(
                    &world.package().traits,
                    &raw_assumptions,
                    crate::ptc::Budget::default(),
                )
                .map_err(|message| error(expr.span, message))?
            };
            // This is checked source for an invocation, not a concrete dispatch
            // closure. The private VM resolves demanded members in RequiredWorld;
            // unrelated concrete implementations must not be selected here.
            // Equality observations of opaque caller Type inputs need their
            // declaration facts. These facts are supplied only to this invocation.
            let invocation_type_context = if abstract_parameters.is_empty() {
                &type_context
            } else {
                &self.context
            };
            let prepared = PreparedRequired {
                world,
                entry,
                inputs,
                result_type,
                scope: std::sync::Arc::new(prepared::InvocationScope {
                    parameter_slots,
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
                }),
            };
            Ok((prepared, consumer))
        })();
        let pending = std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
        if let Some(uses) = &mut self.pending_type_uses {
            uses.extend(pending);
        }
        self.locals = saved_value_locals;
        self.normal_default_bindings = saved_value_bindings;
        self.input_facts.supplied = saved_input_scope;
        self.immutable = saved_input_immutable;
        self.control = saved_control;
        self.captures = saved_captures;
        self.in_code_body = saved_code;
        self.unsafe_depth = saved_unsafe;
        self.in_required_comptime = saved_mode;
        self.numeric_defaults = saved_numeric;
        self.required_entry_call = saved_entry;
        self.active_required.remove(&query);
        result
    }

    pub(super) fn required_function(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<n::Function, Error> {
        let item = self.package.item(owner).unwrap();
        let (is_pattern_fn, sig) = match &item.kind {
            ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) => (false, sig),
            ItemKind::FnDeclaration(sig)
                if self
                    .package
                    .owner_definition(owner)
                    .is_some_and(|def| self.package.has_pattern_fn_arms(def)) =>
            {
                (true, sig)
            }
            _ => return Err(error(span, "required comptime call needs a function body")),
        };
        if !sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head {
            return Err(error(
                span,
                "required comptime evaluation cannot execute a runtime function or normal-form head",
            ));
        }
        if is_pattern_fn {
            return self.check_and_cache_pattern_function(owner, span);
        }
        self.ensure_signature(owner, span)?;
        self.ensure_contract(owner, span)?;
        let signature = self
            .declared_signature(owner)
            .map(|sig| sig.into_owned())
            .ok_or_else(|| {
                error(
                    span,
                    "required comptime function signature still needs invocation inputs",
                )
            })?;
        self.check_source_function(
            owner,
            signature,
            function_bodies::SourceBody::Required,
            span,
        )
    }
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
        | H::TakeAttribute(value)
        | H::TakeIntrinsic(value)
        | H::TakeTupleEnumerate(value)
        | H::TakeTupleIndexed(value)
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
            n::TypePredicate::Bool(_) | n::TypePredicate::IntLe(..) => {}
            n::TypePredicate::Trait(r) => {
                terms.push(&r.subject);
                terms.extend(&r.interface_arguments);
            }
            n::TypePredicate::Equal(a, b) => {
                terms.push(a);
                terms.push(b);
            }
            n::TypePredicate::Kind(ty, _) | n::TypePredicate::NotSized(ty) => terms.push(ty),
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
        Q::IntLe(a, b) => Q::IntLe(
            a.try_map_rigid(map, || {
                error(
                    span,
                    "integer difference term cannot be represented in i128 after substitution",
                )
            })?,
            b.try_map_rigid(map, || {
                error(
                    span,
                    "integer difference term cannot be represented in i128 after substitution",
                )
            })?,
        ),
        Q::Kind(ty, kind) => Q::Kind(map(ty)?, *kind),
        Q::NotSized(ty) => Q::NotSized(map(ty)?),
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

struct RequiredHashWriter(u64);

impl RequiredHashWriter {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
    }

    fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    fn str(&mut self, text: &str) {
        self.u64(text.len() as u64);
        for &byte in text.as_bytes() {
            self.byte(byte);
        }
    }

    fn display(&mut self, value: &impl std::fmt::Display) {
        let _ = std::fmt::Write::write_fmt(self, format_args!("{value}"));
        self.byte(0xff);
    }
}

impl std::fmt::Write for RequiredHashWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for &byte in s.as_bytes() {
            self.byte(byte);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq)]
struct RequiredMemoKey {
    canonical_body: String,
    inputs: Vec<(n::LocalId, Term, Term)>,
    result_type: Term,
}

struct RequiredMemoProtocol;

impl KeyProtocol<RequiredMemoKey> for RequiredMemoProtocol {
    type Error = Error;

    fn hash(
        &mut self,
        value: &RequiredMemoKey,
        context: &mut Context,
    ) -> Result<u64, comptime_executor::Error<Error>> {
        let cost = value
            .canonical_body
            .len()
            .div_ceil(64)
            .saturating_add(value.inputs.len())
            .saturating_add(1);
        context.spend::<Error>(cost)?;
        let mut writer = RequiredHashWriter::new();
        writer.str(&value.canonical_body);
        writer.u64(value.inputs.len() as u64);
        for (local, domain, val) in &value.inputs {
            writer.u64(u64::from(local.owner));
            writer.u64(u64::from(local.index));
            writer.display(domain);
            writer.display(val);
        }
        writer.display(&value.result_type);
        Ok(writer.0)
    }

    fn equivalent(
        &mut self,
        left: &RequiredMemoKey,
        right: &RequiredMemoKey,
        context: &mut Context,
    ) -> Result<bool, comptime_executor::Error<Error>> {
        let cost = left
            .canonical_body
            .len()
            .min(right.canonical_body.len())
            .div_ceil(64)
            .saturating_add(left.inputs.len())
            .saturating_add(1);
        context.spend::<Error>(cost)?;
        Ok(left == right)
    }
}

struct RequiredMemoState {
    cache: Cache<RequiredMemoKey, (Value, Term)>,
    protocol: RequiredMemoProtocol,
    hits: usize,
    misses: usize,
    invalidations: usize,
}

impl RequiredMemoState {
    fn new() -> Self {
        Self {
            cache: Cache::new(
                Scope {
                    definitions: 0,
                    protocols: 0,
                    target: 0,
                },
                256,
            ),
            protocol: RequiredMemoProtocol,
            hits: 0,
            misses: 0,
            invalidations: 0,
        }
    }
}

thread_local! {
    static REQUIRED_MEMO_CACHE: RefCell<RequiredMemoState> = RefCell::new(RequiredMemoState::new());
}

fn is_safe_memo_key_term(term: &Term, depth: usize) -> bool {
    if depth >= 64 {
        return false;
    }
    match term {
        Term::Primitive(_)
        | Term::Unit
        | Term::Bool(_)
        | Term::Char(_)
        | Term::Integer(_)
        | Term::BitVec(_)
        | Term::String(_)
        | Term::Symbol(_)
        | Term::ClosureContextType
        | Term::EffectRowType
        | Term::ErrorRowType
        | Term::ReachabilityType
        | Term::AnyType => true,
        Term::Tuple(items) | Term::TupleValue(items) => items
            .iter()
            .all(|item| is_safe_memo_key_term(item, depth + 1)),
        Term::Optional(inner)
        | Term::Pointer(inner)
        | Term::Lift(inner)
        | Term::ComptimeFunction(inner)
        | Term::UnsafeFunction(inner)
        | Term::PureFunction(inner) => is_safe_memo_key_term(inner, depth + 1),
        Term::NormalFormFunction(inner, _) => is_safe_memo_key_term(inner, depth + 1),
        Term::Record(fields) => fields
            .iter()
            .all(|(_, ty)| is_safe_memo_key_term(ty, depth + 1)),
        Term::Nominal { arguments, .. }
        | Term::NormalForm { arguments, .. }
        | Term::TraitObject { arguments, .. } => arguments
            .iter()
            .all(|arg| is_safe_memo_key_term(arg, depth + 1)),
        Term::Function { parameters, result } => {
            parameters
                .iter()
                .all(|param| is_safe_memo_key_term(&param.ty, depth + 1))
                && is_safe_memo_key_term(result, depth + 1)
        }
        Term::ErrorResult { result, errors } => {
            is_safe_memo_key_term(result, depth + 1)
                && errors
                    .iter()
                    .all(|err| is_safe_memo_key_term(err, depth + 1))
        }
        _ => false,
    }
}

fn is_pure_cacheable_expr(root: &n::Expr, package: &n::Package) -> bool {
    use n::ExprKind as K;
    let pure_fn = |id: DeclarationId| {
        package.functions.iter().any(|function| {
            function.declaration == id
                && function.pure
                && function.phase == n::FunctionPhase::Comptime
        })
    };
    let mut stack = vec![root];
    let mut visited = 0usize;
    while let Some(expr) = stack.pop() {
        visited += 1;
        if visited > 2048 {
            return false;
        }
        match &expr.kind {
            K::BuildStruct(_)
            | K::BuildNewtype(_)
            | K::BuildEnum(_)
            | K::BuildMetadata { .. }
            | K::DefinitionAttributes(_)
            | K::AssociatedValue { .. }
            | K::TraitMethod { .. }
            | K::EffectResume(_)
            | K::EffectHandle { .. }
            | K::EffectRun { .. }
            | K::EffectComputation { .. }
            | K::EffectRequest { .. }
            | K::AddressOf(_)
            | K::ReadPointer(_)
            | K::WritePointer { .. }
            | K::PointerAnonymousField { .. }
            | K::PointerField { .. }
            | K::BitWritePointer { .. }
            | K::DynCast(_)
            | K::DynCall(_)
            | K::StagedValue(_)
            | K::ConstAddress { .. }
            | K::StaticAddress(_) => return false,
            K::Call {
                callee,
                arguments,
                env_arguments,
                ..
            } => {
                let allowed_callee = match &callee.kind {
                    K::Function(id)
                    | K::FunctionInstance {
                        declaration: id, ..
                    } => pure_fn(*id),
                    _ => false,
                };
                if !allowed_callee {
                    return false;
                }
                stack.extend(arguments.iter().map(|arg| &arg.value));
                stack.extend(env_arguments.iter());
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                stack.extend(statements.iter());
                if let Some(result) = result {
                    stack.push(result);
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                stack.push(condition);
                stack.push(then_branch);
                if let Some(else_branch) = else_branch {
                    stack.push(else_branch);
                }
            }
            K::Match { value, arms } => {
                stack.push(value);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        stack.push(guard);
                    }
                    stack.push(&arm.body);
                }
            }
            K::Let { value, .. } => stack.push(value),
            K::LetPattern {
                value,
                guard,
                otherwise,
                ..
            } => {
                stack.push(value);
                if let Some(guard) = guard {
                    stack.push(guard);
                }
                if let Some(otherwise) = otherwise {
                    stack.push(otherwise);
                }
            }
            K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::ScalarLess(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreaterEqual(left, right)
            | K::ScalarEqual(left, right)
            | K::TemplateEqual(left, right)
            | K::TypeEqual(left, right)
            | K::ListConcat(left, right)
            | K::TypeListIndex(left, right)
            | K::BitEqual(left, right)
            | K::BitConcat(left, right)
            | K::BitIndex(left, right)
            | K::And(left, right)
            | K::Or(left, right) => {
                stack.push(left);
                stack.push(right);
            }
            K::Tuple(items) | K::TypeList(items) => {
                stack.extend(items.iter());
            }
            K::Intrinsic { .. } => return false,
            K::Record { fields, .. }
            | K::AnonymousRecord { fields }
            | K::Variant { fields, .. } => {
                stack.extend(fields.iter());
            }
            K::Object { fields, .. } => {
                stack.extend(fields.iter());
            }
            K::Return(value) => {
                if let Some(value) = value {
                    stack.push(value);
                }
            }
            K::Loop { body, .. } => stack.push(body),
            K::Assign { value, .. }
            | K::BitAssign { value, .. }
            | K::BitUpdate { value, .. }
            | K::Not(value)
            | K::Discard(value)
            | K::UnsafeScope(value)
            | K::RequirePredicate(value)
            | K::EnsurePredicate(value)
            | K::CheckPredicate(value)
            | K::Field { value, .. }
            | K::AnonymousField { value, .. }
            | K::OptionalPresent(value)
            | K::OptionalType(value)
            | K::PointerType(value)
            | K::ErrorConvert { value, .. }
            | K::BitSlice { value, .. }
            | K::BitVecType(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::NewtypeConvert(value)
            | K::Snapshot(value)
            | K::TakeBits(value)
            | K::IntegerRepresentation(value)
            | K::TryIntegerConvert(value)
            | K::ComptimeIntegerConvert(value)
            | K::ListLength(value)
            | K::NewtypeRepresentation(value)
            | K::StructFieldTypes(value)
            | K::StructFieldNames(value)
            | K::RecordFieldTypes(value)
            | K::RecordFieldNames(value)
            | K::ReflectCallable(value)
            | K::TypeId(value)
            | K::EnumStructure { subject: value, .. }
            | K::ObjectQuery { subject: value, .. }
            | K::ObjectEdit { subject: value, .. }
            | K::TextQuery { subject: value, .. }
            | K::SymbolName(value)
            | K::EnumName(value)
            | K::TypeKind { operand: value, .. }
            | K::TypeNotSized { operand: value }
            | K::TypeLayout { operand: value, .. } => stack.push(value),
            K::BitWindow {
                value, start, end, ..
            } => {
                stack.push(value);
                stack.push(start);
                stack.push(end);
            }
            K::Literal(_)
            | K::Unit
            | K::Local(_)
            | K::TypeValue(_)
            | K::ComptimeValue(_)
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::OptionalAbsent
            | K::Break(_)
            | K::Continue(_) => {}
            K::NormalFormType { .. }
            | K::ApplyNormalForm { .. }
            | K::ComptimeFunctionInstance { .. }
            | K::PlannedFunctionInstance { .. }
            | K::NominalType { .. }
            | K::CheckTypeAlias { .. }
            | K::WithTypeInputs { .. }
            | K::Closure { .. }
            | K::FunctionType { .. }
            | K::ClosureType { .. }
            | K::TraitObjectType { .. }
            | K::FunctionResultType { .. } => return false,
        }
    }
    true
}

fn canonical_required_body(expr: &n::Expr) -> Option<String> {
    use n::ExprKind as K;
    use std::fmt::Write as _;

    fn write_expr(expr: &n::Expr, out: &mut String, depth: usize) -> Option<()> {
        if depth >= 64 || out.len() > 16_384 {
            return None;
        }
        match &expr.kind {
            K::Block { statements, result } | K::CallPreparation { statements, result }
                if statements.is_empty() =>
            {
                if let Some(inner) = result {
                    return write_expr(inner, out, depth + 1);
                }
                out.push_str("(unit)");
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let _ = write!(out, "(block (ty {})", expr.ty);
                for stmt in statements {
                    out.push(' ');
                    write_expr(stmt, out, depth + 1)?;
                }
                if let Some(inner) = result {
                    out.push_str(" (res ");
                    write_expr(inner, out, depth + 1)?;
                    out.push(')');
                }
                out.push(')');
            }
            K::Call {
                parameter_bindings,
                callee,
                arguments,
                env_arguments,
            } if env_arguments.is_empty() => {
                let _ = write!(out, "(call (ty {}) (bindings", expr.ty);
                for b in parameter_bindings {
                    let _ = write!(out, " {}:{}", b.owner, b.index);
                }
                out.push_str(") ");
                write_expr(callee, out, depth + 1)?;
                for arg in arguments {
                    let _ = write!(
                        out,
                        " (arg {} {} {} ",
                        arg.parameter, arg.is_default, arg.passing
                    );
                    write_expr(&arg.value, out, depth + 1)?;
                    out.push(')');
                }
                out.push(')');
            }
            K::Function(id) => {
                let _ = write!(out, "(fn {id} {})", expr.ty);
            }
            K::FunctionInstance {
                declaration,
                arguments,
            } => {
                let _ = write!(out, "(fn-inst {declaration} (ty {})", expr.ty);
                for arg in arguments {
                    let _ = write!(out, " {arg}");
                }
                out.push(')');
            }
            K::Literal(lit) => {
                let _ = write!(out, "(lit {} {lit})", expr.ty);
            }
            K::TypeValue(ty) => {
                let _ = write!(out, "(ty-val {} {ty})", expr.ty);
            }
            K::ComptimeValue(val) => {
                let _ = write!(out, "(ct-val {} {val})", expr.ty);
            }
            K::Unit => out.push_str("(unit)"),
            K::OptionalAbsent => {
                let _ = write!(out, "(opt-none {})", expr.ty);
            }
            K::Local(local) => {
                let _ = write!(out, "(local {}:{} {})", local.owner, local.index, expr.ty);
            }
            K::Let { binding, value } => {
                let _ = write!(out, "(let {}:{} ", binding.owner, binding.index);
                write_expr(value, out, depth + 1)?;
                out.push(')');
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let _ = write!(out, "(if {} ", expr.ty);
                write_expr(condition, out, depth + 1)?;
                out.push(' ');
                write_expr(then_branch, out, depth + 1)?;
                if let Some(else_branch) = else_branch {
                    out.push(' ');
                    write_expr(else_branch, out, depth + 1)?;
                }
                out.push(')');
            }
            K::Tuple(items) => {
                let _ = write!(out, "(tuple {}", expr.ty);
                for item in items {
                    out.push(' ');
                    write_expr(item, out, depth + 1)?;
                }
                out.push(')');
            }
            K::IntegerBinary { op, left, right } => {
                let _ = write!(out, "(ibin {} {op:?} ", expr.ty);
                write_expr(left, out, depth + 1)?;
                out.push(' ');
                write_expr(right, out, depth + 1)?;
                out.push(')');
            }
            K::ScalarEqual(left, right) => {
                let _ = write!(out, "(seq {} ", expr.ty);
                write_expr(left, out, depth + 1)?;
                out.push(' ');
                write_expr(right, out, depth + 1)?;
                out.push(')');
            }
            K::ScalarLess(left, right) => {
                let _ = write!(out, "(slt {} ", expr.ty);
                write_expr(left, out, depth + 1)?;
                out.push(' ');
                write_expr(right, out, depth + 1)?;
                out.push(')');
            }
            K::ScalarGreater(left, right) => {
                let _ = write!(out, "(sgt {} ", expr.ty);
                write_expr(left, out, depth + 1)?;
                out.push(' ');
                write_expr(right, out, depth + 1)?;
                out.push(')');
            }
            K::ScalarLessEqual(left, right) => {
                let _ = write!(out, "(sle {} ", expr.ty);
                write_expr(left, out, depth + 1)?;
                out.push(' ');
                write_expr(right, out, depth + 1)?;
                out.push(')');
            }
            K::ScalarGreaterEqual(left, right) => {
                let _ = write!(out, "(sge {} ", expr.ty);
                write_expr(left, out, depth + 1)?;
                out.push(' ');
                write_expr(right, out, depth + 1)?;
                out.push(')');
            }
            K::TypeEqual(left, right) => {
                let _ = write!(out, "(teq {} ", expr.ty);
                write_expr(left, out, depth + 1)?;
                out.push(' ');
                write_expr(right, out, depth + 1)?;
                out.push(')');
            }
            K::Not(value) => {
                let _ = write!(out, "(not {} ", expr.ty);
                write_expr(value, out, depth + 1)?;
                out.push(')');
            }
            _ => return None,
        }
        Some(())
    }

    let mut out = String::new();
    write_expr(expr, &mut out, 0)?;
    Some(out)
}

fn scope_for_required(
    package: &n::Package,
    entry: DeclarationId,
    protocols_cfg: &Protocols,
) -> Scope {
    let mut defs = RequiredHashWriter::new();
    defs.u64(u64::from(package.definition_identity_cursor));
    defs.display(&package.observed_associated_types);
    defs.display(&package.formed_nullary_nominals);
    defs.display(&package.refinements);
    for effect in &package.effects {
        defs.display(effect);
    }
    for identity in &package.runtime_function_identities {
        defs.display(identity);
    }
    for decl in &package.function_parameters {
        defs.display(decl);
    }
    for head in &package.normal_forms {
        defs.display(head);
    }
    for enum_def in &package.enums {
        defs.display(enum_def);
    }
    for contract in &package.alias_contracts {
        defs.display(contract);
    }
    for newtype in &package.newtypes {
        defs.display(newtype);
    }
    for struct_def in &package.structs {
        defs.display(struct_def);
    }
    for function in &package.functions {
        if function.declaration != entry {
            defs.display(function);
        }
    }
    for static_def in &package.statics {
        let _ = std::fmt::Write::write_fmt(&mut defs, format_args!("{static_def:?}"));
        defs.byte(0xff);
    }
    defs.display(&package.definition_attributes);
    defs.display(&package.volatile_storage);
    for (pkg_id, meta) in &package.package_metadata {
        defs.u64(*pkg_id);
        defs.display(meta);
    }
    for (decl_id, path) in &package.declaration_paths {
        defs.display(decl_id);
        defs.display(path);
    }

    let mut protos = RequiredHashWriter::new();
    for pkg in &protocols_cfg.builtin_implementation_packages {
        protos.u64(*pkg);
    }
    for (pkg, name) in &protocols_cfg.package_names {
        protos.u64(*pkg);
        protos.str(name);
    }
    for (pkg, version) in &protocols_cfg.package_versions {
        protos.u64(*pkg);
        protos.str(version);
    }
    for slot in [
        protocols_cfg.copy,
        protocols_cfg.drop,
        protocols_cfg.index,
        protocols_cfg.iterator,
        protocols_cfg.into_iterator,
        protocols_cfg.ordering,
        protocols_cfg.equality,
        protocols_cfg.list_pattern,
        protocols_cfg.comptime_list_pattern,
        protocols_cfg.stable_list_pattern,
        protocols_cfg.list_pattern_endless,
        protocols_cfg.stable_list_pattern_endless,
        protocols_cfg.splice,
        protocols_cfg.meta_list,
        protocols_cfg.tuple,
    ] {
        match slot {
            None => protos.byte(0),
            Some(id) => {
                protos.byte(1);
                protos.u64(id.package);
                protos.u64(u64::from(id.local));
            }
        }
    }
    for trait_def in &package.traits {
        protos.display(trait_def);
    }
    for implementation in &package.implementations {
        protos.display(implementation);
    }
    for link in &package.method_links {
        protos.display(link);
    }
    for instance in &package.dispatch_instances {
        protos.display(instance);
    }

    let mut target = RequiredHashWriter::new();
    match &package.target {
        None => target.byte(0),
        Some(data) => {
            target.byte(1);
            target.display(data);
        }
    }

    Scope {
        definitions: defs.0,
        protocols: protos.0,
        target: target.0,
    }
}

fn is_exportable_memo_result(value: &Value) -> bool {
    if matches!(value, Value::TypeParameter(_) | Value::AbstractValue { .. }) {
        return false;
    }
    let mut fuel = 100_000usize;
    value.inspect_immutable(&mut fuel).is_ok()
}

#[allow(clippy::too_many_arguments)]
fn try_memoize_pure_invocation(
    prepared: PreparedRequired,
    values: &[(n::LocalId, Value)],
    supply_attributes: bool,
    protocols: &Protocols,
    steps: &mut u64,
    span: Span,
    package_id: u64,
    identities: &crate::definitions::DefinitionIdentities,
) -> Result<
    Result<((Value, Term), crate::definitions::PublishedDefinitions), PreparedRequired>,
    Error,
> {
    if supply_attributes
        || !values.is_empty()
        || !prepared.scope.scoped_parameters.is_empty()
        || !prepared.scope.abstract_parameters.is_empty()
        || !prepared.scope.abstract_value_parameters.is_empty()
        || !prepared.scope.invocation_assumptions.is_empty()
    {
        return Ok(Err(prepared));
    }
    let (query, key, scope) = {
        let package = prepared.world.package();
        if package.functions.iter().any(|f| {
            f.declaration != prepared.entry && (!f.pure || f.phase != n::FunctionPhase::Comptime)
        }) {
            return Ok(Err(prepared));
        }
        let Some(entry_fn) = package
            .functions
            .iter()
            .find(|f| f.declaration == prepared.entry)
        else {
            return Ok(Err(prepared));
        };
        if entry_fn.phase != n::FunctionPhase::Comptime
            || package
                .functions
                .iter()
                .any(|f| !is_pure_cacheable_expr(&f.body, package))
        {
            return Ok(Err(prepared));
        }
        let Some(canonical_body) = canonical_required_body(&entry_fn.body) else {
            return Ok(Err(prepared));
        };
        let mut key_inputs = Vec::with_capacity(prepared.inputs.len());
        for (local, domain, value) in &prepared.inputs {
            let Some(value) = value else {
                return Ok(Err(prepared));
            };
            if !is_safe_memo_key_term(domain, 0) || !is_safe_memo_key_term(value, 0) {
                return Ok(Err(prepared));
            }
            key_inputs.push((*local, domain.clone(), value.clone()));
        }
        if !is_safe_memo_key_term(&prepared.result_type, 0) {
            return Ok(Err(prepared));
        }
        let scope = scope_for_required(package, prepared.entry, protocols);
        let mut op_hash = RequiredHashWriter::new();
        op_hash.str(&canonical_body);
        let query = QueryId {
            package: package_id,
            declaration: prepared.entry.local,
            operation: op_hash.0 as u32,
        };
        let key = RequiredMemoKey {
            canonical_body,
            inputs: key_inputs,
            result_type: prepared.result_type.clone(),
        };
        (query, key, scope)
    };
    let mut pending_prepared = Some(prepared);
    let mut published_out = None;
    let cached = REQUIRED_MEMO_CACHE.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else {
            return Ok(Err(pending_prepared
                .take()
                .expect("pending prepared present on borrow contention")));
        };
        if state.cache.scope() != scope {
            state.invalidations += 1;
            state.cache.set_scope(scope);
        }
        let available = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut context = Context::new(available);
        let state_ref = &mut *state;
        let outcome = state_ref.cache.query_pure_admitted(
            query,
            vec![key],
            &mut context,
            &mut state_ref.protocol,
            |_, context| {
                let prepared = pending_prepared
                    .take()
                    .expect("pure memo miss invokes prepared once");
                let evaluated = context
                    .with_allowance(usize::MAX, |remaining| {
                        let mut eval_steps = *remaining as u64;
                        let result = prepared.invoke_publishing(
                            Vec::new(),
                            &mut eval_steps,
                            span,
                            package_id,
                            identities.clone(),
                            None,
                        );
                        *remaining = usize::try_from(eval_steps).unwrap_or(usize::MAX);
                        result
                    })
                    .map_err(comptime_executor::Error::Evaluation)?;
                let (result, definitions) = evaluated;
                let publish = definitions.is_empty() && is_exportable_memo_result(&result.0);
                if !definitions.is_empty() {
                    published_out = Some(definitions);
                }
                Ok((result, publish))
            },
        );
        *steps = context.remaining() as u64;
        match outcome {
            Ok(outcome) => {
                if outcome.reused {
                    state.hits += 1;
                } else {
                    state.misses += 1;
                }
                Ok(Ok((*outcome.value).clone()))
            }
            Err(comptime_executor::Error::Evaluation(err)) => Err(err),
            Err(comptime_executor::Error::BudgetExhausted) => Err(error(
                span,
                "required comptime memoization step budget exhausted",
            )),
            Err(other) => Err(error(
                span,
                format!("required comptime memoization error: {other}"),
            )),
        }
    })?;
    Ok(cached.map(|result| (result, published_out.unwrap_or_default())))
}

#[cfg(test)]
mod memoization_tests {
    use super::*;
    use crate::program::Protocols;
    use n::{ExprKind as K, Function, FunctionDeclaration, FunctionPhase, Literal, SourceRange};
    use nir::{Primitive as P, Sort};

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 7, local }
    }

    fn fn_decl(declaration: DeclarationId, pure: bool, body: n::Expr) -> Function {
        Function {
            interface: FunctionDeclaration {
                source: SourceRange { start: 0, end: 0 },
                declaration,
                name: format!("f{}", declaration.local),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(P::Bool)),
                },
                parameters: vec![],
                env_parameters: vec![],
                defaults: vec![],
            },
            body,
            runtime_deliveries: Default::default(),
        }
    }

    #[test]
    fn required_memo_cache_hits_on_identical_pure_queries_and_invalidates_on_scope() {
        let entry = id(99);
        let helper = id(1);
        let src1 = SourceRange { start: 10, end: 20 };
        let src2 = SourceRange { start: 40, end: 50 };
        let make_call = |source| n::Expr {
            source,
            ty: Term::Primitive(P::Bool),
            kind: K::Call {
                parameter_bindings: vec![],
                callee: Box::new(n::Expr {
                    source,
                    ty: Term::Function {
                        parameters: vec![],
                        result: Box::new(Term::Primitive(P::Bool)),
                    }
                    .with_comptime_phase(true),
                    kind: K::Function(helper),
                }),
                arguments: vec![],
                env_arguments: vec![],
            },
        };
        let make_pkg = |pure: bool, helper_val: bool, target| n::Package {
            target,
            functions: vec![fn_decl(
                helper,
                pure,
                n::Expr {
                    source: SourceRange { start: 1, end: 5 },
                    ty: Term::Primitive(P::Bool),
                    kind: K::Literal(Literal::Bool(helper_val)),
                },
            )],
            ..Default::default()
        };

        let pkg_a = make_pkg(true, true, None);
        let pkg_b = make_pkg(true, false, None);
        let pkg_target = make_pkg(true, true, Some(nir::target::TargetData::new(64).unwrap()));
        let pkg_impure = make_pkg(false, true, None);
        let protocols = Protocols::default();

        let call_a = make_call(src1);
        let call_b = make_call(src2);
        assert!(is_pure_cacheable_expr(&call_a, &pkg_a));
        assert!(!is_pure_cacheable_expr(&call_a, &pkg_impure));
        assert_eq!(
            canonical_required_body(&call_a),
            canonical_required_body(&call_b)
        );

        let scope_a = scope_for_required(&pkg_a, entry, &protocols);
        let scope_b = scope_for_required(&pkg_b, entry, &protocols);
        let scope_target = scope_for_required(&pkg_target, entry, &protocols);
        assert_ne!(scope_a.definitions, scope_b.definitions);
        assert_ne!(scope_a.target, scope_target.target);

        let mut proto = RequiredMemoProtocol;
        let mut cache = Cache::new(scope_a, 8);
        let key = RequiredMemoKey {
            canonical_body: canonical_required_body(&call_a).unwrap(),
            inputs: vec![],
            result_type: Term::Primitive(P::Bool),
        };
        let qid = QueryId {
            package: 7,
            declaration: entry.local,
            operation: 1,
        };
        let mut ctx = Context::new(10_000);
        let first = cache
            .query_pure_admitted(qid, vec![key.clone()], &mut ctx, &mut proto, |_, _| {
                Ok(((Value::Bool(true), Term::Primitive(P::Bool)), true))
            })
            .unwrap();
        assert!(!first.reused);
        let second = cache
            .query_pure_admitted(qid, vec![key.clone()], &mut ctx, &mut proto, |_, _| {
                panic!("must hit cache on identical scope and query")
            })
            .unwrap();
        assert!(second.reused);
        assert_eq!(cache.len(), 1);

        cache.set_scope(scope_b);
        assert_eq!(cache.len(), 0);
        let third = cache
            .query_pure_admitted(qid, vec![key], &mut ctx, &mut proto, |_, _| {
                Ok(((Value::Bool(false), Term::Primitive(P::Bool)), true))
            })
            .unwrap();
        assert!(!third.reused);
        assert_eq!(third.value.0, Value::Bool(false));

        // Reordered named arguments and distinct LocalId owners must not collide.
        let make_sub_call = |first_param: usize, second_param: usize| n::Expr {
            source: src1,
            ty: Term::Primitive(P::Bool),
            kind: K::Call {
                parameter_bindings: vec![
                    n::LocalId { owner: 1, index: 0 },
                    n::LocalId { owner: 1, index: 1 },
                ],
                callee: Box::new(n::Expr {
                    source: src1,
                    ty: Term::Function {
                        parameters: vec![],
                        result: Box::new(Term::Primitive(P::Bool)),
                    }
                    .with_comptime_phase(true),
                    kind: K::Function(helper),
                }),
                arguments: vec![
                    n::CallArgument {
                        passing: n::ArgumentPassing::Value,
                        parameter: first_param,
                        value: n::Expr {
                            source: src1,
                            ty: Term::Primitive(P::Bool),
                            kind: K::Literal(Literal::Bool(true)),
                        },
                        is_default: false,
                    },
                    n::CallArgument {
                        passing: n::ArgumentPassing::Value,
                        parameter: second_param,
                        value: n::Expr {
                            source: src1,
                            ty: Term::Primitive(P::Bool),
                            kind: K::Literal(Literal::Bool(false)),
                        },
                        is_default: false,
                    },
                ],
                env_arguments: vec![],
            },
        };
        assert_ne!(
            canonical_required_body(&make_sub_call(0, 1)),
            canonical_required_body(&make_sub_call(1, 0))
        );
        let local_owner_1 = n::Expr {
            source: src1,
            ty: Term::Primitive(P::Bool),
            kind: K::Local(n::LocalId { owner: 1, index: 0 }),
        };
        let local_owner_2 = n::Expr {
            source: src1,
            ty: Term::Primitive(P::Bool),
            kind: K::Local(n::LocalId { owner: 2, index: 0 }),
        };
        assert_ne!(
            canonical_required_body(&local_owner_1),
            canonical_required_body(&local_owner_2)
        );
        let key_owner_1 = RequiredMemoKey {
            canonical_body: "body".into(),
            inputs: vec![(
                n::LocalId { owner: 1, index: 0 },
                Term::Primitive(P::Bool),
                Term::Bool(true),
            )],
            result_type: Term::Primitive(P::Bool),
        };
        let key_owner_2 = RequiredMemoKey {
            canonical_body: "body".into(),
            inputs: vec![(
                n::LocalId { owner: 2, index: 0 },
                Term::Primitive(P::Bool),
                Term::Bool(true),
            )],
            result_type: Term::Primitive(P::Bool),
        };
        assert!(key_owner_1 != key_owner_2);

        // Conservative rejection of intrinsics and helper bodies requiring data::Reader.
        let intrinsic_expr = n::Expr {
            source: src1,
            ty: Term::Primitive(P::Bool),
            kind: K::Intrinsic {
                tag: nir::intrinsics::Tag {
                    family: id(2),
                    variant: 0,
                },
                binding: None,
                inputs: vec![],
            },
        };
        assert!(!is_pure_cacheable_expr(&intrinsic_expr, &pkg_a));
        let attr_helper_pkg = n::Package {
            functions: vec![fn_decl(
                helper,
                true,
                n::Expr {
                    source: src1,
                    ty: Term::Primitive(P::Bool),
                    kind: K::DefinitionAttributes(Box::new(n::Expr {
                        source: src1,
                        ty: Term::Primitive(P::Type),
                        kind: K::TypeValue(Term::Primitive(P::U32)),
                    })),
                },
            )],
            ..Default::default()
        };
        assert!(
            attr_helper_pkg
                .functions
                .iter()
                .any(|f| !is_pure_cacheable_expr(&f.body, &attr_helper_pkg))
        );
        let _ = Sort::Type;
    }
}
