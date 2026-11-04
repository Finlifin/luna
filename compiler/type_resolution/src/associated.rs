//! Evidence-driven associated Type queries in an immutable, admitted world.
//! One-step queries retain residuals; recursive normalization closes projections
//! transactionally with cycle detection and a query-local completed cache.
//! Substitution never invokes normal-form heads or substitutes trait defaults.
use crate::capabilities::{Qualification, World};
use crate::implementation::{Selection, TypeAssessment, TypeEvidence};
use crate::ptc::{Budget, Formula, Outcome};
use nir::program::{ExprKind, InterfaceInstance};
use nir::{NominalAdmissionError as QueryError, Primitive, Term};
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
    fn check_associated_formation(
        &self,
        contract: &nir::program::Trait,
        substitution: &nir::DeclarationSubstitution<'_>,
        budget: &mut Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
    ) -> Result<(), QueryError> {
        for requirement in &contract.requirements {
            let mut pending = vec![(requirement, 0usize)];
            while let Some((predicate, depth)) = pending.pop() {
                if depth >= budget.max_depth.min(64) {
                    return Err(QueryError::Budget);
                }
                budget.steps = budget.steps.checked_sub(1).ok_or(QueryError::Budget)?;
                match predicate {
                    nir::program::TypePredicate::And(items)
                    | nir::program::TypePredicate::Or(items) => {
                        if items.len() > budget.steps {
                            return Err(QueryError::Budget);
                        }
                        pending.extend(items.iter().map(|item| (item, depth + 1)));
                    }
                    nir::program::TypePredicate::Not(inner) => pending.push((inner, depth + 1)),
                    _ => {}
                }
            }
            let requirement =
                substitution.instantiate_predicate_bounded(requirement, &mut budget.steps)?;
            match self
                .context
                .prove_metered(Formula::from_predicate(&requirement), budget)
            {
                Outcome::Entailed(_) => {}
                Outcome::Unknown {
                    reason: crate::ptc::UnknownReason::BudgetExhausted,
                    ..
                } => return Err(QueryError::Budget),
                Outcome::Unknown { reason, .. } => {
                    return Err(QueryError::Unresolved(format!(
                        "associated interface requirement: {reason:?}"
                    )));
                }
                Outcome::NotEntailed(_) => {
                    return Err(QueryError::Unresolved(
                        "associated interface requirement is not established".into(),
                    ));
                }
                Outcome::IllFormed(reason) => return Err(QueryError::Invalid(reason)),
            }
        }
        for capability in &contract.capabilities {
            let capability =
                substitution.instantiate_requirement_bounded(capability, &mut budget.steps)?;
            if self.given_metered(&capability, budget)?.is_some() {
                continue;
            }
            let resolved = resolve(
                &capability.interface_instance(),
                &capability.subject,
                budget,
            )?;
            if !matches!(resolved.types.selection, Selection::Selected(_)) {
                return Err(QueryError::Unresolved(
                    "associated interface capability is not established".into(),
                ));
            }
        }
        Ok(())
    }

    /// Resolve a well-scoped Type projection after its input substitutions.
    /// Member lookup has already chosen an origin (including inherited origins).
    /// The caller must establish any requested child view separately.
    pub fn resolve_associated_type(
        &self,
        projection: &Term,
        budget: Budget,
    ) -> Result<AssociatedTypeResolution, QueryError> {
        self.resolve_associated_type_budgeted(projection, &mut { budget })
            .map(|(result, _)| result)
    }

    fn resolve_associated_type_budgeted(
        &self,
        projection: &Term,
        budget: &mut Budget,
    ) -> Result<(AssociatedTypeResolution, usize), QueryError> {
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
        budget: &mut Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
    ) -> Result<(AssociatedTypeResolution, usize), QueryError> {
        let mut fuel = budget.steps;
        let result = (|| {
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
            let substitution = contract.input_substitution(&inputs, &[], &mut fuel)?;
            let member_type = substitution.instantiate_bounded(&member.ty, &mut fuel)?;
            if member_type != Term::Primitive(Primitive::Type) {
                return Err("associated projection member is not Type-valued".into());
            }
            let mut formation_budget = Budget {
                steps: fuel,
                ..*budget
            };
            let formed = self.check_associated_formation(
                contract,
                &substitution,
                &mut formation_budget,
                resolve,
            );
            fuel = formation_budget.steps;
            formed?;
            let interface = InterfaceInstance {
                declaration: origin.interface,
                arguments: arguments.clone(),
            };
            // Formation is complete. The final selection can use the remaining
            // work, then return its unused allowance for evidence validation and
            // later projections. Repeatedly halving here starves nested adapters
            // even when the preceding formation checks are inexpensive.
            let mut allowance = Budget {
                steps: fuel,
                ..*budget
            };
            let resolution = resolve(&interface, subject, &mut allowance);
            fuel = allowance.steps;
            let resolution = resolution?;
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
                return Err(QueryError::Unresolved(format!(
                    "associated Type implementation selection is unresolved: {} (remaining-steps {fuel}) (pending {pending})",
                    resolution.types.selection
                )));
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
            nir::copy_budget::charge_substitution(
                value,
                &binders,
                &evidence.type_arguments,
                &mut fuel,
            )
            .map_err(|_| QueryError::Budget)?;
            let substitution = nir::DeclarationSubstitution::bounded(
                &implementation.type_parameters,
                &implementation.parameter_types,
                &evidence.type_arguments,
                &[],
                &mut fuel,
            )?;
            let mut completed = value.clone();
            completed.rewrite_types(&mut |ty| substitution.instantiate_bounded(ty, &mut fuel))?;
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
        })();
        budget.steps = fuel;
        result
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
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        self.normalize_associated_types_budgeted(ty, budget)
            .map(|(result, _)| result)
    }

    pub(crate) fn normalize_associated_types_budgeted(
        &self,
        ty: &Term,
        budget: Budget,
    ) -> Result<(AssociatedTypeNormalization, usize), QueryError> {
        self.normalize_associated_types_mode(ty, &mut { budget }, false, None, None)
            .map(|(result, remaining, _)| (result, remaining))
    }

    /// The caller owns this cache for one immutable implementation world.
    /// Publish newly completed projections only after the whole query succeeds.
    pub fn normalize_associated_types_cached(
        &self,
        ty: &Term,
        budget: Budget,
        cache: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<(AssociatedTypeNormalization, usize), QueryError> {
        let (result, remaining, completed) =
            self.normalize_associated_types_mode(ty, &mut { budget }, false, Some(cache), None)?;
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
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        self.associated_type_view_metered(ty, &mut { budget })
    }

    pub fn associated_type_view_metered(
        &self,
        ty: &Term,
        budget: &mut Budget,
    ) -> Result<AssociatedTypeNormalization, QueryError> {
        self.normalize_associated_types_mode(ty, budget, true, None, None)
            .map(|(value, _, _)| value)
    }

    pub fn associated_type_view_cached(
        &self,
        ty: &Term,
        budget: Budget,
        cache: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<(AssociatedTypeNormalization, usize), QueryError> {
        let (value, remaining, completed) =
            self.normalize_associated_types_mode(ty, &mut { budget }, true, Some(cache), None)?;
        cache.extend(completed);
        Ok((value, remaining))
    }

    pub(crate) fn associated_type_view_for_selection(
        &self,
        ty: &Term,
        budget: &mut Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
    ) -> Result<(Term, usize), QueryError> {
        self.normalize_associated_types_mode(ty, budget, true, None, Some(resolve))
            .map(|(result, remaining, _)| (result.value, remaining))
    }

    fn normalize_associated_types_mode(
        &self,
        ty: &Term,
        budget: &mut Budget,
        preserve_given: bool,
        shared: Option<&std::collections::HashMap<Term, Term>>,
        resolve: Option<
            &mut dyn FnMut(
                &InterfaceInstance,
                &Term,
                &mut Budget,
            ) -> Result<crate::capabilities::Resolution, QueryError>,
        >,
    ) -> Result<
        (
            AssociatedTypeNormalization,
            usize,
            std::collections::HashMap<Term, Term>,
        ),
        QueryError,
    > {
        let mut normalizer = Normalizer {
            world: self,
            resolve,
            preserve_given,
            budget: *budget,
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
        let result = (|| {
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
                    steps: std::mem::take(&mut normalizer.steps),
                },
                normalizer.remaining,
                std::mem::take(&mut normalizer.completed),
            ))
        })();
        budget.steps = normalizer.remaining;
        result
    }
}

struct Normalizer<'a, 'w, 'r> {
    resolve: Option<
        &'r mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
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
    fn spend(&mut self, steps: usize) -> Result<(), QueryError> {
        self.remaining = self
            .remaining
            .checked_sub(steps)
            .ok_or(QueryError::Budget)?;
        Ok(())
    }
    fn charge_copy(&mut self, term: &Term) -> Result<(), QueryError> {
        charge_projection_copy(term, &mut self.remaining)
    }
    fn many(&mut self, terms: &[Term], depth: usize) -> Result<Vec<Term>, QueryError> {
        if terms.len() > self.remaining {
            return Err(QueryError::Budget);
        }
        terms
            .iter()
            .map(|term| self.normalize(term, depth))
            .collect()
    }
    fn normalize(&mut self, term: &Term, depth: usize) -> Result<Term, QueryError> {
        if depth >= self.budget.max_depth.min(64) {
            return Err(QueryError::Budget);
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
                    .collect::<Result<Vec<_>, QueryError>>()?;
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
                        })?;
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
                        return Err(QueryError::Budget);
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
                    if found?.is_some() {
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
                            })?;
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
                    return Err(QueryError::Unresolved(
                        "cyclic associated Type projection".into(),
                    ));
                }
                let result = (|| {
                    let mut allowance = Budget {
                        steps: self.remaining,
                        ..self.budget
                    };
                    let resolved = if let Some(resolve) = self.resolve.as_deref_mut() {
                        self.world
                            .resolve_associated_type_using(&key, &mut allowance, resolve)
                    } else {
                        self.world
                            .resolve_associated_type_budgeted(&key, &mut allowance)
                    };
                    self.remaining = allowance.steps;
                    let (resolution, _) = resolved.map_err(|error| match error {
                        QueryError::Budget => QueryError::Budget,
                        QueryError::Invalid(message) => QueryError::Invalid(format!(
                            "associated Type normalization of {key}: {message}"
                        )),
                        QueryError::Unresolved(message) => QueryError::Unresolved(format!(
                            "associated Type normalization of {key}: {message}"
                        )),
                    })?;
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
                    return Err(QueryError::Budget);
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
                    return Err(QueryError::Budget);
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
                        .collect::<Result<_, QueryError>>()?;
                    Ok::<_, QueryError>(Term::Function {
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

// Cache hits must pay for payload copying too, not only the node count.
// Shared preflight bounds recursive depth and the complete substituted payload.
fn charge_projection_substitution(
    term: &Term,
    parameters: &[nir::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), QueryError> {
    nir::type_computations::charge_parameter_substitution(term, parameters, arguments, fuel)
        .map_err(QueryError::from)
}

pub fn charge_projection_copy(term: &Term, fuel: &mut usize) -> Result<(), QueryError> {
    nir::type_budget::charge_structure(term, fuel).map_err(|_| QueryError::Budget)
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
                .to_string()
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
                .to_string()
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
                .to_string()
                .contains("not Type-valued")
        );
        assert!(
            world
                .associated_type_view(&callable, Budget::default())
                .unwrap_err()
                .to_string()
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
        // Both normalization entry paths must debit recursive selection once.
        // The standalone view receives the same half-allowance as prerequisite_type.
        let mut view_budget = Budget {
            steps: 50_000,
            ..Budget::default()
        };
        let view = world
            .associated_type_view_metered(&projection, &mut view_budget)
            .unwrap();
        let mut sequential = Budget {
            steps: 100_000 - (50_000 - view_budget.steps),
            ..Budget::default()
        };
        let target: InterfaceInstance = id(90).into();
        let direct = world
            .resolve_instance_metered(&target, &view.value, &mut sequential)
            .unwrap();
        let mut nested = Budget::default();
        let projected = world
            .resolve_projected_instance_metered(&target, &projection, &mut nested)
            .unwrap();
        assert_eq!(projected.types.selection, direct.types.selection);
        assert_eq!(
            nested.steps, sequential.steps,
            "nested query work must be debited exactly once"
        );

        // Host query failures retain their category and actual consumption;
        // neither an error nor an unspent reservation can zero the caller.
        for failure in [
            QueryError::Budget,
            QueryError::Invalid("bad origin".into()),
            QueryError::Unresolved("pending origin".into()),
        ] {
            let mut consumption = Vec::new();
            for cost in [0, 17] {
                let mut budget = Budget::default();
                let mut invocations = 0;
                let result = world.associated_type_view_for_selection(
                    &projection,
                    &mut budget,
                    &mut |_, _, allowance| {
                        invocations += 1;
                        allowance.steps -= cost;
                        Err(failure.clone())
                    },
                );
                let error = result.unwrap_err();
                assert_eq!(
                    std::mem::discriminant(&error),
                    std::mem::discriminant(&failure)
                );
                assert_eq!(invocations, 1);
                assert!(budget.steps > 0 && budget.steps < 100_000);
                consumption.push(100_000 - budget.steps);
            }
            assert_eq!(consumption[1] - consumption[0], 17);
        }
        let mut invalid = projection.clone();
        let Term::AssociatedType { origin, .. } = &mut invalid else {
            unreachable!()
        };
        origin.member = id(99);
        let mut invalid_budget = Budget::default();
        assert!(matches!(
            world.associated_type_view_metered(&invalid, &mut invalid_budget),
            Err(QueryError::Invalid(_))
        ));
        assert!(invalid_budget.steps > 0 && invalid_budget.steps < 100_000);
        assert_eq!(
            world
                .associated_type_view_metered(&projection, &mut invalid_budget)
                .unwrap()
                .value,
            result
        );
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
        let mut selection_budget = Budget {
            steps: fuel,
            ..budget
        };
        self.check_associated_formation(
            contract,
            &substitution,
            &mut selection_budget,
            &mut |interface, subject, allowance| {
                self.resolve_projected_instance_metered(interface, subject, allowance)
            },
        )?;
        let resolution =
            self.resolve_projected_instance_metered(interface, subject, &mut selection_budget)?;
        fuel = selection_budget.steps;
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
    nir::completed_values::validate_with_collection_evidence_in(
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
        &|ty, fuel| {
            nir::normal_forms::list_element_budgeted(layouts, ty, fuel)
                .map_err(|_| "invalid completed collection layout evidence")
        },
        &|id, fuel| nir::completed_values::lookup_nominal(layouts, id, fuel),
        layouts.target,
        fuel,
    )
    .map_err(str::to_owned)
}
