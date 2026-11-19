//! Associated member selection and admission of its instantiated supply.
use super::*;
use crate::implementation::TypeAssessment;
use nir::associated_initializers::{AssociatedInitializer, AssociatedSelection};
use nir::completed_values::CompletedValue;

/// An admitted associated value and the exact selection evidence supporting it.
#[derive(Clone, Debug)]
pub struct AssociatedValueResolution {
    pub supply: AssociatedSelection,
    pub evidence: TypeEvidence,
    pub qualification: Qualification,
}
impl fmt::Display for AssociatedValueResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-value-resolution {} (implementation {}) (arguments",
            self.supply, self.evidence.implementation
        )?;
        for argument in &self.evidence.type_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") {})", self.qualification)
    }
}
/// A member selected in one declaration world, before reading its implementation
/// payload. The same world owns the assumptions, substitution and layout facts.
pub struct SelectedAssociatedMember<'a, 'world> {
    world: &'a World<'world>,
    row: &'world nir::program::Implementation,
    member: nir::DeclarationId,
    member_type: Term,
    evidence: TypeEvidence,
    qualification: Qualification,
}

#[derive(Clone, Copy)]
pub(super) enum Access {
    TypeProjection,
    Value,
}

impl<'world> World<'world> {
    pub fn resolve_associated_value(
        &self,
        interface: &InterfaceInstance,
        member_id: nir::DeclarationId,
        subject: &Term,
        budget: Budget,
    ) -> Result<AssociatedValueResolution, QueryError> {
        let value =
            self.resolve_associated_value_plan_metered(interface, member_id, subject, &mut {
                budget
            })?;
        if matches!(value.supply, AssociatedSelection::Initializer(_)) {
            return Err(QueryError::Unresolved(
                "associated initializer requires comptime invocation".into(),
            ));
        }
        Ok(value)
    }

    /// Select a completed value or initialization plan without executing it.
    /// All outcomes return the actual remaining allowance to the caller.
    pub fn resolve_associated_value_plan_metered(
        &self,
        interface: &InterfaceInstance,
        member_id: nir::DeclarationId,
        subject: &Term,
        budget: &mut Budget,
    ) -> Result<AssociatedValueResolution, QueryError> {
        self.select_associated_member(interface, member_id, subject, budget)?
            .supply(budget)
    }

    /// Read already completed data for a closed projection. A proof consumer
    /// cannot run an initializer or use a default as evidence about an open
    /// receiver. An absent supply leaves the observation symbolic.
    pub fn known_associated_value(
        &self,
        interface: &InterfaceInstance,
        member: nir::DeclarationId,
        subject: &Term,
        budget: &mut Budget,
    ) -> Result<Option<CompletedValue>, QueryError> {
        if !self
            .closed_inputs_metered(std::iter::once(subject).chain(&interface.arguments), budget)?
        {
            return Ok(None);
        }
        let selected = match self.select_associated_member(interface, member, subject, budget) {
            Ok(selected) => selected,
            Err(QueryError::Unresolved(_)) => return Ok(None),
            Err(failure) => return Err(failure),
        };
        let Some(MemberSource::Value(value)) = selected.source(&mut budget.steps)? else {
            return Ok(None);
        };
        selected.complete_value(value, budget).map(Some)
    }

    pub fn select_associated_member<'a>(
        &'a self,
        interface: &InterfaceInstance,
        member_id: nir::DeclarationId,
        subject: &Term,
        budget: &mut Budget,
    ) -> Result<SelectedAssociatedMember<'a, 'world>, QueryError> {
        charge_projection_copy(subject, &mut budget.steps)?;
        subject.check_scope(&[]).map_err(|e| e.to_string())?;
        for argument in &interface.arguments {
            charge_projection_copy(argument, &mut budget.steps)?;
            argument.check_scope(&[]).map_err(|e| e.to_string())?;
        }
        self.select_scoped_member(
            interface,
            member_id,
            subject,
            Access::Value,
            budget,
            &mut |interface, subject, budget| {
                self.resolve_projected_instance_metered(interface, subject, budget)
            },
        )
    }

    pub(super) fn select_scoped_member<'a>(
        &'a self,
        interface: &InterfaceInstance,
        member_id: nir::DeclarationId,
        subject: &Term,
        access: Access,
        budget: &mut Budget,
        resolve: &mut dyn FnMut(
            &InterfaceInstance,
            &Term,
            &mut Budget,
        ) -> Result<crate::capabilities::Resolution, QueryError>,
    ) -> Result<SelectedAssociatedMember<'a, 'world>, QueryError> {
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
        let parameters = contract.binders_with_self_checked(&mut budget.steps)?;
        let inputs: Vec<_> = std::iter::once(subject.clone())
            .chain(interface.arguments.iter().cloned())
            .collect();
        charge_projection_substitution(&member.ty, &parameters, &inputs, &mut budget.steps)?;
        let substitution = contract.input_substitution(&inputs, &[], &mut budget.steps)?;
        let member_type = substitution.instantiate_bounded(&member.ty, &mut budget.steps)?;
        if matches!(access, Access::TypeProjection)
            && member_type != Term::Primitive(Primitive::Type)
        {
            return Err("associated projection member is not Type-valued".into());
        }
        self.check_associated_formation(contract, &substitution, budget, resolve)?;
        let mut resolution = resolve(interface, subject, budget)?;
        let Selection::Selected(selected) = resolution.types.selection else {
            let kind = match access {
                Access::TypeProjection => "Type",
                Access::Value => "value",
            };
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
                "associated {kind} implementation selection is unresolved: {} (remaining-steps {}) (pending {pending})",
                resolution.types, budget.steps
            )));
        };
        let Some(TypeAssessment::Applicable(evidence)) =
            resolution.types.assessments.remove(&selected)
        else {
            return Err("associated value selection lacks type evidence".into());
        };
        let Some(qualification @ Qualification::Proved(_)) =
            resolution.qualifications.remove(&selected)
        else {
            return Err("associated value selection lacks capability evidence".into());
        };
        let implementation = self
            .implementations
            .iter()
            .find(|i| i.declaration == selected)
            .ok_or("associated value selection has no implementation contract")?;
        if member.phase == nir::program::TraitPhase::Comptime && !implementation.phases.comptime {
            return Err("selected implementation lacks comptime trait phase".into());
        }
        Ok(SelectedAssociatedMember {
            world: self,
            row: implementation,
            member: member_id,
            member_type,
            evidence,
            qualification,
        })
    }
}

enum MemberSource<'a> {
    Value(&'a nir::program::Expr),
    Initializer(&'a AssociatedInitializer),
}

impl fmt::Display for SelectedAssociatedMember<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(selected-associated-member {} (implementation {}) (result {}) (arguments",
            self.member, self.evidence.implementation, self.member_type
        )?;
        for argument in &self.evidence.type_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") {})", self.qualification)
    }
}

impl SelectedAssociatedMember<'_, '_> {
    pub fn implementation(&self) -> nir::DeclarationId {
        self.evidence.implementation
    }

    pub fn implementation_arguments(&self) -> &[Term] {
        &self.evidence.type_arguments
    }

    pub fn member(&self) -> nir::DeclarationId {
        self.member
    }

    pub fn result_domain(&self) -> &Term {
        &self.member_type
    }

    /// Substitute the selected implementation arguments and admit its supply.
    pub fn supply(self, budget: &mut Budget) -> Result<AssociatedValueResolution, QueryError> {
        let source = self
            .source(&mut budget.steps)?
            .ok_or("selected implementation has no associated value")?;
        let supply = match source {
            MemberSource::Value(value) => {
                AssociatedSelection::Completed(self.complete_value(value, budget)?)
            }
            MemberSource::Initializer(template) => {
                let body = self.member_result(template.expression(), budget)?;
                AssociatedSelection::Initializer(AssociatedInitializer::new(
                    body,
                    &mut budget.steps,
                )?)
            }
        };
        Ok(AssociatedValueResolution {
            supply,
            evidence: self.evidence,
            qualification: self.qualification,
        })
    }

    fn complete_value(
        &self,
        value: &nir::program::Expr,
        budget: &mut Budget,
    ) -> Result<CompletedValue, QueryError> {
        let layouts = &self.world.index.nominal_layouts;
        let lookup_head = |id| collection_head(id, layouts, self.world.context);
        nir::completed_values::validate_in_layouts(
            value,
            layouts,
            &lookup_head,
            &mut budget.steps,
        )?;
        let value = self.member_result(value, budget)?;
        Ok(CompletedValue::new(
            value,
            layouts,
            &lookup_head,
            &mut budget.steps,
        )?)
    }

    fn member_result(
        &self,
        value: &nir::program::Expr,
        budget: &mut Budget,
    ) -> Result<nir::program::Expr, QueryError> {
        let mut value = self.instantiate(value, &mut budget.steps)?;
        // Compare completed type views without unfolding opaque normal-form heads.
        let mut cache = std::collections::HashMap::new();
        let mut normalize = |ty: &Term| {
            let result =
                self.world
                    .normalize_associated_types_mode(ty, budget, false, Some(&cache), None);
            let (normalized, _, completed) = result?;
            cache.extend(completed);
            Ok::<_, QueryError>(normalized)
        };
        let expected = normalize(&self.member_type)?;
        let actual = normalize(&value.ty)?;
        if actual.value != expected.value {
            return Err("selected associated value disagrees with its member contract".into());
        }
        value.ty = actual.value;
        Ok(value)
    }

    fn source(&self, fuel: &mut usize) -> Result<Option<MemberSource<'_>>, QueryError> {
        let entries = self
            .row
            .associated
            .iter()
            .map(|(id, value)| (id, MemberSource::Value(value)))
            .chain(
                self.row
                    .associated_initializers
                    .iter()
                    .map(|(id, template)| (id, MemberSource::Initializer(template))),
            );
        let mut found = None;
        for (id, entry) in entries {
            *fuel = fuel.checked_sub(1).ok_or(QueryError::Budget)?;
            if *id == self.member && found.replace(entry).is_some() {
                return Err("selected implementation has duplicate associated values".into());
            }
        }
        Ok(found)
    }

    fn instantiate(
        &self,
        value: &nir::program::Expr,
        fuel: &mut usize,
    ) -> Result<nir::program::Expr, QueryError> {
        let binders = self.row.parameter_binders().map_err(|e| e.to_string())?;
        nir::copy_budget::charge_substitution(value, &binders, &self.evidence.type_arguments, fuel)
            .map_err(|_| QueryError::Budget)?;
        let substitution = nir::DeclarationSubstitution::bounded(
            &self.row.type_parameters,
            &self.row.parameter_types,
            &self.evidence.type_arguments,
            &[],
            fuel,
        )?;
        let mut value = value.clone();
        value.rewrite_types(&mut |ty| substitution.instantiate_bounded(ty, fuel))?;
        Ok(value)
    }

    // A type projection consumes a completed Type template without invoking code.
    pub(super) fn complete_type(
        self,
        fuel: &mut usize,
    ) -> Result<AssociatedTypeResolution, QueryError> {
        let Some(source) = self.source(fuel)? else {
            return Err("selected implementation has no associated Type value".into());
        };
        let MemberSource::Value(source) = source else {
            return Err("selected associated value is not a completed Type template".into());
        };
        let completed = self.instantiate(source, fuel)?;
        if completed.ty != Term::Primitive(Primitive::Type) {
            return Err("selected associated value has a non-Type contract".into());
        }
        let ExprKind::TypeValue(value) = completed.kind else {
            return Err("selected associated value is not a completed Type template".into());
        };
        charge_projection_copy(&value, fuel)?;
        value.check_scope(&[]).map_err(|e| e.to_string())?;
        if value.sort() != nir::Sort::Type {
            return Err("selected associated Type template has the wrong sort".into());
        }
        Ok(AssociatedTypeResolution {
            value,
            evidence: self.evidence,
            qualification: self.qualification,
        })
    }
}

fn collection_head<'a>(
    id: nir::DeclarationId,
    layouts: &'a nir::program::Package,
    context: &crate::ptc::Context,
) -> Option<&'a nir::normal_forms::Head> {
    let heads = &layouts.normal_forms;
    let slot = heads
        .binary_search_by_key(&id, |metadata| metadata.declaration)
        .ok()?;
    // Duplicate declarations cannot provide unique layout authority.
    if slot
        .checked_sub(1)
        .is_some_and(|prior| heads[prior].declaration == id)
        || heads
            .get(slot + 1)
            .is_some_and(|next| next.declaration == id)
    {
        return None;
    }
    let head = &heads[slot];
    (head.role != nir::normal_forms::Role::Tuple || context.is_tuple_head(id)).then_some(head)
}
