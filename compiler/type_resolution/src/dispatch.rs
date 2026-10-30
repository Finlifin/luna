//! Checked declaration world shared by annotation queries and dispatch closure.
//! This service never traverses or invokes function bodies.
use crate::{
    implementation::{ImplementationHead, Index, Selection, TypeAssessment},
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
        let mut index = Index::for_builtin_packages(
            package.target,
            package.builtin_implementation_packages.iter().copied(),
        );
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
    pub fn input_domains(&self, parameters: &[Id], fuel: &mut usize) -> Result<Vec<Term>, String> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or("input domain budget exhausted")?;
        parameters
            .iter()
            .map(|id| {
                let domain = self
                    .index
                    .parameter_domains()
                    .get(id)
                    .unwrap_or(&Term::Primitive(nir::Primitive::Type));
                nir::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
                Ok(domain.clone())
            })
            .collect()
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

impl DeclarationWorld<'_> {
    /// Select the implementation relationship for already normalized inputs.
    /// This does not close its executable dependencies or grant memory access.
    pub fn select_interface(
        &self,
        interface: &n::InterfaceInstance,
        subject: &Term,
        budget: Budget,
    ) -> Result<(Id, Vec<Term>), String> {
        let resolution = crate::capabilities::World {
            index: &self.index,
            implementations: &self.package.implementations,
            traits: &self.package.traits,
            assumptions: &self.assumptions,
            context: &self.context,
        }
        .resolve_instance(interface, subject, budget)?
        .types;
        let Selection::Selected(id) = resolution.selection else {
            return Err(format!(
                "dispatch selection is unresolved for {subject}: {}; candidates: {}",
                resolution.selection,
                resolution
                    .assessments
                    .iter()
                    .map(|(id, assessment)| format!("{id}: {assessment}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        };
        let Some(TypeAssessment::Applicable(evidence)) = resolution.assessments.get(&id) else {
            return Err("dispatch selection lacks evidence".into());
        };
        Ok((id, evidence.type_arguments.clone()))
    }

    /// Substitute a selected member's complete declaration-side contract.
    /// The caller must still discharge requirements and close capabilities,
    /// signature types, predicate expressions and the selected body.
    pub fn prepare_method(
        &self,
        instance: &n::DispatchInstance,
        member: Id,
        method_arguments: &[Term],
        runtime: bool,
        fuel: &mut usize,
    ) -> Result<(n::MethodInstance, n::MethodContract), String> {
        let contract = self
            .package
            .traits
            .iter()
            .find(|t| t.declaration == instance.interface)
            .ok_or("missing method interface")?;
        let template = contract
            .methods
            .iter()
            .find(|m| m.declaration == member)
            .ok_or("unknown dispatch member")?;
        if template.is_quote && runtime {
            return Err("trait quote method requires deterministic expansion after implementation selection; runtime dispatch is not permitted".into());
        }
        if template.type_parameters.len() != method_arguments.len() {
            return Err("dispatch method type argument count mismatch".into());
        }
        let parameters: Vec<_> = contract
            .binders_with_self(fuel)?
            .into_iter()
            .chain(self.input_binders(&template.type_parameters))
            .collect();
        let arguments: Vec<_> = std::iter::once(instance.subject.clone())
            .chain(instance.interface_arguments.iter().cloned())
            .chain(method_arguments.iter().cloned())
            .collect();
        let implementation = self
            .package
            .implementations
            .iter()
            .find(|i| i.declaration == instance.implementation)
            .ok_or("missing selected implementation")?;
        let body = implementation
            .methods
            .iter()
            .find(|(name, _)| name == &template.name)
            .ok_or("missing selected method body")?
            .1;
        let mut body_arguments = if template.default == Some(body) {
            std::iter::once(instance.subject.clone())
                .chain(instance.interface_arguments.iter().cloned())
                .collect::<Vec<_>>()
        } else {
            instance.arguments.clone()
        };
        body_arguments.extend(method_arguments.iter().cloned());
        let mut specialized = template.clone();
        specialized.type_arguments = method_arguments.to_vec();
        specialized.type_parameters.clear();
        specialized.default = None;
        specialized.signature = template
            .signature
            .instantiate_parameters(&parameters, &arguments)
            .map_err(|e| e.to_string())?;
        specialized.program = template
            .program
            .instantiate_parameters(&parameters, &arguments)
            .map_err(|e| e.to_string())?;
        specialized.requirements = template
            .requirements
            .iter()
            .map(|r| r.instantiate_parameters(&parameters, &arguments))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        specialized.capabilities = template
            .capabilities
            .iter()
            .map(|r| r.instantiate_parameters(&parameters, &arguments))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let method = n::MethodInstance {
            member,
            body,
            arguments: body_arguments,
            method_arguments: method_arguments.to_vec(),
        };
        Ok((method, specialized))
    }
}
