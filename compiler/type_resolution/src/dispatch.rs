//! Checked declaration world shared by annotation queries and dispatch closure.
//! Queries never traverse or invoke function bodies. Source ownership capture
//! separately preflights copied payloads without interpreting them.
use crate::declaration_budget as construction;
use crate::{
    implementation::{Index, Selection, TypeAssessment},
    ptc::{Budget, Context},
};
use nir::value_admission::Error as AdmissionError;
use nir::{DeclarationId as Id, Term, program as n};
use std::{collections::HashMap, sync::Arc};
mod catalog;
pub use catalog::{
    BorrowedCatalog, ClosedDeclarations, DeclarationSnapshot, DeclarationSource, MemberDataSource,
};
mod inputs;
mod methods;

pub struct DeclarationWorld<'a> {
    local_type_observations: n::ObservedAssociatedTypes,
    normalized_types: HashMap<Term, Term>,
    scope: DeclarationScope<'a>,
}

/// Immutable declaration and premise authority. Retaining a scope does not
/// retain an exclusive borrow of the method normalization cache.
#[derive(Clone)]
pub struct DeclarationScope<'a> {
    source: DeclarationSource<'a>,
    metadata: Arc<catalog::DeclarationMetadata>,
    context: Context,
    assumptions: Arc<[n::TraitRequirement]>,
}
impl<'a> DeclarationScope<'a> {
    pub(crate) fn observed_associated_types(&self) -> &n::ObservedAssociatedTypes {
        &self.source.package().observed_associated_types
    }
    pub(crate) fn accepts_data(&self, owner: &MemberDataSource, row: Id, member: Id) -> bool {
        matches!(self.source, DeclarationSource::Snapshot(snapshot) if snapshot.accepts_data(owner, row, member))
    }

    pub(crate) fn member_program(
        &self,
        owner: Id,
        member: Id,
    ) -> Option<&'a nir::associated_initializers::AssociatedInitializer> {
        match self.source {
            DeclarationSource::Borrowed(_) | DeclarationSource::Prepared(_) => None,
            DeclarationSource::Snapshot(snapshot) => {
                snapshot.member_programs().get(&owner)?.get(&member)
            }
        }
    }

    /// A query view of this exact source, declaration index and premise scope.
    /// It carries no executable contracts or persistent proof authority.
    pub fn capabilities(&self) -> crate::capabilities::World<'_> {
        self.metadata
            .capabilities(self.source.package(), &self.context, &self.assumptions)
    }
}
impl<'a> DeclarationWorld<'a> {
    pub fn new(
        package: &'a n::Package,
        budget: Budget,
        context: Option<(&Context, &[n::TraitRequirement])>,
    ) -> Result<(Self, usize), String> {
        let mut budget = budget;
        Self::new_metered(package, &mut budget, context)
            .map(|world| (world, budget.steps))
            .map_err(String::from)
    }

    /// Build privately using one allowance. Failed construction publishes no
    /// index or world and returns its unspent budget to the caller.
    pub fn new_metered(
        package: &'a n::Package,
        budget: &mut Budget,
        context: Option<(&Context, &[n::TraitRequirement])>,
    ) -> Result<Self, AdmissionError> {
        Self::from_source_metered(DeclarationSource::Borrowed(package), budget, context)
    }

    pub fn from_source_metered(
        source: DeclarationSource<'a>,
        budget: &mut Budget,
        context: Option<(&Context, &[n::TraitRequirement])>,
    ) -> Result<Self, AdmissionError> {
        construction::requirements(
            context.map_or(&[], |(_, assumptions)| assumptions),
            &mut budget.steps,
        )?;
        let assumptions = Arc::from(context.map_or(&[][..], |(_, assumptions)| assumptions));
        let context = match context {
            Some((context, _)) => context.clone(),
            None => source.closed_context(&mut budget.steps)?,
        };
        let metadata = source.metadata(budget)?;
        Ok(Self {
            local_type_observations: Default::default(),
            normalized_types: HashMap::new(),
            scope: DeclarationScope {
                source,
                metadata,
                context,
                assumptions,
            },
        })
    }
    /// Borrow checked metadata without scanning or copying executable bodies.
    pub fn function_declaration(&self, declaration: Id) -> Option<&'a n::Function> {
        self.scope
            .metadata
            .function(self.scope.source.package(), declaration)
    }
    pub fn runtime_function_identity(
        &self,
        declaration: Id,
    ) -> Option<&'a n::RuntimeFunctionIdentity> {
        self.scope
            .metadata
            .runtime_callables
            .get(&declaration)
            .map(|slot| &self.scope.source.package().runtime_function_identities[*slot])
    }
    pub fn function_formation(
        &self,
        declaration: Id,
    ) -> Option<nir::callable_source::Formation<'a>> {
        self.runtime_function_identity(declaration)
            .map(Into::into)
            .or_else(|| {
                self.function_declaration(declaration)
                    .map(|f| (&f.interface).into())
            })
    }

    fn normal_head(&self, declaration: Id) -> Option<&'a nir::normal_forms::Head> {
        self.scope
            .metadata
            .normal_heads
            .get(&declaration)
            .map(|slot| &self.scope.source.package().normal_forms[*slot])
    }

    pub fn scope(&self) -> &DeclarationScope<'a> {
        &self.scope
    }

    pub fn capabilities(&self) -> crate::capabilities::World<'_> {
        self.scope.capabilities()
    }

    pub fn index(&self) -> &Index {
        &self.scope.metadata.index
    }
    pub fn context(&self) -> &Context {
        &self.scope.context
    }
    pub fn assumptions(&self) -> &[n::TraitRequirement] {
        &self.scope.assumptions
    }
    pub fn input_domains(
        &self,
        parameters: &[Id],
        fuel: &mut usize,
    ) -> Result<Vec<Term>, AdmissionError> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(AdmissionError::Budget)?;
        parameters
            .iter()
            .map(|id| {
                let domain = self
                    .scope
                    .metadata
                    .index
                    .parameter_domains()
                    .get(id)
                    .unwrap_or(&Term::Primitive(nir::Primitive::Type));
                nir::type_computations::charge_term(domain, fuel)?;
                Ok(domain.clone())
            })
            .collect()
    }
    /// Signature evidence for a frozen callable, without body traversal.
    /// Missing declarations return `Ok(None)`; malformed inputs and exhausted
    /// budgets remain distinct failures. Formation predicates are discharged by
    /// the enclosing admission pass.
    pub fn callable_signature(
        &self,
        declaration: Id,
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<Option<Term>, AdmissionError> {
        *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
        if let Some(head) = self.normal_head(declaration) {
            if !arguments.is_empty() {
                return Err("normal-form callable identity cannot carry applied inputs".into());
            }
            *fuel = fuel
                .checked_sub(head.parameters.len())
                .ok_or(AdmissionError::Budget)?;
            return head
                .callable_type_in(fuel)
                .map(Some)
                .map_err(AdmissionError::from);
        }
        let Some(formation) = self.function_formation(declaration) else {
            return Ok(None);
        };
        let signature = self.specialized_callable_signature(
            formation.parameters,
            formation.signature,
            arguments,
            frames,
            fuel,
        )?;
        Ok(Some(formation.qualify(signature)))
    }

    pub fn callable_value_signature(
        &self,
        value: nir::CallableValueRef<'_>,
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<Option<Term>, AdmissionError> {
        match value {
            nir::CallableValueRef::Function {
                declaration,
                arguments,
            } => self.callable_signature(declaration, arguments, frames, fuel),
            nir::CallableValueRef::Method(method) => {
                *fuel = fuel
                    .checked_sub(self.scope.source.package().traits.len().saturating_add(1))
                    .ok_or(AdmissionError::Budget)?;
                let Some(interface) = self
                    .scope
                    .source
                    .package()
                    .traits
                    .iter()
                    .find(|t| t.declaration == method.interface.declaration)
                else {
                    return Ok(None);
                };
                *fuel = fuel
                    .checked_sub(interface.methods.len().saturating_add(1))
                    .ok_or(AdmissionError::Budget)?;
                let Some(contract) = interface
                    .methods
                    .iter()
                    .find(|m| m.declaration == method.member)
                else {
                    return Ok(None);
                };
                if method.interface.arguments.len() != interface.type_parameters.len()
                    || method.method_arguments.len() != contract.type_parameters.len()
                    || contract.signature.is_comptime_callable()
                {
                    return Err("method identity has incompatible inputs or callable phase".into());
                }
                *fuel = fuel
                    .checked_sub(
                        interface
                            .type_parameters
                            .len()
                            .saturating_add(contract.type_parameters.len())
                            .saturating_add(1),
                    )
                    .ok_or(AdmissionError::Budget)?;
                let mut parameters = interface.parameters_with_self();
                parameters.extend(&contract.type_parameters);
                let inputs: Vec<_> = method.terms().collect();
                self.specialized_callable_signature(
                    &parameters,
                    contract
                        .signature
                        .ordinary()
                        .map_err(|e| AdmissionError::Invalid(e.to_string()))?,
                    &inputs,
                    frames,
                    fuel,
                )
                .map(Some)
            }
        }
    }

    fn specialized_callable_signature<T: std::borrow::Borrow<Term>>(
        &self,
        parameters: &[Id],
        signature: &Term,
        arguments: &[T],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<Term, AdmissionError> {
        let inputs = self.admit_inputs(
            parameters,
            arguments,
            nir::DeclarationInputScope::Caller(frames),
            fuel,
        )?;
        Ok(inputs.instantiate(signature, fuel)?)
    }

    /// Check declaration arguments in order. A dependent slot's domain can
    /// become Type only after its preceding arguments have been admitted.
    pub fn admit_inputs<'v, T: std::borrow::Borrow<Term>>(
        &self,
        parameters: &[Id],
        arguments: &'v [T],
        scope: nir::DeclarationInputScope<'v>,
        fuel: &mut usize,
    ) -> Result<nir::AdmittedArguments<'v, T>, AdmissionError> {
        let domains = nir::function_parameters::ordered_domains(
            self.index().parameter_domains(),
            parameters,
            fuel,
        )?;
        self.with_admission_queries(|queries| {
            nir::admit_declaration_inputs(parameters, &domains, arguments, scope, 0, fuel, queries)
                .map_err(|failure| failure.reason)
        })
    }

    /// A concrete materialized body uses the same input domains and receiving
    /// world as a callable signature query. Program contracts stay on the body.
    pub fn instantiate_function(
        &self,
        template: &n::Function,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<n::Function, AdmissionError> {
        let domains = nir::function_parameters::ordered_domains(
            self.index().parameter_domains(),
            &template.type_parameters,
            fuel,
        )?;
        self.with_admission_queries(|queries| {
            template.instantiate_admitted_parameters(&domains, arguments, fuel, queries)
        })
    }

    /// Supply receiving-world representation evidence for ordinary and opaque
    /// inputs without evaluating a head or a callable body. Predicate formation
    /// remains the responsibility of the enclosing declaration check.
    pub fn with_admission_queries<R>(
        &self,
        check: impl FnOnce(&mut nir::value_admission::DomainQueries<'_, '_>) -> R,
    ) -> R {
        use nir::value_admission::DomainQueries;
        check(&mut DomainQueries {
            layout: &mut |ty, variant, fuel| {
                if variant.is_none() {
                    if let Some(fields) =
                        nir::normal_forms::registered_tuple_fields(ty, |id| self.normal_head(id))
                    {
                        nir::type_computations::charge_term(ty, fuel)?;
                        return Ok(fields.to_vec());
                    }
                }
                nir::normal_forms::query_nominal_value_fields(
                    self.scope.source.package(),
                    ty,
                    variant,
                    fuel,
                )
            },
            declared: &mut |argument, domain, fuel| {
                nir::type_computations::charge_term(domain, fuel)?;
                match argument {
                    Term::RuntimeField { .. } => Ok(nir::runtime_fields::declared_domain(
                        argument,
                        self.index().parameter_domains(),
                        self.index().runtime_domains(),
                        Some(self.scope.source.package()),
                        &[],
                        fuel,
                    )?
                    .as_ref()
                        == Some(domain)),
                    Term::Runtime(id) => Ok(self
                        .index()
                        .runtime_domains()
                        .get(id)
                        .is_some_and(|declared| declared == domain)),
                    Term::Rigid {
                        declaration,
                        sort: nir::Sort::Runtime,
                    } => Ok(self
                        .index()
                        .parameter_domains()
                        .get(declaration)
                        .is_some_and(|declared| declared == domain)),
                    _ => Ok(false),
                }
            },
            collection: &mut |domain, fuel| {
                let Term::NormalForm { head, .. } = domain else {
                    return Ok(None);
                };
                let Some(element) = self.normal_head(*head).and_then(|h| h.list_element(domain))
                else {
                    return Ok(None);
                };
                nir::type_computations::charge_term(element, fuel)?;
                Ok(Some(element.clone()))
            },
            callable: &mut |value, frames, fuel| {
                self.callable_value_signature(value, frames, fuel)
                    .map(|signature| signature.map(std::borrow::Cow::Owned))
            },
            target: self.scope.source.package().target,
        })
    }

    pub fn input_binders(&self, parameters: &[Id]) -> Vec<nir::DeclarationParameter> {
        nir::DeclarationParameter::from_domains(parameters, self.index().parameter_domains())
    }
    pub fn delivered_scoped_type_view(
        &self,
        ty: &Term,
        budget: &mut Budget,
        cache: &mut HashMap<Term, Term>,
    ) -> Result<crate::associated::AssociatedTypeNormalization, AdmissionError> {
        let result = self.scope.delivered_type_view(
            ty,
            budget,
            cache,
            true,
            &self.local_type_observations,
            None,
        )?;
        cache.extend(result.completed);
        Ok(result.view)
    }
    /// Execute missing closed initializers on demand. New identities become
    /// available to subsequent views only after the entire traversal succeeds.
    pub fn scoped_type_with_host<'h>(
        &mut self,
        ty: &Term,
        budget: &mut Budget,
        cache: &mut HashMap<Term, Term>,
        host: &'h mut crate::associated::TypeInitializerHost<'h>,
    ) -> Result<crate::associated::AssociatedTypeNormalization, AdmissionError> {
        let result = self.scope.delivered_type_view(
            ty,
            budget,
            cache,
            true,
            &self.local_type_observations,
            Some(host),
        )?;
        self.local_type_observations
            .publish(result.observations, &mut budget.steps)?;
        cache.extend(result.completed);
        Ok(result.view)
    }
    pub fn normalized_type(
        &mut self,
        ty: &Term,
        budget: Budget,
        fuel: &mut usize,
    ) -> Result<Term, AdmissionError> {
        self.normalized_type_with_host(ty, budget, fuel, None)
    }
    /// Require a closed identity, optionally asking the source VM for missing
    /// initializers. Passive readers can consume completed facts in this world.
    pub fn normalized_type_with_host<'h>(
        &mut self,
        ty: &Term,
        budget: Budget,
        fuel: &mut usize,
        host: Option<&'h mut crate::associated::TypeInitializerHost<'h>>,
    ) -> Result<Term, AdmissionError> {
        crate::associated::charge_projection_copy(ty, fuel)?;
        if let Some(value) = self.normalized_types.get(ty) {
            crate::associated::charge_projection_copy(value, fuel)?;
            return Ok(value.clone());
        }
        let mut query_budget = Budget {
            steps: *fuel,
            ..budget
        };
        let prepared = self.scope.delivered_type_view(
            ty,
            &mut query_budget,
            &self.normalized_types,
            false,
            &self.local_type_observations,
            host,
        );
        *fuel = query_budget.steps;
        let result = prepared?;
        let normalized = result.view;
        // Retaining the root is part of this transaction. Failed retention
        // cannot expose its successfully normalized children to later queries.
        crate::associated::charge_projection_copy(&normalized.value, fuel)?;
        self.local_type_observations
            .publish(result.observations, fuel)?;
        self.normalized_types.extend(result.completed);
        self.normalized_types
            .insert(ty.clone(), normalized.value.clone());
        Ok(normalized.value)
    }
}

/// Query a concrete annotation without executing or expanding any body.
/// Construction and normalization share one allowance on every outcome.
pub fn normalize_concrete_type(
    package: &n::Package,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Term, AdmissionError> {
    let mut budget = Budget {
        steps: *fuel,
        ..Budget::default()
    };
    let world = DeclarationWorld::new_metered(package, &mut budget, None);
    *fuel = budget.steps;
    let mut world = world?;
    world.normalized_type(ty, budget, fuel)
}

impl DeclarationWorld<'_> {
    /// Select the implementation relationship for already normalized inputs.
    /// This does not close its executable dependencies or grant memory access.
    pub fn select_interface(
        &self,
        interface: &n::InterfaceInstance,
        subject: &Term,
        budget: &mut Budget,
    ) -> Result<(Id, Vec<Term>), AdmissionError> {
        let mut resolution = self
            .capabilities()
            .resolve_instance_metered(interface, subject, budget)?
            .types;
        let Selection::Selected(id) = resolution.selection else {
            return Err(AdmissionError::Unresolved(format!(
                "dispatch selection is unresolved for {subject}: {}; candidates: {}",
                resolution.selection,
                resolution
                    .assessments
                    .iter()
                    .map(|(id, assessment)| format!("{id}: {assessment}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        };
        let Some(TypeAssessment::Applicable(evidence)) = resolution.assessments.remove(&id) else {
            return Err("dispatch selection lacks evidence".into());
        };
        Ok((id, evidence.type_arguments))
    }

    fn method_inputs(
        &self,
        instance: &n::DispatchInstance,
        member: Id,
        method_arguments: &[Term],
        runtime: bool,
        fuel: &mut usize,
    ) -> Result<MethodInputs<'_>, AdmissionError> {
        let contract = self
            .scope
            .source
            .package()
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
        *fuel = fuel
            .checked_sub(
                contract
                    .type_parameters
                    .len()
                    .saturating_add(template.type_parameters.len())
                    .saturating_add(1),
            )
            .ok_or(AdmissionError::Budget)?;
        for term in std::iter::once(&instance.subject)
            .chain(&instance.interface_arguments)
            .chain(method_arguments)
            .chain(&instance.arguments)
        {
            nir::type_computations::charge_term(term, fuel)?;
        }
        let parameters: Vec<_> = contract
            .parameters_with_self()
            .into_iter()
            .chain(template.type_parameters.iter().copied())
            .collect();
        let parameter_domains = self.input_domains(&parameters, fuel)?;
        let arguments: Vec<_> = std::iter::once(instance.subject.clone())
            .chain(instance.interface_arguments.iter().cloned())
            .chain(method_arguments.iter().cloned())
            .collect();
        let implementation = self
            .scope
            .source
            .package()
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
        let method = n::MethodInstance {
            member,
            body,
            arguments: body_arguments,
            method_arguments: method_arguments.to_vec(),
        };
        Ok(MethodInputs {
            template,
            parameters,
            parameter_domains,
            arguments,
            method,
        })
    }

    /// Return code identity and substitutions without copying or visiting the
    /// member's executable contracts. Actual call admission belongs to the VM.
    pub fn prepare_method_identity(
        &self,
        instance: &n::DispatchInstance,
        member: Id,
        method_arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<n::MethodInstance, AdmissionError> {
        let inputs = self.method_inputs(instance, member, method_arguments, false, fuel)?;
        nir::DeclarationSubstitution::bounded(
            &inputs.parameters,
            &inputs.parameter_domains,
            &inputs.arguments,
            &[],
            fuel,
        )?;
        Ok(inputs.method)
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
    ) -> Result<(n::MethodInstance, n::MethodContract), AdmissionError> {
        let MethodInputs {
            template,
            parameters,
            parameter_domains,
            arguments,
            method,
        } = self.method_inputs(instance, member, method_arguments, runtime, fuel)?;
        let mut specialized = template.clone();
        specialized.type_arguments = method_arguments.to_vec();
        specialized.type_parameters.clear();
        specialized.default = None;
        let substitution = nir::DeclarationSubstitution::bounded(
            &parameters,
            &parameter_domains,
            &arguments,
            &[],
            fuel,
        )?;
        specialized.signature = template
            .signature
            .try_map_types(|ty| substitution.instantiate_bounded(ty, fuel))?;
        let frames = match template.signature.callable_signature() {
            Ok(Term::Function { parameters, .. }) => {
                vec![parameters.iter().map(|p| p.sort).collect()]
            }
            _ => vec![],
        };
        let scoped = nir::DeclarationSubstitution::bounded(
            &parameters,
            &parameter_domains,
            &arguments,
            &frames,
            fuel,
        )?;
        specialized.program = template.program.instantiate_declaration_parameters(
            &parameters,
            &parameter_domains,
            &arguments,
            &frames,
            fuel,
        )?;
        specialized.requirements = template
            .requirements
            .iter()
            .map(|r| scoped.instantiate_predicate_bounded(r, fuel))
            .collect::<Result<_, _>>()?;
        specialized.capabilities = template
            .capabilities
            .iter()
            .map(|r| {
                let n::TypePredicate::Trait(value) = scoped
                    .instantiate_predicate_bounded(&n::TypePredicate::Trait(r.clone()), fuel)?
                else {
                    unreachable!()
                };
                Ok(value)
            })
            .collect::<Result<_, nir::type_computations::Error>>()?;

        Ok((method, specialized))
    }
}

struct MethodInputs<'a> {
    template: &'a n::MethodContract,
    parameters: Vec<Id>,
    parameter_domains: Vec<Term>,
    arguments: Vec<Term>,
    method: n::MethodInstance,
}
