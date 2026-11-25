//! Associated member selection and admission of its instantiated supply.
use super::*;
use crate::implementation::TypeAssessment;
use nir::associated_initializers::{
    AssociatedInitializer, AssociatedSelection, AssociatedSourceDemand, MemberSource,
};
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
    facts: SelectionFacts,
}

struct SelectionFacts {
    row: usize,
    member: nir::DeclarationId,
    member_type: Term,
    evidence: TypeEvidence,
    qualification: Qualification,
}

#[derive(Clone, Copy)]
pub(super) enum Access {
    TypeProjection,
    TypeCallProjection(usize),
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
        let Some(MemberSource::Value(value)) = selected
            .row()
            .associated_source(member, &mut budget.steps)?
        else {
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
        let substitution =
            contract.input_substitution(&inputs, self.context.scope(), &mut budget.steps)?;
        let member_type = substitution.instantiate_bounded(&member.ty, &mut budget.steps)?;
        match access {
            Access::TypeProjection => {
                if member_type != Term::Primitive(Primitive::Type) {
                    return Err("associated projection member is not Type-valued".into());
                }
            }
            Access::TypeCallProjection(arg_count) => {
                if !is_type_call_contract(&member_type, arg_count) {
                    return Err(
                        "associated type-call member must be a comptime callable returning Type"
                            .into(),
                    );
                }
            }
            Access::Value => {}
        }
        self.check_associated_formation(contract, &substitution, budget, resolve)?;
        let mut resolution = resolve(interface, subject, budget)?;
        let Selection::Selected(selected) = resolution.types.selection else {
            let kind = match access {
                Access::TypeProjection | Access::TypeCallProjection(_) => "Type",
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
        let mut rows = self.implementations.iter().enumerate();
        let (row, implementation) = loop {
            let (slot, row) = rows
                .next()
                .ok_or("associated value selection has no implementation contract")?;
            budget.steps = budget.steps.checked_sub(1).ok_or(QueryError::Budget)?;
            if row.declaration == selected {
                break (slot, row);
            }
        };
        if member.phase == nir::program::TraitPhase::Comptime && !implementation.phases.comptime {
            return Err("selected implementation lacks comptime trait phase".into());
        }
        Ok(SelectedAssociatedMember {
            world: self,
            facts: SelectionFacts {
                row,
                member: member_id,
                member_type,
                evidence,
                qualification,
            },
        })
    }
}

/// Only a successful implementation selection can request source. Absence is
/// distinct from an invalid payload, an undecided obligation or a budget limit.
pub enum AssociatedRead<Missing = AssociatedSourceDemand> {
    Ready(AssociatedValueResolution),
    NeedsSource(Missing),
}

/// A linear selection in its original source and premise scope. It can be
/// held across another declaration query without keeping a query cache borrowed.
pub struct PreparedAssociatedMember<'source> {
    scope: crate::dispatch::DeclarationScope<'source>,
    facts: SelectionFacts,
}

impl<'source> crate::dispatch::DeclarationScope<'source> {
    pub fn prepare_associated_member(
        self,
        interface: &InterfaceInstance,
        member: nir::DeclarationId,
        subject: &Term,
        budget: &mut Budget,
    ) -> Result<PreparedAssociatedMember<'source>, QueryError> {
        let facts = self
            .capabilities()
            .select_associated_member(interface, member, subject, budget)?
            .facts;
        Ok(PreparedAssociatedMember { scope: self, facts })
    }
}

impl<'source> PreparedAssociatedMember<'source> {
    pub fn read_in_source(self, budget: &mut Budget) -> Result<AssociatedRead, QueryError> {
        match self.read(budget)? {
            AssociatedRead::Ready(value) => Ok(AssociatedRead::Ready(value)),
            AssociatedRead::NeedsSource(selection) => selection.read_source(budget),
        }
    }

    pub fn read(
        self,
        budget: &mut Budget,
    ) -> Result<AssociatedRead<PendingAssociatedSource<'source>>, QueryError> {
        SelectedAssociatedMember {
            world: &self.scope.capabilities(),
            facts: self.facts,
        }
        .read_with(budget, |facts| {
            PendingAssociatedSource(PreparedAssociatedMember {
                scope: self.scope.clone(),
                facts,
            })
        })
    }
}

/// The checked row lacks this member. Only a source owner can satisfy it;
/// rereading the immutable row cannot advance the computation.
pub struct PendingAssociatedSource<'source>(PreparedAssociatedMember<'source>);

impl<'source> PendingAssociatedSource<'source> {
    pub fn source_slot(&self) -> (nir::DeclarationId, nir::DeclarationId) {
        (self.0.facts.evidence.implementation, self.0.facts.member)
    }

    /// Read from the original source owner after releasing the selection
    /// service. No replacement row, world or premises enter this admission.
    pub fn read_source(self, budget: &mut Budget) -> Result<AssociatedRead, QueryError> {
        Ok(match self.read_captured(budget)? {
            AssociatedRead::Ready(value) => AssociatedRead::Ready(value),
            AssociatedRead::NeedsSource(pending) => {
                AssociatedRead::NeedsSource(pending.into_source_demand())
            }
        })
    }

    /// Keep the selected row and its original evidence when its source is
    /// still being formed. A missing template never starts a second selection.
    pub fn read_captured(self, budget: &mut Budget) -> Result<AssociatedRead<Self>, QueryError> {
        let (owner, member) = self.source_slot();
        budget.steps = budget.steps.checked_sub(1).ok_or(QueryError::Budget)?;
        let Some(program) = self.0.scope.member_program(owner, member) else {
            return Ok(AssociatedRead::NeedsSource(self));
        };
        self.admit_source(MemberSource::Initializer(program), budget)
            .map(AssociatedRead::Ready)
    }

    pub fn for_data_owner(
        self,
        owner: &crate::dispatch::MemberDataSource,
    ) -> Result<SelectedMemberData<'source>, AssociatedSourceDemand> {
        let (row, member) = self.source_slot();
        if self.0.scope.accepts_data(owner, row, member) {
            Ok(SelectedMemberData(self))
        } else {
            Err(self.into_source_demand())
        }
    }

    fn admit_source(
        self,
        source: MemberSource<'_>,
        budget: &mut Budget,
    ) -> Result<AssociatedValueResolution, QueryError> {
        SelectedAssociatedMember {
            world: &self.0.scope.capabilities(),
            facts: self.0.facts,
        }
        .admit(source, budget)
    }

    pub fn into_source_demand(self) -> AssociatedSourceDemand {
        self.0.facts.into_source_demand()
    }
}

/// A pending read paired with the data owner registered by its source.
pub struct SelectedMemberData<'source>(PendingAssociatedSource<'source>);

impl SelectedMemberData<'_> {
    pub fn admit(
        self,
        value: &nir::program::Expr,
        budget: &mut Budget,
    ) -> Result<AssociatedValueResolution, QueryError> {
        self.0.admit_source(MemberSource::Value(value), budget)
    }
}

impl fmt::Display for PendingAssociatedSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(pending-associated-source {})", self.0)
    }
}

impl SelectionFacts {
    fn into_source_demand(self) -> AssociatedSourceDemand {
        AssociatedSourceDemand {
            implementation: self.evidence.implementation,
            arguments: self.evidence.type_arguments,
            member: self.member,
            result: self.member_type,
        }
    }
}

impl fmt::Display for PreparedAssociatedMember<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.facts.fmt(f)
    }
}

impl fmt::Display for SelectedAssociatedMember<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.facts.fmt(f)
    }
}

impl fmt::Display for SelectionFacts {
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

impl<'a, 'world> SelectedAssociatedMember<'a, 'world> {
    fn row(&self) -> &'world nir::program::Implementation {
        &self.world.implementations[self.facts.row]
    }

    pub fn implementation(&self) -> nir::DeclarationId {
        self.facts.evidence.implementation
    }

    pub fn implementation_arguments(&self) -> &[Term] {
        &self.facts.evidence.type_arguments
    }

    pub fn member(&self) -> nir::DeclarationId {
        self.facts.member
    }

    pub fn result_domain(&self) -> &Term {
        &self.facts.member_type
    }

    /// Substitute the selected implementation arguments and admit its supply.
    pub fn supply(self, budget: &mut Budget) -> Result<AssociatedValueResolution, QueryError> {
        match self.read(budget)? {
            AssociatedRead::Ready(value) => Ok(value),
            AssociatedRead::NeedsSource(_) => {
                Err("selected implementation has no associated value".into())
            }
        }
    }

    pub fn read(self, budget: &mut Budget) -> Result<AssociatedRead, QueryError> {
        self.read_with(budget, SelectionFacts::into_source_demand)
    }

    fn read_with<M>(
        self,
        budget: &mut Budget,
        missing: impl FnOnce(SelectionFacts) -> M,
    ) -> Result<AssociatedRead<M>, QueryError> {
        let Some(source) = self
            .row()
            .associated_source(self.facts.member, &mut budget.steps)?
        else {
            return Ok(AssociatedRead::NeedsSource(missing(self.facts)));
        };
        self.admit(source, budget).map(AssociatedRead::Ready)
    }

    fn admit(
        self,
        source: MemberSource<'_>,
        budget: &mut Budget,
    ) -> Result<AssociatedValueResolution, QueryError> {
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
            evidence: self.facts.evidence,
            qualification: self.facts.qualification,
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
            let result = self.world.normalize_associated_types_mode(
                ty,
                budget,
                ProjectionView::ConcreteType,
                Some(&cache),
                None,
            );
            let (normalized, _, completed) = result?;
            cache.extend(completed);
            Ok::<_, QueryError>(normalized)
        };
        let expected = normalize(&self.facts.member_type)?;
        let actual = normalize(&value.ty)?;
        if actual.value != expected.value {
            return Err("selected associated value disagrees with its member contract".into());
        }
        value.ty = actual.value;
        Ok(value)
    }

    fn instantiate(
        &self,
        value: &nir::program::Expr,
        fuel: &mut usize,
    ) -> Result<nir::program::Expr, QueryError> {
        Ok(value.instantiate_declaration_parameters_bounded(
            &self.row().type_parameters,
            &self.row().parameter_types,
            &self.facts.evidence.type_arguments,
            &[],
            fuel,
        )?)
    }

    // Selection reports executable Type supplies without invoking their code.
    pub(super) fn prepare_type(
        self,
        projection: &Term,
        budget: &mut Budget,
        captured: Option<&AssociatedInitializer>,
    ) -> Result<TypeProjection, QueryError> {
        budget.steps = budget.steps.checked_sub(1).ok_or(QueryError::Budget)?;
        let source = match (
            self.row()
                .associated_source(self.facts.member, &mut budget.steps)?,
            captured,
        ) {
            (Some(source), None) => source,
            (None, Some(program)) => MemberSource::Initializer(program),
            (Some(_), Some(_)) => return Err("duplicate associated Type supply".into()),
            (None, None) => {
                return Err("selected implementation has no associated Type value".into());
            }
        };
        let is_type_call = matches!(projection, Term::AssociatedTypeCall(_));
        let source = match source {
            MemberSource::Value(value) if is_type_call => {
                let completed = self.instantiate(value, &mut budget.steps)?;
                let program = AssociatedInitializer::new(completed, &mut budget.steps)?;
                return Ok(TypeProjection::Initializer(SelectedTypeInitializer {
                    source: program.expression().source,
                    program: program.identity(),
                    open_inputs: nir::type_computations::has_rigid_parameters_bounded(
                        projection,
                        &mut budget.steps,
                    )?,
                    evidence: self.facts.evidence,
                    qualification: self.facts.qualification,
                }));
            }
            MemberSource::Value(value) => value,
            MemberSource::Initializer(program) => {
                // A dependent result domain is compared after the selected substitution.
                if program.expression().ty != Term::Primitive(Primitive::Type) && !is_type_call {
                    self.member_result(program.expression(), budget)?;
                }
                return Ok(TypeProjection::Initializer(SelectedTypeInitializer {
                    program: program.identity(),
                    source: program.expression().source,
                    open_inputs: nir::type_computations::has_rigid_parameters_bounded(
                        projection,
                        &mut budget.steps,
                    )?,
                    evidence: self.facts.evidence,
                    qualification: self.facts.qualification,
                }));
            }
        };
        let fuel = &mut budget.steps;
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
        Ok(TypeProjection::Ready(AssociatedTypeResolution {
            value,
            evidence: self.facts.evidence,
            qualification: self.facts.qualification,
        }))
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
