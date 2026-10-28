//! Checked declaration world shared by annotation queries and dispatch closure.
//! This service never traverses or invokes function bodies.
use crate::{
    implementation::{ImplementationHead, Index},
    ptc::{Budget, Context},
};
use nir::{DeclarationId as Id, Term, program as n};
use std::collections::{BTreeMap, HashMap};

pub struct DeclarationWorld<'a> {
    normalized_types: HashMap<Term, Term>,
    package: &'a n::Package,
    index: Index,
    context: Context,
    assumptions: Vec<n::TraitRequirement>,
    declaration_parameter_sorts: BTreeMap<Id, nir::Sort>,
}
impl<'a> DeclarationWorld<'a> {
    pub fn new(
        package: &'a n::Package,
        budget: Budget,
        context: Option<(&Context, &[n::TraitRequirement])>,
    ) -> Result<(Self, usize), String> {
        let assumptions = context
            .map(|(_, assumptions)| assumptions.to_vec())
            .unwrap_or_default();
        let context = match context {
            Some((context, _)) => context.clone(),
            None => Context::from_package(package)
                .map_err(|reason| format!("dispatch type context: {reason}"))?,
        };
        let mut index = Index::for_target(package.target);
        index.set_normal_form_metadata(package.normal_forms.clone());
        index.sync_nominal_layouts(&package.structs, &package.enums, &package.newtypes);

        for implementation in &package.implementations {
            if implementation.interface.is_some() {
                index
                    .insert(
                        ImplementationHead {
                            parameter_types: implementation.parameter_types.clone(),
                            declaration: implementation.declaration,
                            interface: implementation.interface,
                            interface_arguments: implementation.interface_arguments.clone(),
                            subject: implementation.subject.clone(),
                            type_parameters: implementation.type_parameters.clone(),
                            requirements: implementation.requirements.clone(),
                        },
                        budget,
                    )
                    .map_err(|e| format!("dispatch index: {e:?}"))?;
            }
        }
        let mut input_fuel = budget.steps;
        let domains = package.declaration_parameter_domains(&mut input_fuel)?;
        let declaration_parameter_sorts = domains
            .iter()
            .map(|(id, domain)| {
                (
                    *id,
                    if *domain == Term::Primitive(nir::Primitive::Type) {
                        nir::Sort::Type
                    } else {
                        nir::Sort::ComptimeValue
                    },
                )
            })
            .collect();
        for (id, domain) in domains {
            index.register_parameter_domain(id, domain)?;
        }
        Ok((
            Self {
                normalized_types: HashMap::new(),
                package,
                index,
                context,
                assumptions,
                declaration_parameter_sorts,
            },
            input_fuel,
        ))
    }
    pub fn index(&self) -> &Index {
        &self.index
    }
    pub fn context(&self) -> &Context {
        &self.context
    }
    pub fn assumptions(&self) -> &[n::TraitRequirement] {
        &self.assumptions
    }
    pub fn input_binders(&self, parameters: &[Id]) -> Vec<nir::DeclarationParameter> {
        parameters
            .iter()
            .map(|id| nir::DeclarationParameter {
                declaration: *id,
                sort: self
                    .declaration_parameter_sorts
                    .get(id)
                    .copied()
                    .unwrap_or(nir::Sort::Type),
            })
            .collect()
    }
    pub fn normalized_type(
        &mut self,
        ty: &Term,
        budget: Budget,
        fuel: &mut usize,
    ) -> Result<Term, String> {
        crate::associated::charge_projection_copy(ty, fuel)?;
        if let Some(value) = self.normalized_types.get(ty) {
            crate::associated::charge_projection_copy(value, fuel)?;
            return Ok(value.clone());
        }
        let (normalized, remaining) = crate::capabilities::World {
            index: &self.index,
            implementations: &self.package.implementations,
            traits: &self.package.traits,
            assumptions: &self.assumptions,
            context: &self.context,
        }
        .normalize_associated_types_cached(
            ty,
            Budget {
                steps: *fuel,
                ..budget
            },
            &mut self.normalized_types,
        )?;
        *fuel = remaining;
        crate::associated::charge_projection_copy(&normalized.value, fuel)?;
        self.normalized_types
            .insert(ty.clone(), normalized.value.clone());
        Ok(normalized.value)
    }
}

/// Query a concrete annotation without executing or expanding any body.
/// Failed speculative queries consume their remaining allowance.
pub fn normalize_concrete_type(
    package: &n::Package,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Term, String> {
    let result = (|| {
        let budget = Budget {
            steps: *fuel,
            ..Budget::default()
        };
        let (mut world, remaining) = DeclarationWorld::new(package, budget, None)?;
        *fuel = remaining;
        world.normalized_type(ty, budget, fuel)
    })();
    if result.is_err() {
        *fuel = 0;
    }
    result
}
