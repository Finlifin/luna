//! Immutable declaration evidence shared inside one package analysis. This
//! contains no call-site assumptions, cleanup choices or behavior certificates.
use nir::{
    DeclarationId, Primitive, Sort, Term,
    program::{Function, FunctionPhase, Package},
    type_computations::Error,
};
use std::{borrow::Cow, cell::OnceCell, collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy)]
enum SourceFunctionSlot {
    Function(usize),
    QuoteTemplate(usize),
    Ambiguous,
}

pub(crate) fn is_budget_error(error: &str) -> bool {
    error.ends_with("budget exhausted")
}

pub(crate) struct Metadata<'a> {
    package: Option<&'a Package>,
    domains: OnceCell<Arc<BTreeMap<DeclarationId, Term>>>,
    sorts: OnceCell<Arc<BTreeMap<DeclarationId, Sort>>>,
    function_index: OnceCell<Arc<BTreeMap<DeclarationId, usize>>>,
    source_function_index: OnceCell<Arc<BTreeMap<DeclarationId, SourceFunctionSlot>>>,
    runtime_instance_index: OnceCell<Arc<BTreeMap<DeclarationId, Vec<usize>>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::{Primitive, Term, type_aliases::Contract};

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 7, local }
    }

    #[test]
    fn preparation_is_bounded_retryable_and_package_local() {
        let mut package = Package::default();
        package.alias_contracts = (0..256)
            .map(|local| Contract {
                declaration: id(local),
                parameters: vec![id(local + 1000)],
                parameter_types: vec![Term::Primitive(Primitive::Type)],
                requirements: vec![],
                capabilities: vec![],
            })
            .collect();
        let metadata = std::rc::Rc::new(Metadata::new(Some(&package)));
        let mut fuel = 32;
        assert!(metadata.parameter_domains(&[id(1000)], &mut fuel).is_err());
        assert!(fuel < 32);
        assert!(!metadata.domains_prepared());
        let domains = metadata
            .parameter_domains(&[id(1000)], &mut 10_000)
            .unwrap();
        assert_eq!(domains[0], Term::Primitive(Primitive::Type));
        let consumer = metadata.clone();
        let mut fuel = 3;
        assert_eq!(
            consumer.parameter_domains(&[id(1000)], &mut fuel).unwrap(),
            domains
        );
        assert_eq!(fuel, 0);
        assert!(consumer.parameter_domains(&[id(1000)], &mut 0).is_err());
        assert!(
            Metadata::new(Some(&package))
                .parameter_domains(&[id(1000)], &mut 32)
                .is_err()
        );

        let mut other = package.clone();
        other.alias_contracts[0].parameter_types[0] = Term::Primitive(Primitive::Bool);
        assert_eq!(
            Metadata::new(Some(&other))
                .parameter_domains(&[id(1000)], &mut 10_000)
                .unwrap()[0],
            Term::Primitive(Primitive::Bool)
        );
        assert_eq!(
            metadata.parameter_domains(&[id(1000)], &mut 3).unwrap()[0],
            Term::Primitive(Primitive::Type)
        );

        other.alias_contracts[0].parameters.push(id(1000));
        other.alias_contracts[0]
            .parameter_types
            .push(Term::Primitive(Primitive::Bool));
        let invalid = Metadata::new(Some(&other));
        for _ in 0..2 {
            assert!(invalid.parameter_domains(&[id(1000)], &mut 10_000).is_err());
            assert!(!invalid.domains_prepared());
        }
    }

    #[test]
    fn warm_directory_still_charges_selected_domain_payloads() {
        let domain = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 256]);
        let package = Package {
            alias_contracts: vec![Contract {
                declaration: id(1),
                parameters: vec![id(2)],
                parameter_types: vec![domain.clone()],
                requirements: vec![],
                capabilities: vec![],
            }],
            ..Default::default()
        };
        let metadata = Metadata::new(Some(&package));
        metadata.domains(&mut 10_000).unwrap();
        assert_eq!(
            metadata.parameter_domains(&[id(2)], &mut 8),
            Err(Error::Budget)
        );
        let mut fuel = 10_000;
        assert_eq!(
            metadata.parameter_domains(&[id(2)], &mut fuel).unwrap(),
            vec![domain]
        );
        assert!(10_000 - fuel >= 256);
        assert_eq!(
            metadata.sorts(&mut 10_000).unwrap()[&id(2)],
            Sort::ComptimeValue
        );
    }

    #[test]
    fn runtime_instance_index_is_bounded_reusable_and_checks_validity() {
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: boolean.clone(),
            }],
            result: Box::new(boolean.clone()),
        };
        let make_instance = |local: u32, arg: Term| nir::program::RuntimeInstance {
            declaration: id(local),
            arguments: vec![arg],
            function: Function {
                interface: nir::program::FunctionDeclaration {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    phase: FunctionPhase::Runtime,
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: vec![],
                    declaration: id(local),
                    name: format!("f_{local}"),
                    parameters: vec![nir::program::LocalId {
                        owner: local,
                        index: 0,
                    }],
                    env_parameters: vec![],
                    defaults: vec![],
                    signature: signature.clone(),
                },
                body: nir::program::Expr {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    ty: boolean.clone(),
                    kind: nir::program::ExprKind::Literal(nir::program::Literal::Bool(true)),
                },
                runtime_deliveries: Default::default(),
            },
            cleanup_accesses: vec![],
        };
        let mut package = Package::default();
        for index in 0..32u32 {
            package.runtime_instances.push(make_instance(
                index + 1,
                Term::Tuple(vec![boolean.clone(); index as usize]),
            ));
        }
        package
            .runtime_instances
            .push(make_instance(1, Term::Tuple(vec![boolean.clone(); 40])));
        let metadata = Metadata::new(Some(&package));
        let target_arg = [Term::Tuple(vec![])];
        assert_eq!(
            metadata.runtime_instance_body_checked(id(1), &target_arg, &mut 8),
            Err("runtime instance index budget exhausted")
        );
        assert!(metadata.runtime_instance_index.get().is_none());

        let mut fuel = 10_000;
        let found = metadata
            .runtime_instance_body_checked(id(1), &target_arg, &mut fuel)
            .unwrap()
            .unwrap();
        assert_eq!(found.declaration, id(1));

        // Warm index lookup uses per-declaration bucket size rather than the full table.
        let mut warm_fuel = 3;
        let found_again = metadata
            .runtime_instance_body_checked(id(1), &target_arg, &mut warm_fuel)
            .unwrap()
            .unwrap();
        assert_eq!(found_again.declaration, id(1));
        assert_eq!(warm_fuel, 0);
        assert_eq!(
            metadata.runtime_instance_body_checked(id(1), &target_arg, &mut 2),
            Err("runtime instance lookup budget exhausted")
        );
        assert_eq!(
            metadata
                .runtime_instance_body_checked(id(999), &[], &mut 1)
                .unwrap(),
            None
        );

        let mut duplicate = package.clone();
        duplicate
            .runtime_instances
            .push(make_instance(2, Term::Tuple(vec![boolean.clone(); 1])));
        let dup_meta = Metadata::new(Some(&duplicate));
        assert_eq!(
            dup_meta.runtime_instance_body_checked(
                id(2),
                &[Term::Tuple(vec![boolean.clone(); 1])],
                &mut 10_000,
            ),
            Err("runtime function instance is ambiguous or malformed")
        );

        let mut malformed = package;
        malformed.runtime_instances[2].function.phase = FunctionPhase::Comptime;
        let bad_meta = Metadata::new(Some(&malformed));
        assert_eq!(
            bad_meta.runtime_instance_body_checked(
                id(3),
                &[Term::Tuple(vec![boolean; 2])],
                &mut 10_000,
            ),
            Err("runtime function instance is ambiguous or malformed")
        );
    }

    #[test]
    fn runtime_body_index_reuses_source_lookup_and_preserves_ambiguity_and_arity() {
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: boolean.clone(),
            }],
            result: Box::new(boolean.clone()),
        };
        let make_function =
            |local: u32, name: &str, type_parameters: Vec<DeclarationId>| Function {
                interface: nir::program::FunctionDeclaration {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    phase: FunctionPhase::Runtime,
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters,
                    declaration: id(local),
                    name: name.to_string(),
                    parameters: vec![nir::program::LocalId {
                        owner: local,
                        index: 0,
                    }],
                    env_parameters: vec![],
                    defaults: vec![],
                    signature: signature.clone(),
                },
                body: nir::program::Expr {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    ty: boolean.clone(),
                    kind: nir::program::ExprKind::Literal(nir::program::Literal::Bool(true)),
                },
                runtime_deliveries: Default::default(),
            };
        let mut package = Package::default();
        for local in 1..=64u32 {
            package.functions.push(make_function(
                local,
                &format!("f_{local}"),
                if local == 50 { vec![id(900)] } else { vec![] },
            ));
        }
        package
            .quote_templates
            .push(make_function(65, "quoted_template", vec![]));
        // Duplicate declaration id across functions and quote_templates.
        package
            .quote_templates
            .push(make_function(10, "duplicate_ten", vec![]));
        package
            .runtime_instances
            .push(nir::program::RuntimeInstance {
                declaration: id(50),
                arguments: vec![boolean.clone()],
                function: make_function(50, "f_50_inst", vec![]),
                cleanup_accesses: vec![],
            });

        let metadata = Metadata::new(Some(&package));
        // Exhausting during cold index build must not cache a partial index.
        assert_eq!(
            metadata.runtime_body_checked(id(1), &[], &mut 4),
            Err("source function index budget exhausted")
        );
        assert!(metadata.source_function_index.get().is_none());

        let mut cold_fuel = 10_000;
        let (found, args) = metadata
            .runtime_body_checked(id(65), &[], &mut cold_fuel)
            .unwrap();
        assert_eq!(found.name, "quoted_template");
        assert!(args.is_empty());
        assert!(metadata.source_function_index.get().is_some());

        // Subsequent lookups spend O(1) fuel regardless of package size.
        let mut warm_fuel = 3;
        let (found_warm, _) = metadata
            .runtime_body_checked(id(42), &[], &mut warm_fuel)
            .unwrap();
        assert_eq!(found_warm.name, "f_42");
        assert!(warm_fuel >= 1);

        // Runtime instances take precedence and yield empty type arguments.
        let (inst, inst_args) = metadata
            .runtime_body_checked(id(50), std::slice::from_ref(&boolean), &mut 10)
            .unwrap();
        assert_eq!(inst.name, "f_50_inst");
        assert!(inst_args.is_empty());

        // Ambiguity stays scoped to the duplicated declaration id.
        assert_eq!(
            metadata
                .runtime_body_checked(id(10), &[], &mut 10)
                .unwrap_err(),
            "function declaration is ambiguous"
        );
        assert_eq!(
            metadata
                .runtime_body_checked(id(999), &[], &mut 10)
                .unwrap_err(),
            "unknown function"
        );
        assert_eq!(
            metadata
                .runtime_body_checked(id(50), &[boolean.clone(), boolean], &mut 10)
                .unwrap_err(),
            "function type arguments have incorrect arity"
        );
    }
}

impl<'a> Metadata<'a> {
    pub(crate) fn new(package: Option<&'a Package>) -> Self {
        Self {
            package,
            domains: OnceCell::new(),
            sorts: OnceCell::new(),
            function_index: OnceCell::new(),
            source_function_index: OnceCell::new(),
            runtime_instance_index: OnceCell::new(),
        }
    }

    pub(crate) fn package(&self) -> Option<&'a Package> {
        self.package
    }

    pub(crate) fn function_index(
        &self,
        fuel: &mut usize,
    ) -> Result<Option<Arc<BTreeMap<DeclarationId, usize>>>, &'static str> {
        let Some(package) = self.package else {
            return Ok(None);
        };
        if let Some(index) = self.function_index.get() {
            return Ok(Some(Arc::clone(index)));
        }
        let cost = package.functions.len().max(1);
        *fuel = fuel
            .checked_sub(cost)
            .ok_or("rejection summary budget exhausted")?;
        let mut definitions = BTreeMap::new();
        for (slot, function) in package.functions.iter().enumerate() {
            if definitions.insert(function.declaration, slot).is_some() {
                return Err("duplicate function identity in rejection summary");
            }
        }
        let index = Arc::new(definitions);
        let _ = self.function_index.set(Arc::clone(&index));
        Ok(Some(index))
    }

    pub(crate) fn runtime_instance_body_checked(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Option<&'a Function>, &'static str> {
        let Some(package) = self.package else {
            return Ok(None);
        };
        let index = if let Some(index) = self.runtime_instance_index.get() {
            Arc::clone(index)
        } else {
            let cost = package.runtime_instances.len().max(1);
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("runtime instance index budget exhausted")?;
            let mut grouped: BTreeMap<DeclarationId, Vec<usize>> = BTreeMap::new();
            for (slot, instance) in package.runtime_instances.iter().enumerate() {
                grouped.entry(instance.declaration).or_default().push(slot);
            }
            let index = Arc::new(grouped);
            let _ = self.runtime_instance_index.set(Arc::clone(&index));
            index
        };
        *fuel = fuel
            .checked_sub(1)
            .ok_or("runtime instance lookup budget exhausted")?;
        let Some(slots) = index.get(&declaration) else {
            return Ok(None);
        };
        *fuel = fuel
            .checked_sub(slots.len().max(1))
            .ok_or("runtime instance lookup budget exhausted")?;
        let mut matches = slots
            .iter()
            .map(|&slot| &package.runtime_instances[slot])
            .filter(|instance| instance.arguments == arguments);
        if let Some(instance) = matches.next() {
            if matches.next().is_some()
                || instance.function.declaration != declaration
                || !instance.function.type_parameters.is_empty()
                || instance.function.phase != FunctionPhase::Runtime
                || !matches!(&instance.function.signature, Term::Function { parameters, .. }
                    if parameters.len() == instance.function.parameters.len()
                        && parameters.iter().all(|p| p.sort == Sort::Runtime))
            {
                return Err("runtime function instance is ambiguous or malformed");
            }
            return Ok(Some(&instance.function));
        }
        Ok(None)
    }

    pub(crate) fn runtime_body_checked<'b>(
        &self,
        declaration: DeclarationId,
        arguments: &'b [Term],
        fuel: &mut usize,
    ) -> Result<(&'a Function, &'b [Term]), &'static str> {
        let Some(package) = self.package else {
            return Err("unknown function");
        };
        if let Some(instance) = self.runtime_instance_body_checked(declaration, arguments, fuel)? {
            return Ok((instance, &[]));
        }
        let index = if let Some(index) = self.source_function_index.get() {
            Arc::clone(index)
        } else {
            let cost = package
                .functions
                .len()
                .saturating_add(package.quote_templates.len())
                .max(1);
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("source function index budget exhausted")?;
            let mut definitions: BTreeMap<DeclarationId, SourceFunctionSlot> = BTreeMap::new();
            for (slot, function) in package.functions.iter().enumerate() {
                definitions
                    .entry(function.declaration)
                    .and_modify(|existing| *existing = SourceFunctionSlot::Ambiguous)
                    .or_insert(SourceFunctionSlot::Function(slot));
            }
            for (slot, function) in package.quote_templates.iter().enumerate() {
                definitions
                    .entry(function.declaration)
                    .and_modify(|existing| *existing = SourceFunctionSlot::Ambiguous)
                    .or_insert(SourceFunctionSlot::QuoteTemplate(slot));
            }
            let index = Arc::new(definitions);
            let _ = self.source_function_index.set(Arc::clone(&index));
            index
        };
        *fuel = fuel
            .checked_sub(1)
            .ok_or("source function lookup budget exhausted")?;
        let function = match index.get(&declaration).copied() {
            Some(SourceFunctionSlot::Function(slot)) => &package.functions[slot],
            Some(SourceFunctionSlot::QuoteTemplate(slot)) => &package.quote_templates[slot],
            Some(SourceFunctionSlot::Ambiguous) => return Err("function declaration is ambiguous"),
            None => return Err("unknown function"),
        };
        if function.type_parameters.len() != arguments.len() {
            return Err("function type arguments have incorrect arity");
        }
        Ok((function, arguments))
    }

    /// A residual body without declaration inputs needs no copy or rewrite.
    /// Borrow it from its owner unless an uncanonical error row still needs
    /// instantiation normalization; consumers still pay for their analysis.
    /// This is preparation of checked code, not a replacement for NIR
    /// validation.
    pub(crate) fn specialize_function<'f>(
        &self,
        function: &'f nir::program::Function,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Cow<'f, nir::program::Function>, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if function.type_parameters.is_empty()
            && arguments.is_empty()
            && !term_has_noncanonical_error_row(&function.signature, fuel)?
            && !term_has_noncanonical_error_row(&function.body.ty, fuel)?
        {
            return Ok(Cow::Borrowed(function));
        }
        let domains = self.parameter_domains(&function.type_parameters, fuel)?;
        function
            .instantiate_declaration_parameters(&domains, arguments, fuel)
            .map(Cow::Owned)
    }

    pub(crate) fn domains(
        &self,
        fuel: &mut usize,
    ) -> Result<&Arc<BTreeMap<DeclarationId, Term>>, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if self.domains.get().is_none() {
            let domains = match self.package {
                Some(package) => package.declaration_parameter_domains_checked(fuel)?,
                None => BTreeMap::new(),
            };
            let _ = self.domains.set(Arc::new(domains));
        }
        Ok(self.domains.get().expect("complete declaration domains"))
    }

    pub(crate) fn parameter_domains(
        &self,
        parameters: &[DeclarationId],
        fuel: &mut usize,
    ) -> Result<Vec<Term>, Error> {
        *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
        if parameters.is_empty() {
            return Ok(vec![]);
        }
        let domains = self.domains(fuel)?;
        parameters
            .iter()
            .map(|id| {
                let domain = domains.get(id).unwrap_or(&Term::Primitive(Primitive::Type));
                nir::type_computations::charge_term(domain, fuel)?;
                Ok(domain.clone())
            })
            .collect()
    }

    pub(crate) fn sorts(
        &self,
        fuel: &mut usize,
    ) -> Result<&Arc<BTreeMap<DeclarationId, Sort>>, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if self.sorts.get().is_none() {
            let domains = self.domains(fuel)?;
            *fuel = fuel.checked_sub(domains.len()).ok_or(Error::Budget)?;
            let sorts = domains
                .iter()
                .map(|(id, domain)| (*id, Sort::of_comptime_domain(domain)))
                .collect();
            // Publish only a complete directory. A failed attempt remains
            // retryable and never refunds the work already performed.
            let _ = self.sorts.set(Arc::new(sorts));
        }
        Ok(self.sorts.get().expect("complete declaration directory"))
    }

    pub(crate) fn domains_prepared(&self) -> bool {
        self.domains.get().is_some()
    }
}

fn term_has_noncanonical_error_row(ty: &Term, fuel: &mut usize) -> Result<bool, Error> {
    let mut stack = vec![(ty, 0usize)];
    while let Some((term, depth)) = stack.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if depth >= 128 {
            return Ok(true);
        }
        if let Term::ErrorResult { errors, result } = term {
            if errors.is_empty() || matches!(result.as_ref(), Term::ErrorResult { .. }) {
                return Ok(true);
            }
            if errors.len() > 1 {
                let mut keys = errors.iter().map(ToString::to_string);
                let mut prev = keys.next().unwrap();
                for key in keys {
                    *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
                    if prev >= key {
                        return Ok(true);
                    }
                    prev = key;
                }
            }
        }
        term.visit_children::<Error>(|child| {
            stack.push((child, depth + 1));
            Ok(())
        })?;
    }
    Ok(false)
}
