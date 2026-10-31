//! Evidence-driven associated Type queries in an immutable, admitted world.
//! One-step queries retain residuals; recursive normalization closes projections
//! transactionally with cycle detection and a query-local completed cache.
//! Substitution never invokes normal-form heads or substitutes trait defaults.
use crate::capabilities::{Qualification, World};
use crate::implementation::{Selection, TypeAssessment, TypeEvidence};
use crate::ptc::{Budget, Formula, Outcome};
use nir::program::{ExprKind, InterfaceInstance};
use nir::{Primitive, Term};
use std::fmt;

/// The selected result and its exact implementation substitution travel
/// together. Evidence is valid only in the immutable world used for this query.
#[derive(Clone, Debug)]
pub struct AssociatedTypeResolution {
    pub value: Term,
    pub evidence: TypeEvidence,
    pub qualification: Qualification,
}

impl World<'_> {
    /// Resolve a well-scoped Type projection after its input substitutions.
    /// Member lookup has already chosen an origin (including inherited origins).
    /// The caller must establish any requested child view separately.
    pub fn resolve_associated_type(
        &self,
        projection: &Term,
        budget: Budget,
    ) -> Result<AssociatedTypeResolution, String> {
        self.resolve_associated_type_budgeted(projection, budget)
            .map(|(result, _)| result)
    }

    fn resolve_associated_type_budgeted(
        &self,
        projection: &Term,
        budget: Budget,
    ) -> Result<(AssociatedTypeResolution, usize), String> {
        self.resolve_associated_type_using(
            projection,
            budget,
            &mut |interface, subject, allowance| {
                self.resolve_projected_instance_metered(interface, subject, allowance)
            },
        )
    }

    fn resolve_associated_type_using(
        &self,
        projection: &Term,
        budget: Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, String>,
    ) -> Result<(AssociatedTypeResolution, usize), String> {
        let mut fuel = budget.steps;
        charge_projection_copy(projection, &mut fuel)?;
        projection.check_scope(&[]).map_err(|e| e.to_string())?;
        let Term::AssociatedType {
            origin,
            subject,
            arguments,
        } = projection
        else {
            return Err("associated Type query requires a projection".into());
        };
        let contract = self
            .traits
            .iter()
            .find(|t| t.declaration == origin.interface)
            .ok_or("associated Type query has no originating interface")?;
        if arguments.len() != contract.type_parameters.len() {
            return Err("associated Type query interface argument count mismatch".into());
        }
        let member = contract
            .associated
            .iter()
            .find(|m| m.declaration == origin.member)
            .ok_or("associated Type query member does not belong to its interface")?;
        let parameters = contract.binders_with_self(&mut fuel)?;
        let inputs: Vec<_> = std::iter::once(subject.as_ref().clone())
            .chain(arguments.iter().cloned())
            .collect();
        charge_projection_substitution(&member.ty, &parameters, &inputs, &mut fuel)?;
        let substitution = contract
            .input_substitution(&inputs, &[], &mut fuel)
            .map_err(|e| e.to_string())?;
        let member_type = substitution
            .instantiate_bounded(&member.ty, &mut fuel)
            .map_err(|e| e.to_string())?;
        if member_type != Term::Primitive(Primitive::Type) {
            return Err("associated projection member is not Type-valued".into());
        }
        // Reserve a disjoint allowance for each proof/search rather than
        // resetting the full caller budget at every formation obligation.
        let queries = contract
            .requirements
            .len()
            .saturating_add(contract.capabilities.len().saturating_mul(2))
            .saturating_add(1);
        let query_steps = fuel / queries.saturating_mul(2);
        if query_steps == 0 {
            return Err("associated Type query budget exhausted".into());
        }
        fuel -= query_steps * queries;
        let query_budget = Budget {
            steps: query_steps,
            ..budget
        };
        // Interface-parameter constraints are formation obligations, not
        // assumptions supplied by selecting an implementation.
        for requirement in &contract.requirements {
            let mut pending = vec![(requirement, 0usize)];
            while let Some((requirement, depth)) = pending.pop() {
                if depth >= budget.max_depth.min(64) {
                    return Err("associated Type requirement depth exhausted".into());
                }
                fuel = fuel
                    .checked_sub(1)
                    .ok_or("associated Type query budget exhausted")?;
                match requirement {
                    nir::program::TypePredicate::Trait(requirement) => {
                        charge_projection_substitution(
                            &requirement.subject,
                            &parameters,
                            &inputs,
                            &mut fuel,
                        )?;
                        for argument in &requirement.interface_arguments {
                            charge_projection_substitution(
                                argument,
                                &parameters,
                                &inputs,
                                &mut fuel,
                            )?;
                        }
                    }
                    nir::program::TypePredicate::Equal(a, b) => {
                        charge_projection_substitution(a, &parameters, &inputs, &mut fuel)?;
                        charge_projection_substitution(b, &parameters, &inputs, &mut fuel)?;
                    }
                    nir::program::TypePredicate::And(items)
                    | nir::program::TypePredicate::Or(items) => {
                        if items.len() > fuel {
                            return Err("associated Type query budget exhausted".into());
                        }
                        pending.extend(items.iter().map(|item| (item, depth + 1)));
                    }
                    nir::program::TypePredicate::Not(inner) => pending.push((inner, depth + 1)),
                    nir::program::TypePredicate::Bool(_) => (),
                }
            }
            let requirement = substitution
                .instantiate_predicate_bounded(requirement, &mut fuel)
                .map_err(|e| e.to_string())?;
            if !matches!(
                self.context
                    .prove(Formula::from_predicate(&requirement), query_budget),
                Outcome::Entailed(_)
            ) {
                return Err("associated Type interface requirement is not established".into());
            }
        }
        for capability in &contract.capabilities {
            charge_projection_substitution(&capability.subject, &parameters, &inputs, &mut fuel)?;
            for argument in &capability.interface_arguments {
                charge_projection_substitution(argument, &parameters, &inputs, &mut fuel)?;
            }
            let capability = substitution
                .instantiate_requirement_bounded(capability, &mut fuel)
                .map_err(|e| e.to_string())?;
            if self.given(&capability, query_budget).is_some() {
                continue;
            }
            let mut allowance = query_budget;
            let resolved = resolve(
                &capability.interface_instance(),
                &capability.subject,
                &mut allowance,
            )?;
            fuel += allowance.steps;
            if !matches!(resolved.types.selection, Selection::Selected(_)) {
                return Err("associated Type interface capability is not established".into());
            }
        }
        let interface = InterfaceInstance {
            declaration: origin.interface,
            arguments: arguments.clone(),
        };
        // Formation is complete. The final selection can use the remaining
        // work, then return its unused allowance for evidence validation and
        // later projections. Repeatedly halving here starves nested adapters
        // even when the preceding formation checks are inexpensive.
        let mut allowance = Budget {
            steps: fuel + query_budget.steps,
            ..budget
        };
        let resolution = resolve(&interface, subject, &mut allowance)?;
        fuel = allowance.steps;
        let Selection::Selected(selected) = resolution.types.selection else {
            let pending = resolution
                .qualifications
                .iter()
                .filter_map(|(id, qualification)| {
                    matches!(qualification, Qualification::Pending(_))
                        .then(|| format!("(candidate {id} {qualification})"))
                })
                .collect::<Vec<_>>()
                .join(" ");
            return Err(format!(
                "associated Type implementation selection is unresolved: {} (remaining-steps {fuel}) (pending {pending})",
                resolution.types.selection
            ));
        };
        let Some(TypeAssessment::Applicable(evidence)) =
            resolution.types.assessments.get(&selected)
        else {
            return Err("associated Type selection lacks type evidence".into());
        };
        let Some(qualification @ Qualification::Proved(_)) =
            resolution.qualifications.get(&selected)
        else {
            return Err("associated Type selection lacks capability evidence".into());
        };
        let implementation = self
            .implementations
            .iter()
            .find(|i| i.declaration == selected)
            .ok_or("associated Type selection has no implementation contract")?;
        let mut values = implementation
            .associated
            .iter()
            .filter(|(id, _)| *id == origin.member);
        let (_, value) = values
            .next()
            .ok_or("selected implementation has no associated Type value")?;
        if values.next().is_some() {
            return Err("selected implementation has duplicate associated Type values".into());
        }
        let binders = implementation
            .parameter_binders()
            .map_err(|e| e.to_string())?;
        nir::copy_budget::charge_substitution(value, &binders, &evidence.type_arguments, &mut fuel)
            .map_err(|_| "associated Type substitution budget exhausted")?;
        let substitution = nir::DeclarationSubstitution::bounded(
            &implementation.type_parameters,
            &implementation.parameter_types,
            &evidence.type_arguments,
            &[],
            &mut fuel,
        )
        .map_err(|e| e.to_string())?;
        let mut completed = value.clone();
        completed
            .rewrite_types(&mut |ty| substitution.instantiate_bounded(ty, &mut fuel))
            .map_err(|e| e.to_string())?;
        if completed.ty != Term::Primitive(Primitive::Type) {
            return Err("selected associated value has a non-Type contract".into());
        }
        let ExprKind::TypeValue(value) = completed.kind else {
            return Err("selected associated value is not a completed Type template".into());
        };
        charge_projection_copy(&value, &mut fuel)?;
        value.check_scope(&[]).map_err(|e| e.to_string())?;
        if value.sort() != nir::Sort::Type {
            return Err("selected associated Type template has the wrong sort".into());
        }
        Ok((
            AssociatedTypeResolution {
                value,
                evidence: evidence.clone(),
                qualification: qualification.clone(),
            },
            fuel,
        ))
    }
}

impl fmt::Display for AssociatedTypeResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-type-resolution (value {}) (implementation {}) (arguments",
            self.value, self.evidence.implementation
        )?;
        for argument in &self.evidence.type_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (qualification {}))", self.qualification)
    }
}

/// Completed projection closure plus its deterministic, dependency-ordered
/// selection trace. No partial result or cross-world cache escapes on failure.
#[derive(Clone, Debug)]
pub struct AssociatedTypeNormalization {
    pub value: Term,
    pub steps: Vec<AssociatedTypeStep>,
}
#[derive(Clone, Debug)]
pub struct AssociatedTypeStep {
    pub projection: Term,
    pub resolution: AssociatedTypeResolution,
}

impl World<'_> {
    /// Resolve projections throughout type structure after caller substitution.
    /// Opaque normal-form heads are retained; only their inputs are traversed.
    /// A projection still depending on a function-local binder cannot be chosen
    /// here and must wait for that binder's instantiation.
    pub fn normalize_associated_types(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<AssociatedTypeNormalization, String> {
        self.normalize_associated_types_budgeted(ty, budget)
            .map(|(result, _)| result)
    }

    pub(crate) fn normalize_associated_types_budgeted(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<(AssociatedTypeNormalization, usize), String> {
        self.normalize_associated_types_mode(ty, budget, false, None, None)
            .map(|(result, remaining, _)| (result, remaining))
    }

    /// The caller owns this cache for one immutable implementation world.
    /// Publish newly completed projections only after the whole query succeeds.
    pub fn normalize_associated_types_cached(
        &self,
        ty: &Term,
        budget: Budget,
        cache: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<(AssociatedTypeNormalization, usize), String> {
        let (result, remaining, completed) =
            self.normalize_associated_types_mode(ty, budget, false, Some(cache), None)?;
        cache.extend(completed);
        Ok((result, remaining))
    }

    /// Definition-side type view: given interfaces remain abstract, while
    /// independent concrete projections may still resolve. IIR uses the strict
    /// normalization entry and cannot retain these abstract results.
    pub fn associated_type_view(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<AssociatedTypeNormalization, String> {
        self.associated_type_view_metered(ty, &mut { budget })
    }

    pub fn associated_type_view_metered(
        &self,
        ty: &Term,
        budget: &mut Budget,
    ) -> Result<AssociatedTypeNormalization, String> {
        let result = self.normalize_associated_types_mode(ty, *budget, true, None, None);
        match result {
            Ok((value, remaining, _)) => {
                budget.steps = remaining;
                Ok(value)
            }
            Err(error) => {
                budget.steps = 0;
                Err(error)
            }
        }
    }

    pub fn associated_type_view_cached(
        &self,
        ty: &Term,
        budget: Budget,
        cache: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<(AssociatedTypeNormalization, usize), String> {
        let (value, remaining, completed) =
            self.normalize_associated_types_mode(ty, budget, true, Some(cache), None)?;
        cache.extend(completed);
        Ok((value, remaining))
    }

    pub(crate) fn associated_type_view_for_selection(
        &self,
        ty: &Term,
        budget: Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, String>,
    ) -> Result<(Term, usize), String> {
        self.normalize_associated_types_mode(ty, budget, true, None, Some(resolve))
            .map(|(result, remaining, _)| (result.value, remaining))
    }

    fn normalize_associated_types_mode(
        &self,
        ty: &Term,
        budget: Budget,
        preserve_given: bool,
        shared: Option<&std::collections::HashMap<Term, Term>>,
        resolve: Option<
            &mut dyn FnMut(
                &InterfaceInstance,
                &Term,
                &mut Budget,
            ) -> Result<crate::capabilities::Resolution, String>,
        >,
    ) -> Result<
        (
            AssociatedTypeNormalization,
            usize,
            std::collections::HashMap<Term, Term>,
        ),
        String,
    > {
        let mut normalizer = Normalizer {
            world: self,
            resolve,
            preserve_given,
            budget,
            remaining: budget.steps,
            active: std::collections::HashSet::new(),
            completed: std::collections::HashMap::new(),
            shared,
            steps: Vec::new(),
            frames: if preserve_given {
                self.context.scope().to_vec()
            } else {
                Vec::new()
            },
        };
        normalizer.charge_copy(ty)?;
        ty.check_scope(&normalizer.frames)
            .map_err(|e| e.to_string())?;
        if ty.sort() != nir::Sort::Type {
            return Err("associated normalization requires a Type".into());
        }
        let value = normalizer.normalize(ty, 0)?;
        value
            .check_scope(&normalizer.frames)
            .map_err(|e| e.to_string())?;
        Ok((
            AssociatedTypeNormalization {
                value,
                steps: normalizer.steps,
            },
            normalizer.remaining,
            normalizer.completed,
        ))
    }
}

struct Normalizer<'a, 'w, 'r> {
    resolve: Option<
        &'r mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, String>,
    >,
    world: &'a World<'w>,
    preserve_given: bool,
    budget: Budget,
    remaining: usize,
    active: std::collections::HashSet<Term>,
    completed: std::collections::HashMap<Term, Term>,
    shared: Option<&'a std::collections::HashMap<Term, Term>>,
    steps: Vec<AssociatedTypeStep>,
    frames: Vec<Vec<nir::Sort>>,
}
impl Normalizer<'_, '_, '_> {
    fn spend(&mut self, steps: usize) -> Result<(), String> {
        self.remaining = self
            .remaining
            .checked_sub(steps)
            .ok_or("associated normalization budget exhausted")?;
        Ok(())
    }
    fn charge_copy(&mut self, term: &Term) -> Result<(), String> {
        charge_projection_copy(term, &mut self.remaining)
    }
    fn many(&mut self, terms: &[Term], depth: usize) -> Result<Vec<Term>, String> {
        if terms.len() > self.remaining {
            return Err("associated normalization budget exhausted".into());
        }
        terms
            .iter()
            .map(|term| self.normalize(term, depth))
            .collect()
    }
    fn normalize(&mut self, term: &Term, depth: usize) -> Result<Term, String> {
        if depth >= self.budget.max_depth.min(64) {
            return Err("associated normalization depth exhausted".into());
        }
        self.spend(1)?;
        Ok(match term {
            Term::ErrorResult { errors, result } => Term::error_result(
                self.many(errors, depth + 1)?,
                self.normalize(result, depth + 1)?,
            ),
            Term::Computation { effects, result } => {
                self.spend(effects.len())?;
                let operations = effects
                    .iter()
                    .map(|operation| {
                        Ok(nir::effects::Operation {
                            declaration: operation.declaration,
                            arguments: self.many(&operation.arguments, depth + 1)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Term::computation(operations, self.normalize(result, depth + 1)?)
            }
            Term::AssociatedType {
                origin,
                arguments,
                subject,
            } => {
                let subject = self.normalize(subject, depth + 1)?;
                let arguments = self.many(arguments, depth + 1)?;
                let key = Term::AssociatedType {
                    origin: origin.clone(),
                    arguments,
                    subject: Box::new(subject),
                };
                if self.preserve_given && key.check_scope(&[]).is_err() {
                    // A projection can capture any enclosing callable frame,
                    // not just its immediate depth-zero frame. Validate it in
                    // the complete telescope before retaining it uncached.
                    key.check_scope(&self.frames).map_err(|e| e.to_string())?;
                    // A callable's ordinary inputs have not been supplied yet.
                    // Retain the projection without choosing an implementation
                    // or manufacturing an equality. Formation evidence is
                    // checked by elaboration; strict IIR normalization still
                    // requires a selected concrete projection.
                    let Term::AssociatedType {
                        origin,
                        subject,
                        arguments,
                    } = &key
                    else {
                        unreachable!()
                    };
                    let contract = self
                        .world
                        .traits
                        .iter()
                        .find(|t| t.declaration == origin.interface)
                        .ok_or("associated Type query has no originating interface")?;
                    if arguments.len() != contract.type_parameters.len() {
                        return Err(
                            "associated Type query interface argument count mismatch".into()
                        );
                    }
                    let member = contract
                        .associated
                        .iter()
                        .find(|m| m.declaration == origin.member)
                        .ok_or("associated Type query member does not belong to its interface")?;
                    self.charge_copy(&member.ty)?;
                    let inputs = std::iter::once(subject.as_ref().clone())
                        .chain(arguments.iter().cloned())
                        .collect::<Vec<_>>();
                    let binders = contract.binders_with_self(&mut self.remaining)?;
                    charge_projection_substitution(
                        &member.ty,
                        &binders,
                        &inputs,
                        &mut self.remaining,
                    )?;
                    let ty = contract
                        .input_substitution(&inputs, &self.frames, &mut self.remaining)
                        .and_then(|substitution| {
                            substitution.instantiate_bounded(&member.ty, &mut self.remaining)
                        })
                        .map_err(|e| e.to_string())?;
                    if ty != Term::Primitive(Primitive::Type) {
                        return Err("associated projection member is not Type-valued".into());
                    }
                    return Ok(key);
                }
                if let Some(value) = self
                    .completed
                    .get(&key)
                    .or_else(|| self.shared.and_then(|cache| cache.get(&key)))
                {
                    // Use a temporary borrowed walk before cloning the cached
                    // result; the cache is local to this immutable-world call.
                    charge_projection_copy(value, &mut self.remaining)?;
                    return Ok(value.clone());
                }
                self.charge_copy(&key)?;
                if self.preserve_given && !self.world.assumptions.is_empty() {
                    let Term::AssociatedType {
                        origin,
                        subject,
                        arguments,
                    } = &key
                    else {
                        unreachable!()
                    };
                    let query_steps = self.remaining / 2;
                    if query_steps == 0 {
                        return Err("associated view budget exhausted".into());
                    }
                    let given = nir::program::TraitRequirement {
                        subject: subject.as_ref().clone(),
                        interface: origin.interface,
                        interface_arguments: arguments.clone(),
                    };
                    let mut given_budget = Budget {
                        steps: query_steps,
                        ..self.budget
                    };
                    let found = self.world.given_metered(&given, &mut given_budget);
                    // A failed premise lookup may consume very little work.
                    // Charging its whole allowance starves nested projections.
                    self.spend(query_steps - given_budget.steps)?;
                    if found.is_some() {
                        let contract = self
                            .world
                            .traits
                            .iter()
                            .find(|t| t.declaration == origin.interface)
                            .ok_or("associated Type query has no originating interface")?;
                        if arguments.len() != contract.type_parameters.len() {
                            return Err(
                                "associated Type query interface argument count mismatch".into()
                            );
                        }
                        let member = contract
                            .associated
                            .iter()
                            .find(|m| m.declaration == origin.member)
                            .ok_or(
                                "associated Type query member does not belong to its interface",
                            )?;
                        self.charge_copy(&member.ty)?;
                        let inputs: Vec<_> = std::iter::once(subject.as_ref().clone())
                            .chain(arguments.iter().cloned())
                            .collect();
                        let binders = contract.binders_with_self(&mut self.remaining)?;
                        charge_projection_substitution(
                            &member.ty,
                            &binders,
                            &inputs,
                            &mut self.remaining,
                        )?;
                        let ty = contract
                            .input_substitution(&inputs, &[], &mut self.remaining)
                            .and_then(|substitution| {
                                substitution.instantiate_bounded(&member.ty, &mut self.remaining)
                            })
                            .map_err(|e| e.to_string())?;
                        if ty != Term::Primitive(Primitive::Type) {
                            return Err("associated projection member is not Type-valued".into());
                        }
                        self.charge_copy(&key)?;
                        self.charge_copy(&key)?;
                        self.completed.insert(key.clone(), key.clone());
                        return Ok(key);
                    }
                }
                if !self.active.insert(key.clone()) {
                    return Err("cyclic associated Type projection".into());
                }
                let result = (|| {
                    let allowance = Budget {
                        steps: self.remaining,
                        ..self.budget
                    };
                    let resolved = if let Some(resolve) = self.resolve.as_deref_mut() {
                        self.world
                            .resolve_associated_type_using(&key, allowance, resolve)
                    } else {
                        self.world.resolve_associated_type_budgeted(&key, allowance)
                    };
                    let (resolution, remaining) = resolved.map_err(|message| {
                        format!("associated Type normalization of {key}: {message}")
                    })?;
                    self.remaining = remaining;
                    let value = self.normalize(&resolution.value, depth + 1)?;
                    self.charge_copy(&value)?;
                    self.charge_copy(&key)?;
                    self.charge_copy(&key)?;
                    self.completed.insert(key.clone(), value.clone());
                    self.steps.push(AssociatedTypeStep {
                        projection: key.clone(),
                        resolution,
                    });
                    Ok(value)
                })();
                self.active.remove(&key);
                return result;
            }
            Term::Nominal {
                declaration,
                arguments,
            } => Term::Nominal {
                declaration: *declaration,
                arguments: self.many(arguments, depth + 1)?,
            },
            Term::NormalForm {
                head,
                sort,
                arguments,
            } => Term::NormalForm {
                head: *head,
                sort: *sort,
                arguments: self.many(arguments, depth + 1)?,
            },
            Term::Tuple(items) => Term::Tuple(self.many(items, depth + 1)?),
            Term::TupleValue(items) => Term::TupleValue(self.many(items, depth + 1)?),
            Term::ListConcat { ty, parts } => {
                Term::list_concat(self.normalize(ty, depth + 1)?, self.many(parts, depth + 1)?)
                    .map_err(|error| error.to_string())?
            }
            Term::OptionalValue { ty, value } => Term::OptionalValue {
                ty: Box::new(self.normalize(ty, depth + 1)?),
                value: value
                    .as_ref()
                    .map(|value| self.normalize(value, depth + 1).map(Box::new))
                    .transpose()?,
            },
            Term::ErrorValue { ty, label, value } => {
                let Term::ErrorResult { errors, result } = ty.as_ref() else {
                    return Err("invalid frozen error result domain".into());
                };
                Term::error_value(
                    self.many(errors, depth + 1)?,
                    self.normalize(result, depth + 1)?,
                    match label {
                        Some(label) => Some(Box::new(self.normalize(label, depth + 1)?)),
                        None => None,
                    },
                    self.normalize(value, depth + 1)?,
                )
                .map_err(|error| error.to_string())?
            }
            Term::NominalValue {
                ty,
                variant,
                fields,
            } => Term::NominalValue {
                ty: Box::new(self.normalize(ty, depth + 1)?),
                variant: *variant,
                fields: self.many(fields, depth + 1)?,
            },
            Term::Record(fields) | Term::RecordValue(fields) => {
                if fields.len() > self.remaining {
                    return Err("associated normalization budget exhausted".into());
                }
                let mut output = Vec::with_capacity(fields.len());
                for (name, ty) in fields {
                    self.spend(name.len())?;
                    output.push((name.clone(), self.normalize(ty, depth + 1)?));
                }
                if matches!(term, Term::RecordValue(_)) {
                    Term::RecordValue(output)
                } else {
                    Term::Record(output)
                }
            }
            Term::Optional(inner) => Term::Optional(Box::new(self.normalize(inner, depth + 1)?)),
            Term::Pointer(inner) => Term::Pointer(Box::new(self.normalize(inner, depth + 1)?)),
            Term::NormalFormFunction(inner, collector) => {
                Term::NormalFormFunction(Box::new(self.normalize(inner, depth + 1)?), *collector)
            }
            Term::ComptimeFunction(inner) => {
                Term::ComptimeFunction(Box::new(self.normalize(inner, depth + 1)?))
            }
            Term::UnsafeFunction(inner) => {
                Term::UnsafeFunction(Box::new(self.normalize(inner, depth + 1)?))
            }
            Term::Function { parameters, result } => {
                if parameters.len() > self.remaining {
                    return Err("associated normalization budget exhausted".into());
                }
                self.frames
                    .push(parameters.iter().map(|p| p.sort).collect());
                let normalized = (|| {
                    let parameters = parameters
                        .iter()
                        .map(|p| {
                            Ok(nir::Parameter {
                                sort: p.sort,
                                ty: self.normalize(&p.ty, depth + 1)?,
                            })
                        })
                        .collect::<Result<_, String>>()?;
                    Ok::<_, String>(Term::Function {
                        parameters,
                        result: Box::new(self.normalize(result, depth + 1)?),
                    })
                })();
                self.frames.pop();
                normalized?
            }
            Term::Closure {
                environment,
                signature,
            } => Term::Closure {
                environment: self.many(environment, depth + 1)?,
                signature: Box::new(self.normalize(signature, depth + 1)?),
            },
            Term::Float(value) => {
                self.spend(
                    value
                        .digits()
                        .len()
                        .saturating_add(value.exponent().bit_len().div_ceil(32)),
                )?;
                term.clone()
            }
            Term::Integer(value) => {
                self.spend(value.bit_len().div_ceil(32))?;
                term.clone()
            }
            Term::BitVec(value) => {
                self.spend(value.words().len())?;
                term.clone()
            }
            Term::String(value) | Term::Symbol(value) => {
                self.spend(value.len())?;
                term.clone()
            }
            _ => term.clone(),
        })
    }
}
impl fmt::Display for AssociatedTypeStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-type-step (projection {}) (resolution {}))",
            self.projection, self.resolution
        )
    }
}
impl fmt::Display for AssociatedTypeNormalization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-type-normalization (value {}) (steps",
            self.value
        )?;
        for step in &self.steps {
            write!(f, " {step}")?;
        }
        f.write_str("))")
    }
}

// Cache hits must pay for payload copying too, not only the node count. The
// preliminary shape walk bounds recursive clone depth before visiting payloads.
fn charge_projection_substitution(
    term: &Term,
    parameters: &[nir::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), String> {
    nir::type_computations::charge_parameter_substitution(term, parameters, arguments, fuel)
        .map_err(|e| e.to_string())
}

pub fn charge_projection_copy(term: &Term, fuel: &mut usize) -> Result<(), String> {
    nir::type_budget::charge_structure(term, fuel)
        .map_err(|_| "memory summary type budget exhausted".to_string())?;
    let mut pending = vec![term];
    while let Some(term) = pending.pop() {
        let cost = match term {
            Term::ErrorResult { errors, result } => {
                pending.push(result);
                pending.extend(errors);
                0
            }
            Term::Computation { effects, result } => {
                pending.push(result);
                for operation in effects {
                    pending.extend(&operation.arguments);
                }
                effects.len()
            }
            Term::AssociatedType {
                arguments, subject, ..
            } => {
                pending.extend(arguments);
                pending.push(subject);
                0
            }
            Term::Tuple(items)
            | Term::TupleValue(items)
            | Term::Nominal {
                arguments: items, ..
            }
            | Term::NormalForm {
                arguments: items, ..
            } => {
                pending.extend(items);
                0
            }
            Term::OptionalValue { ty, value } => {
                pending.push(ty);
                if let Some(value) = value {
                    pending.push(value);
                }
                0
            }
            Term::ErrorValue { ty, label, value } => {
                pending.push(ty);
                if let Some(label) = label {
                    pending.push(label);
                }
                pending.push(value);
                0
            }
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
                pending.push(ty);
                pending.extend(fields);
                0
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                let mut cost = 0usize;
                for (name, ty) in fields {
                    cost = cost
                        .checked_add(name.len())
                        .ok_or("associated copy budget exhausted")?;
                    pending.push(ty);
                }
                cost
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::NormalFormFunction(inner, _)
            | Term::UnsafeFunction(inner) => {
                pending.push(inner);
                0
            }
            Term::Function { parameters, result } => {
                pending.extend(parameters.iter().map(|p| &p.ty));
                pending.push(result);
                0
            }
            Term::Closure {
                environment,
                signature,
            } => {
                pending.extend(environment);
                pending.push(signature);
                0
            }
            Term::Integer(value) => value.bit_len().div_ceil(32),
            Term::Float(value) => value
                .digits()
                .len()
                .saturating_add(value.exponent().bit_len().div_ceil(32)),
            Term::BitVec(value) => value.words().len(),
            Term::String(value) | Term::Symbol(value) => value.len(),
            _ => 0,
        };
        *fuel = fuel
            .checked_sub(cost)
            .ok_or("associated copy budget exhausted")?;
    }
    Ok(())
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use nir::program::{AssociatedContract, Trait, TraitRequirement};
    use nir::{AssociatedTypeOrigin, DeclarationId, Sort};

    #[test]
    fn given_views_preserve_valid_projections_without_weakening_strict_normalization() {
        let id = |local| DeclarationId { package: 1, local };
        let subject = Term::Rigid {
            declaration: id(3),
            sort: Sort::Type,
        };
        let projection = Term::AssociatedType {
            origin: Box::new(AssociatedTypeOrigin {
                interface: id(1),
                member: id(2),
            }),
            arguments: vec![],
            subject: Box::new(subject.clone()),
        };
        let mut traits = vec![Trait {
            comptime_only: false,
            parameter_types: Vec::new(),
            declaration: id(1),
            self_parameter: id(1),
            name: "Source".into(),
            type_parameters: vec![],
            parents: vec![],
            methods: vec![],
            requirements: vec![],
            capabilities: vec![],
            associated: vec![AssociatedContract {
                phase: Default::default(),
                declaration: id(2),
                name: "Element".into(),
                ty: Term::Primitive(Primitive::Type),
                default: None,
            }],
        }];
        let assumptions = vec![TraitRequirement {
            subject,
            interface: id(1),
            interface_arguments: vec![],
        }];
        let index = crate::implementation::Index::default();
        let context = crate::ptc::Context::default();
        let world = World {
            index: &index,
            implementations: &[],
            traits: &traits,
            assumptions: &assumptions,
            context: &context,
        };
        let repeated = Term::Tuple(vec![projection.clone(), projection.clone()]);
        let view = world
            .associated_type_view(&repeated, Budget::default())
            .unwrap();
        assert_eq!(view.value, repeated);
        assert!(view.steps.is_empty());
        assert!(
            world
                .normalize_associated_types(&projection, Budget::default())
                .is_err()
        );
        assert!(
            world
                .associated_type_view(
                    &projection,
                    Budget {
                        steps: 0,
                        ..Budget::default()
                    }
                )
                .is_err()
        );
        let mut invalid = projection.clone();
        let Term::AssociatedType { origin, .. } = &mut invalid else {
            unreachable!()
        };
        origin.member = id(99);
        assert!(
            world
                .associated_type_view(&invalid, Budget::default())
                .unwrap_err()
                .contains("member does not belong")
        );
        let without_givens = World {
            assumptions: &[],
            ..world
        };
        assert!(
            without_givens
                .associated_type_view(&projection, Budget::default())
                .is_err()
        );
        let mut scoped_projection = projection.clone();
        if let Term::AssociatedType { subject, .. } = &mut scoped_projection {
            *subject = Box::new(Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::Type,
            });
        }
        let callable = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(scoped_projection.clone()),
        };
        let view = without_givens
            .associated_type_view(&callable, Budget::default())
            .unwrap();
        assert_eq!(view.value, callable);
        assert!(view.steps.is_empty());
        let mut view_cache = std::collections::HashMap::new();
        let (cached, _) = without_givens
            .associated_type_view_cached(&callable, Budget::default(), &mut view_cache)
            .unwrap();
        assert_eq!(cached.value, callable);
        assert!(
            view_cache.is_empty(),
            "caller-bound projections must not enter a context-free cache"
        );
        let mut outer_projection = scoped_projection.clone();
        if let Term::AssociatedType { subject, .. } = &mut outer_projection {
            *subject = Box::new(Term::Bound {
                depth: 1,
                index: 0,
                sort: nir::Sort::Type,
            });
        }
        let nested = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(outer_projection),
            }),
        };
        let (nested_view, _) = without_givens
            .associated_type_view_cached(&nested, Budget::default(), &mut view_cache)
            .unwrap();
        assert_eq!(nested_view.value, nested);
        assert!(nested_view.steps.is_empty());
        assert!(
            view_cache.is_empty(),
            "outer-frame projections must not enter a context-free cache"
        );
        assert!(
            without_givens
                .normalize_associated_types(&nested, Budget::default())
                .is_err()
        );
        assert!(
            without_givens
                .normalize_associated_types(&callable, Budget::default())
                .is_err()
        );
        assert!(
            without_givens
                .associated_type_view(&scoped_projection, Budget::default())
                .is_err()
        );
        let mut bad_member = callable.clone();
        if let Term::Function { result, .. } = &mut bad_member {
            if let Term::AssociatedType { origin, .. } = result.as_mut() {
                origin.member = id(99);
            }
        }
        assert!(
            without_givens
                .associated_type_view(&bad_member, Budget::default())
                .unwrap_err()
                .contains("member does not belong")
        );
        assert!(
            without_givens
                .associated_type_view_cached(&bad_member, Budget::default(), &mut view_cache)
                .is_err()
        );
        assert!(view_cache.is_empty());
        traits[0].associated[0].ty = Term::Primitive(Primitive::Bool);
        let world = World {
            index: &index,
            implementations: &[],
            traits: &traits,
            assumptions: &assumptions,
            context: &context,
        };
        assert!(
            world
                .associated_type_view(&projection, Budget::default())
                .unwrap_err()
                .contains("not Type-valued")
        );
        assert!(
            world
                .associated_type_view(&callable, Budget::default())
                .unwrap_err()
                .contains("not Type-valued")
        );
    }
}

#[cfg(test)]
mod shared_cache_tests {
    use super::*;
    use nir::program::{AssociatedContract, Expr, Implementation, SourceRange, Trait};
    use nir::{AssociatedTypeOrigin, DeclarationId};

    #[test]
    fn completed_projection_cache_reuses_nested_results_and_rolls_back_failed_queries() {
        let id = |local| DeclarationId { package: 1, local };
        let result = Term::Primitive(Primitive::Char);
        let projection = Term::AssociatedType {
            origin: Box::new(AssociatedTypeOrigin {
                interface: id(1),
                member: id(2),
            }),
            arguments: vec![],
            subject: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let traits = vec![Trait {
            comptime_only: false,
            parameter_types: Vec::new(),
            declaration: id(1),
            self_parameter: id(1),
            name: "Source".into(),
            type_parameters: vec![],
            parents: vec![],
            methods: vec![],
            requirements: vec![],
            capabilities: vec![],
            associated: vec![AssociatedContract {
                phase: Default::default(),
                declaration: id(2),
                name: "Element".into(),
                ty: Term::Primitive(Primitive::Type),
                default: None,
            }],
        }];
        let implementations = vec![Implementation {
            phases: Default::default(),
            associated_initializers: vec![],
            parameter_types: vec![],
            declaration: id(3),
            subject: Term::Primitive(Primitive::Bool),
            interface: Some(id(1)),
            interface_arguments: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            requirements: vec![],
            methods: vec![],
            associated: vec![(
                id(2),
                Expr {
                    ty: Term::Primitive(Primitive::Type),
                    source: SourceRange { start: 0, end: 0 },
                    kind: ExprKind::TypeValue(result.clone()),
                },
            )],
        }];
        let mut index = crate::implementation::Index::default();
        index
            .insert(
                crate::implementation::ImplementationHead {
                    parameter_types: vec![],
                    declaration: id(3),
                    subject: Term::Primitive(Primitive::Bool),
                    interface: Some(id(1)),
                    interface_arguments: vec![],
                    type_parameters: vec![],
                    requirements: vec![],
                },
                Budget::default(),
            )
            .unwrap();
        let context = crate::ptc::Context::default();
        let world = World {
            index: &index,
            implementations: &implementations,
            traits: &traits,
            assumptions: &[],
            context: &context,
        };
        let mut invalid = projection.clone();
        let Term::AssociatedType { origin, .. } = &mut invalid else {
            unreachable!()
        };
        origin.member = id(99);
        let mut cache = std::collections::HashMap::new();
        assert!(
            world
                .normalize_associated_types_cached(
                    &Term::Tuple(vec![projection.clone(), invalid]),
                    Budget::default(),
                    &mut cache
                )
                .is_err()
        );
        assert!(
            cache.is_empty(),
            "failed outer query must not publish its successful prefix"
        );
        let (first, _) = world
            .normalize_associated_types_cached(
                &Term::Tuple(vec![projection.clone()]),
                Budget::default(),
                &mut cache,
            )
            .unwrap();
        assert_eq!(first.steps.len(), 1);
        assert_eq!(cache.get(&projection), Some(&result));
        let (second, _) = world
            .normalize_associated_types_cached(
                &Term::Pointer(Box::new(projection.clone())),
                Budget {
                    steps: 100,
                    ..Budget::default()
                },
                &mut cache,
            )
            .unwrap();
        assert!(second.steps.is_empty());
        assert_eq!(second.value, Term::Pointer(Box::new(result.clone())));
        let (optional, _) = world
            .normalize_associated_types_cached(
                &Term::Optional(Box::new(projection.clone())),
                Budget::default(),
                &mut cache,
            )
            .unwrap();
        assert!(optional.steps.is_empty());
        assert_eq!(optional.value, Term::Optional(Box::new(result)));
        let before = cache.clone();
        assert!(
            world
                .normalize_associated_types_cached(
                    &projection,
                    Budget {
                        steps: 0,
                        ..Budget::default()
                    },
                    &mut cache
                )
                .is_err()
        );
        assert_eq!(cache, before);
    }
}

/// An admitted associated value and the exact selection evidence supporting it.
#[derive(Clone, Debug)]
pub struct AssociatedValueResolution {
    /// True only for the explicit staging query; ordinary value queries reject it.
    pub requires_invocation: bool,
    pub value: nir::program::Expr,
    pub evidence: TypeEvidence,
    pub qualification: Qualification,
}
impl fmt::Display for AssociatedValueResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-value-resolution (requires-invocation {}) (value {}) (implementation {}) (arguments",
            self.requires_invocation, self.value, self.evidence.implementation
        )?;
        for argument in &self.evidence.type_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") {})", self.qualification)
    }
}
impl World<'_> {
    pub fn resolve_associated_value(
        &self,
        interface: &InterfaceInstance,
        member_id: nir::DeclarationId,
        subject: &Term,
        budget: Budget,
    ) -> Result<AssociatedValueResolution, String> {
        self.resolve_associated_value_budgeted(interface, member_id, subject, budget)
            .map(|(value, _)| value)
    }
    pub(crate) fn resolve_associated_value_budgeted(
        &self,
        interface: &InterfaceInstance,
        member_id: nir::DeclarationId,
        subject: &Term,
        budget: Budget,
    ) -> Result<(AssociatedValueResolution, usize), String> {
        let (value, remaining) =
            self.resolve_associated_value_plan_budgeted(interface, member_id, subject, budget)?;
        if value.requires_invocation {
            return Err("associated initializer requires comptime invocation".into());
        }
        Ok((value, remaining))
    }
    pub fn resolve_associated_value_plan_budgeted(
        &self,
        interface: &InterfaceInstance,
        member_id: nir::DeclarationId,
        subject: &Term,
        budget: Budget,
    ) -> Result<(AssociatedValueResolution, usize), String> {
        let mut fuel = budget.steps;
        charge_projection_copy(subject, &mut fuel)?;
        subject.check_scope(&[]).map_err(|e| e.to_string())?;
        for argument in &interface.arguments {
            charge_projection_copy(argument, &mut fuel)?;
            argument.check_scope(&[]).map_err(|e| e.to_string())?;
        }
        let contract = self
            .traits
            .iter()
            .find(|contract| contract.declaration == interface.declaration)
            .ok_or("associated value query has no originating interface")?;
        if interface.arguments.len() != contract.type_parameters.len() {
            return Err("associated value interface argument count mismatch".into());
        }
        let member = contract
            .associated
            .iter()
            .find(|member| member.declaration == member_id)
            .ok_or("associated value member does not belong to its interface")?;
        let parameters = contract.binders_with_self(&mut fuel)?;
        let inputs: Vec<_> = std::iter::once(subject.clone())
            .chain(interface.arguments.iter().cloned())
            .collect();
        charge_projection_substitution(&member.ty, &parameters, &inputs, &mut fuel)?;
        let substitution = contract
            .input_substitution(&inputs, &[], &mut fuel)
            .map_err(|e| e.to_string())?;
        let member_type = substitution
            .instantiate_bounded(&member.ty, &mut fuel)
            .map_err(|e| e.to_string())?;
        // Reserve a disjoint allowance for each proof/search rather than
        // resetting the full caller budget at every formation obligation.
        let queries = contract
            .requirements
            .len()
            .saturating_add(contract.capabilities.len().saturating_mul(2))
            .saturating_add(1);
        let query_steps = fuel / queries.saturating_mul(2);
        if query_steps == 0 {
            return Err("associated value query budget exhausted".into());
        }
        fuel -= query_steps * queries;
        let query_budget = Budget {
            steps: query_steps,
            ..budget
        };
        // Interface-parameter constraints are formation obligations, not
        // assumptions supplied by selecting an implementation.
        for requirement in &contract.requirements {
            let mut pending = vec![(requirement, 0usize)];
            while let Some((requirement, depth)) = pending.pop() {
                if depth >= budget.max_depth.min(64) {
                    return Err("associated value requirement depth exhausted".into());
                }
                fuel = fuel
                    .checked_sub(1)
                    .ok_or("associated value query budget exhausted")?;
                match requirement {
                    nir::program::TypePredicate::Trait(requirement) => {
                        charge_projection_substitution(
                            &requirement.subject,
                            &parameters,
                            &inputs,
                            &mut fuel,
                        )?;
                        for argument in &requirement.interface_arguments {
                            charge_projection_substitution(
                                argument,
                                &parameters,
                                &inputs,
                                &mut fuel,
                            )?;
                        }
                    }
                    nir::program::TypePredicate::Equal(a, b) => {
                        charge_projection_substitution(a, &parameters, &inputs, &mut fuel)?;
                        charge_projection_substitution(b, &parameters, &inputs, &mut fuel)?;
                    }
                    nir::program::TypePredicate::And(items)
                    | nir::program::TypePredicate::Or(items) => {
                        if items.len() > fuel {
                            return Err("associated value query budget exhausted".into());
                        }
                        pending.extend(items.iter().map(|item| (item, depth + 1)));
                    }
                    nir::program::TypePredicate::Not(inner) => pending.push((inner, depth + 1)),
                    nir::program::TypePredicate::Bool(_) => (),
                }
            }
            let requirement = substitution
                .instantiate_predicate_bounded(requirement, &mut fuel)
                .map_err(|e| e.to_string())?;
            if !matches!(
                self.context
                    .prove(Formula::from_predicate(&requirement), query_budget),
                Outcome::Entailed(_)
            ) {
                return Err("associated value interface requirement is not established".into());
            }
        }
        for capability in &contract.capabilities {
            charge_projection_substitution(&capability.subject, &parameters, &inputs, &mut fuel)?;
            for argument in &capability.interface_arguments {
                charge_projection_substitution(argument, &parameters, &inputs, &mut fuel)?;
            }
            let capability = substitution
                .instantiate_requirement_bounded(capability, &mut fuel)
                .map_err(|e| e.to_string())?;
            if self.given(&capability, query_budget).is_some() {
                continue;
            }
            let resolved = self.resolve_projected_instance(
                &capability.interface_instance(),
                &capability.subject,
                query_budget,
            )?;
            if !matches!(resolved.types.selection, Selection::Selected(_)) {
                return Err("associated value interface capability is not established".into());
            }
        }
        let mut selection_budget = query_budget;
        let resolution =
            self.resolve_projected_instance_metered(interface, subject, &mut selection_budget)?;
        // This allowance was reserved above. Return only the solver's unused
        // work, including prerequisite normalization, to the enclosing query.
        fuel += selection_budget.steps;
        let Selection::Selected(selected) = resolution.types.selection else {
            return Err(format!(
                "associated value implementation selection is unresolved: {}",
                resolution.types
            ));
        };
        let Some(TypeAssessment::Applicable(evidence)) =
            resolution.types.assessments.get(&selected)
        else {
            return Err("associated value selection lacks type evidence".into());
        };
        let Some(qualification @ Qualification::Proved(_)) =
            resolution.qualifications.get(&selected)
        else {
            return Err("associated value selection lacks capability evidence".into());
        };
        let implementation = self
            .implementations
            .iter()
            .find(|i| i.declaration == selected)
            .ok_or("associated value selection has no implementation contract")?;
        let mut values = implementation
            .associated
            .iter()
            .map(|(id, value)| (*id, value, false))
            .chain(
                implementation
                    .associated_initializers
                    .iter()
                    .map(|(id, value)| (*id, value.expression(), true)),
            )
            .filter(|(id, _, _)| *id == member_id);
        let (_, value, requires_invocation) = values
            .next()
            .ok_or("selected implementation has no associated value")?;
        if values.next().is_some() {
            return Err("selected implementation has duplicate associated values".into());
        }
        if !requires_invocation {
            charge_completed_value(value, &self.index.nominal_layouts, self.context, &mut fuel)?;
        }
        let binders = implementation
            .parameter_binders()
            .map_err(|e| e.to_string())?;
        nir::copy_budget::charge_substitution(value, &binders, &evidence.type_arguments, &mut fuel)
            .map_err(|_| "associated value substitution budget exhausted")?;
        let substitution = nir::DeclarationSubstitution::bounded(
            &implementation.type_parameters,
            &implementation.parameter_types,
            &evidence.type_arguments,
            &[],
            &mut fuel,
        )
        .map_err(|e| e.to_string())?;
        let mut value = value.clone();
        value
            .rewrite_types(&mut |ty| substitution.instantiate_bounded(ty, &mut fuel))
            .map_err(|e| e.to_string())?;
        // Compare completed type views without unfolding opaque normal-form heads.
        let mut cache = std::collections::HashMap::new();
        let (expected, remaining) = self.normalize_associated_types_cached(
            &member_type,
            Budget {
                steps: fuel,
                ..budget
            },
            &mut cache,
        )?;
        fuel = remaining;
        let (actual, remaining) = self.normalize_associated_types_cached(
            &value.ty,
            Budget {
                steps: fuel,
                ..budget
            },
            &mut cache,
        )?;
        fuel = remaining;
        if actual.value != expected.value {
            return Err("selected associated value disagrees with its member contract".into());
        }
        value.ty = actual.value;
        // Only completed constant representations can be published as values.
        // Function identities remain values; their bodies are not invoked here.
        if requires_invocation {
            value = nir::associated_initializers::AssociatedInitializer::new(value, &mut fuel)?
                .into_expression();
        } else {
            charge_completed_value(&value, &self.index.nominal_layouts, self.context, &mut fuel)?;
        }
        Ok((
            AssociatedValueResolution {
                requires_invocation,
                value,
                evidence: evidence.clone(),
                qualification: qualification.clone(),
            },
            fuel,
        ))
    }
}

/// Bound both payload copying and structural depth before recursive substitution.
fn charge_completed_value(
    value: &nir::program::Expr,
    layouts: &nir::program::Package,
    context: &crate::ptc::Context,
    fuel: &mut usize,
) -> Result<(), String> {
    nir::completed_values::validate_with_layout_evidence_in(
        value,
        layouts.newtypes.iter(),
        &|ty| {
            let nir::Term::NormalForm { head, .. } = ty else {
                return None;
            };
            if !context.is_tuple_head(*head) {
                return None;
            }
            let heads = &layouts.normal_forms;
            let slot = heads
                .binary_search_by_key(head, |metadata| metadata.declaration)
                .ok()?;
            // Duplicate declarations cannot provide unique layout authority.
            if slot
                .checked_sub(1)
                .is_some_and(|prior| heads[prior].declaration == *head)
                || heads
                    .get(slot + 1)
                    .is_some_and(|next| next.declaration == *head)
            {
                return None;
            }
            heads[slot].tuple_elements(ty)
        },
        &|id, fuel| nir::completed_values::lookup_nominal(layouts, id, fuel),
        layouts.target,
        fuel,
    )
    .map_err(str::to_owned)
}
