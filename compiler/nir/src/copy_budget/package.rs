//! Bounded ownership transfer for a declaration source, not executable dispatch.
use super::{CopyBudgetExhausted as Error, Node, charge_node};
use crate::{Term, code_interfaces as code, program as n};

type Result<T = ()> = std::result::Result<T, Error>;

/// Bound structural observation or copying of one declaration's metadata.
/// Shared owners need no structural equality walk; distinct owners do.
pub fn charge_trait_metadata(value: &n::Trait, fuel: &mut usize) -> Result {
    Meter {
        fuel,
        follow_shared: true,
    }
    .interface(value)
}

pub fn charge_implementation_metadata(value: &n::Implementation, fuel: &mut usize) -> Result {
    Meter {
        fuel,
        follow_shared: true,
    }
    .implementation(value)
}

/// Preflight the borrowed nominal part of a representation world. No owned
/// package or descriptor clone is needed to account for names, contracts,
/// fields, variants and encodings. This grants no storage or type authority.
pub fn charge_representation_metadata<'a>(
    structs: impl IntoIterator<Item = &'a n::Struct>,
    newtypes: impl IntoIterator<Item = &'a crate::newtypes::Newtype>,
    enums: impl IntoIterator<Item = &'a n::Enum>,
    fuel: &mut usize,
) -> Result {
    let mut meter = Meter {
        fuel,
        follow_shared: false,
    };
    meter.width(1)?;
    for definition in structs {
        meter.width(1)?;
        meter.structure(definition)?;
    }
    for definition in newtypes {
        meter.width(1)?;
        meter.newtype(definition)?;
    }
    for definition in enums {
        meter.width(1)?;
        meter.enumeration(definition)?;
    }
    Ok(())
}

/// Charge a head assembled from borrowed input domains and predicates before
/// copying those payloads. Capabilities become ordinary trait predicates in
/// the resulting head, so both forms use the same structural copy visitor.
pub fn charge_normal_form_payload<'a>(
    domains: impl IntoIterator<Item = &'a Term>,
    predicates: &[n::TypePredicate],
    capabilities: &[n::TraitRequirement],
    fuel: &mut usize,
) -> Result {
    let mut meter = Meter {
        fuel,
        follow_shared: false,
    };
    meter.width(1)?;
    meter.normal_form_payload(domains, predicates, capabilities)
}

/// Copy a source's owned declaration payloads after a complete preflight. The
/// selected dispatch table is neither traversed nor copied: it belongs to a
/// different execution view. This grants no phase, type or resource authority.
/// Immutable closure bodies retain their ordinary Arc-sharing behavior.
pub fn clone_declaration_source(package: &n::Package, fuel: &mut usize) -> Result<n::Package> {
    // Exhaustive destructuring makes a new package field require an explicit
    // ownership/accounting decision here, rather than inheriting a hidden clone.
    let n::Package {
        builtin_implementation_packages,
        target,
        tests,
        function_parameters,
        runtime_instances,
        quote_templates,
        quote_identity_cursor,
        definition_identity_cursor,
        internal_return_packets,
        internal_return_empty_replacements,
        expansion_sources,
        effects,
        runtime_function_identities,
        normal_forms,
        dispatch_instances: _,
        traits,
        implementations,
        functions,
        structs,
        newtypes,
        alias_contracts,
        enums,
    } = package;
    let mut meter = Meter {
        fuel,
        follow_shared: false,
    };
    meter.width(1)?;
    for count in [
        builtin_implementation_packages.len(),
        tests.len(),
        function_parameters.len(),
        runtime_instances.len(),
        quote_templates.len(),
        internal_return_packets.len(),
        internal_return_empty_replacements.len(),
        expansion_sources.len(),
        effects.len(),
        runtime_function_identities.len(),
        normal_forms.len(),
        traits.len(),
        implementations.len(),
        functions.len(),
        structs.len(),
        newtypes.len(),
        alias_contracts.len(),
        enums.len(),
    ] {
        meter.width(count)?;
    }
    for declaration in function_parameters {
        let crate::function_parameters::Declaration {
            declaration: _,
            parameters,
            domains,
        } = declaration;
        meter.width(parameters.len())?;
        meter.terms(domains)?;
    }
    for instance in runtime_instances {
        let n::RuntimeInstance {
            declaration: _,
            arguments,
            function,
            cleanup_accesses,
        } = instance;
        meter.terms(arguments)?;
        meter.node(Node::Function(function))?;
        meter.width(cleanup_accesses.len())?;
        for access in cleanup_accesses {
            meter.cleanup(access)?;
        }
    }
    for function in functions.iter().chain(quote_templates) {
        meter.node(Node::Function(function))?;
    }
    for effect in effects {
        let crate::effects::Declaration {
            declaration: _,
            group: _,
            name,
            type_parameters,
            parameter_types,
            signature,
            abi: _,
            inline: _,
            requirements,
            capabilities,
        } = effect;
        meter.width(name.len())?;
        meter.width(type_parameters.len())?;
        meter.terms(parameter_types)?;
        meter.node(Node::Type(signature))?;
        meter.contracts(requirements, capabilities)?;
    }
    for identity in runtime_function_identities {
        let n::RuntimeFunctionIdentity {
            declaration: _,
            type_parameters,
            signature,
        } = identity;
        meter.width(type_parameters.len())?;
        meter.node(Node::Type(signature))?;
    }
    for head in normal_forms {
        meter.head(head)?;
    }
    for contract in traits {
        meter.interface(contract)?;
    }
    for implementation in implementations {
        meter.implementation(implementation)?;
    }
    for definition in structs {
        meter.structure(definition)?;
    }
    for definition in newtypes {
        meter.newtype(definition)?;
    }
    for contract in alias_contracts {
        let crate::type_aliases::Contract {
            declaration: _,
            parameters,
            parameter_types,
            requirements,
            capabilities,
        } = contract;
        meter.width(parameters.len())?;
        meter.terms(parameter_types)?;
        meter.contracts(requirements, capabilities)?;
    }
    for definition in enums {
        meter.enumeration(definition)?;
    }
    // No clone above this point. Failure leaves the caller's source untouched
    // and cannot expose a partially captured declaration world.
    Ok(n::Package {
        builtin_implementation_packages: builtin_implementation_packages.clone(),
        target: *target,
        tests: tests.clone(),
        function_parameters: function_parameters.clone(),
        runtime_instances: runtime_instances.clone(),
        quote_templates: quote_templates.clone(),
        quote_identity_cursor: *quote_identity_cursor,
        definition_identity_cursor: *definition_identity_cursor,
        internal_return_packets: internal_return_packets.clone(),
        internal_return_empty_replacements: internal_return_empty_replacements.clone(),
        expansion_sources: expansion_sources.clone(),
        effects: effects.clone(),
        runtime_function_identities: runtime_function_identities.clone(),
        normal_forms: normal_forms.clone(),
        dispatch_instances: vec![],
        traits: traits.clone(),
        implementations: implementations.clone(),
        functions: functions.clone(),
        structs: structs.clone(),
        newtypes: newtypes.clone(),
        alias_contracts: alias_contracts.clone(),
        enums: enums.clone(),
    })
}

struct Meter<'a> {
    fuel: &'a mut usize,
    follow_shared: bool,
}
impl Meter<'_> {
    fn width(&mut self, count: usize) -> Result {
        *self.fuel = self.fuel.checked_sub(count).ok_or(Error::Steps)?;
        Ok(())
    }
    fn node(&mut self, node: Node<'_>) -> Result {
        let mut steps = *self.fuel as u64;
        let result = charge_node(node, &mut steps, self.follow_shared, None);
        *self.fuel = steps as usize;
        result
    }
    fn terms(&mut self, terms: &[Term]) -> Result {
        self.width(terms.len())?;
        for term in terms {
            self.node(Node::Type(term))?;
        }
        Ok(())
    }
    fn contracts(
        &mut self,
        predicates: &[n::TypePredicate],
        requirements: &[n::TraitRequirement],
    ) -> Result {
        self.width(predicates.len())?;
        self.width(requirements.len())?;
        for predicate in predicates {
            self.node(Node::Predicate(predicate))?;
        }
        for requirement in requirements {
            self.node(Node::Requirement(requirement))?;
        }
        Ok(())
    }
    fn head(&mut self, head: &crate::normal_forms::Head) -> Result {
        let crate::normal_forms::Head {
            declaration: _,
            collector: _,
            role: _,
            parameters,
            requirements,
        } = head;
        self.normal_form_payload(
            parameters.iter().map(|parameter| {
                let crate::normal_forms::Parameter {
                    declaration: _,
                    binding: _,
                    kind: _,
                    ty,
                } = parameter;
                ty
            }),
            requirements,
            &[],
        )
    }
    fn normal_form_payload<'a>(
        &mut self,
        domains: impl IntoIterator<Item = &'a Term>,
        predicates: &[n::TypePredicate],
        capabilities: &[n::TraitRequirement],
    ) -> Result {
        for domain in domains {
            self.width(1)?;
            self.node(Node::Type(domain))?;
        }
        // Assembled heads wrap each capability in a Trait predicate. Account
        // for that additional node before cloning the requirement payload.
        self.width(capabilities.len())?;
        self.contracts(predicates, capabilities)
    }
    fn structure(&mut self, definition: &n::Struct) -> Result {
        let n::Struct {
            parameter_types,
            capabilities,
            requirements,
            type_parameters,
            declaration: _,
            name,
            fields,
        } = definition;
        self.width(name.len())?;
        self.width(type_parameters.len())?;
        self.terms(parameter_types)?;
        self.contracts(requirements, capabilities)?;
        self.width(fields.len())?;
        for (name, ty) in fields {
            self.width(name.len())?;
            self.node(Node::Type(ty))?;
        }
        Ok(())
    }
    fn newtype(&mut self, definition: &crate::newtypes::Newtype) -> Result {
        let crate::newtypes::Newtype {
            declaration: _,
            name,
            type_parameters,
            parameter_types,
            capture_count: _,
            requirements,
            capabilities,
            representation,
        } = definition;
        self.width(name.len())?;
        self.width(type_parameters.len())?;
        self.terms(parameter_types)?;
        self.contracts(requirements, capabilities)?;
        self.node(Node::Type(representation))?;
        Ok(())
    }
    fn implementation(&mut self, implementation: &n::Implementation) -> Result {
        let n::Implementation {
            phases: _,
            parameter_types,
            associated,
            associated_initializers,
            interface_arguments,
            capabilities,
            type_parameters,
            requirements,
            declaration: _,
            subject,
            interface: _,
            methods,
        } = implementation;
        self.width(type_parameters.len())?;
        self.terms(parameter_types)?;
        self.terms(interface_arguments)?;
        self.node(Node::Type(subject))?;
        self.contracts(requirements, capabilities)?;
        self.width(associated.len())?;
        for (_, value) in associated {
            self.node(Node::Expr(value))?;
        }
        self.width(associated_initializers.len())?;
        for (_, initializer) in associated_initializers {
            self.node(Node::Expr(initializer.expression()))?;
        }
        self.width(methods.len())?;
        for (name, _) in methods {
            self.width(name.len())?;
        }
        Ok(())
    }
    fn program(&mut self, program: &crate::contracts::ProgramContract) -> Result {
        let crate::contracts::ProgramContract {
            preconditions,
            postconditions,
            result_binding: _,
            entry_snapshots,
        } = program;
        self.width(entry_snapshots.len())?;
        self.width(preconditions.len())?;
        self.width(postconditions.len())?;
        for expr in preconditions.iter().chain(postconditions) {
            self.node(Node::Expr(expr))?;
        }
        Ok(())
    }
    fn cleanup(&mut self, access: &n::RuntimeCleanupAccess) -> Result {
        let n::RuntimeCleanupAccess {
            parameter: _,
            input_fields,
            through,
            fields,
            write: _,
            read_value: _,
            ty,
            conditions,
        } = access;
        self.width(input_fields.len())?;
        self.width(through.len())?;
        for path in through {
            self.width(path.len())?;
        }
        self.width(fields.len())?;
        self.node(Node::Type(ty))?;
        self.width(conditions.len())?;
        for guards in conditions.values() {
            self.width(guards.len())?;
            for (path, _) in guards {
                self.width(path.len())?;
            }
        }
        Ok(())
    }
    fn interface(&mut self, interface: &n::Trait) -> Result {
        let n::Trait {
            comptime_only: _,
            parameter_types,
            associated,
            requirements,
            capabilities,
            type_parameters,
            declaration: _,
            parents,
            self_parameter: _,
            name,
            methods,
        } = interface;
        self.width(name.len())?;
        self.width(type_parameters.len())?;
        self.terms(parameter_types)?;
        self.contracts(requirements, capabilities)?;
        self.width(parents.len())?;
        for parent in parents {
            self.terms(&parent.arguments)?;
        }
        self.width(associated.len())?;
        for member in associated {
            let n::AssociatedContract {
                phase: _,
                declaration: _,
                name,
                ty,
                default,
            } = member;
            self.width(name.len())?;
            self.node(Node::Type(ty))?;
            if let Some(expr) = default {
                self.node(Node::Expr(expr))?;
            }
        }
        self.width(methods.len())?;
        for method in methods {
            let n::MethodContract {
                phase: _,
                is_quote: _,
                pure: _,
                is_unsafe: _,
                type_arguments,
                type_parameters,
                capabilities,
                program,
                parameter_bindings,
                by_value_receiver: _,
                by_reference_receiver: _,
                declaration: _,
                name,
                signature,
                requirements,
                default: _,
            } = method;
            self.width(name.len())?;
            self.width(type_parameters.len())?;
            self.width(parameter_bindings.len())?;
            self.terms(type_arguments)?;
            self.contracts(requirements, capabilities)?;
            self.program(program)?;
            match signature {
                code::MethodSignature::Callable(ty) => self.node(Node::Type(ty))?,
                code::MethodSignature::Generator(signature) => {
                    self.code_inputs(signature)?;
                    self.expression_code(&signature.output)?;
                }
                code::MethodSignature::PatternGenerator(signature) => {
                    self.code_inputs(signature)?;
                    self.pattern_code(&signature.output)?;
                }
                code::MethodSignature::DataConsumer(signature) => {
                    self.code_inputs(signature)?;
                    self.node(Node::Type(&signature.output))?;
                }
            }
        }
        Ok(())
    }
    fn code_inputs<O>(&mut self, signature: &code::CodeFunctionSignature<O>) -> Result {
        let code::CodeFunctionSignature {
            clause_domains,
            input_binders,
            inputs,
            output: _,
        } = signature;
        self.terms(clause_domains)?;
        self.width(input_binders.len())?;
        self.width(inputs.len())?;
        for (_, input) in inputs {
            match input {
                code::CodeInput::Data(ty) => self.node(Node::Type(ty))?,
                code::CodeInput::Code(code) => self.expression_code(code)?,
                code::CodeInput::Pattern(code) => self.pattern_code(code)?,
            }
        }
        Ok(())
    }
    fn expression_code(&mut self, code: &code::ExpressionCodeInterface) -> Result {
        let code::ExpressionCodeInterface {
            infer_context: _,
            context,
            ports,
            defaults,
            result,
        } = code;
        self.width(context.len())?;
        for ty in context.values() {
            self.node(Node::Type(ty))?;
        }
        self.width(ports.len())?;
        for name in ports.keys() {
            self.width(name.len())?;
        }
        self.width(defaults.len())?;
        self.node(Node::Type(result))
    }
    fn pattern_code(&mut self, code: &code::PatternCodeInterface) -> Result {
        let code::PatternCodeInterface { subject, outputs } = code;
        self.node(Node::Type(subject))?;
        self.width(outputs.len())?;
        for (name, ty) in outputs {
            self.width(name.len())?;
            self.node(Node::Type(ty))?;
        }
        Ok(())
    }
    fn enumeration(&mut self, definition: &n::Enum) -> Result {
        let n::Enum {
            is_intrinsic: _,
            parameter_types,
            capabilities,
            requirements,
            type_parameters,
            declaration: _,
            name,
            variants,
            groups,
            encoding,
            symbol_encoding,
        } = definition;
        self.width(name.len())?;
        self.width(type_parameters.len())?;
        self.terms(parameter_types)?;
        self.contracts(requirements, capabilities)?;
        self.width(groups.len())?;
        for group in groups {
            self.width(group.len())?;
        }
        self.width(variants.len())?;
        for variant in variants {
            let n::Variant {
                name,
                shape,
                fields,
            } = variant;
            self.width(name.len())?;
            match shape {
                n::VariantShape::Unit | n::VariantShape::Tuple => {}
                n::VariantShape::Named(names) => {
                    self.width(names.len())?;
                    for name in names {
                        self.width(name.len())?;
                    }
                }
            }
            self.terms(fields)?;
        }
        if let Some(encoding) = encoding {
            self.width(encoding.variants().len())?;
            for variant in encoding.variants() {
                self.width(variant.name().len())?;
                self.width(variant.fixed_mask().words().len())?;
                self.width(variant.fixed_value().words().len())?;
                self.width(variant.fields().len())?;
                for field in variant.fields() {
                    self.width(field.name.len())?;
                }
            }
        }
        if let Some(encoding) = symbol_encoding {
            self.width(encoding.variants().len())?;
            for variant in encoding.variants() {
                self.width(variant.name.len())?;
                self.width(variant.value.len())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
