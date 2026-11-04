//! Scope a checked comptime world and synchronize its execution allowance.
use crate::{Budget, Error, ErrorKind, Interpreter};
use nir::program::{Package, TraitRequirement};

/// Reserve top-level world traversal and indexing before allocating VM tables.
/// Nested type, contract and substitution work keeps its separate existing cost.
pub(super) fn charge_world_tables(package: &Package, budget: &mut Budget) -> Result<(), Error> {
    let Package {
        builtin_implementation_packages,
        target: _,
        tests,
        function_parameters,
        runtime_instances,
        quote_templates,
        quote_identity_cursor: _,
        definition_identity_cursor: _,
        internal_return_packets,
        internal_return_empty_replacements,
        expansion_sources,
        effects,
        runtime_function_identities,
        normal_forms,
        dispatch_instances,
        traits,
        implementations,
        functions,
        structs,
        newtypes,
        alias_contracts,
        enums,
    } = package;
    let cost = [
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
        dispatch_instances.len(),
        traits.len(),
        implementations.len(),
        functions.len(),
        structs.len(),
        newtypes.len(),
        alias_contracts.len(),
        enums.len(),
    ]
    .into_iter()
    .try_fold(1u64, |sum, count| {
        sum.checked_add(u64::try_from(count).ok()?)
    });
    budget.steps = cost
        .and_then(|cost| budget.steps.checked_sub(cost))
        .ok_or(Error {
            kind: ErrorKind::StepsExhausted,
            source: None,
        })?;
    Ok(())
}

/// Fresh worlds validate opaque declaration metadata using construction fuel.
/// Resource exhaustion is not malformed metadata, even when fuel remains after
/// a failed width reservation. The caller separately establishes phase evidence.
pub(super) fn validate_opaque_metadata(
    package: &Package,
    budget: &mut Budget,
) -> Result<(), Error> {
    let before = usize::try_from(budget.steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = nir::normal_forms::validate_in(package, &mut fuel);
    budget.steps -= (before - fuel) as u64;
    result.map_err(|failure| {
        use nir::normal_forms::ValidationError;
        Error {
            kind: match failure {
                ValidationError::Budget
                | ValidationError::Newtype(nir::newtypes::ValidationError::Budget) => {
                    ErrorKind::StepsExhausted
                }
                ValidationError::Invalid(_)
                | ValidationError::Newtype(nir::newtypes::ValidationError::Invalid(_)) => {
                    ErrorKind::InvalidCallPlan
                }
            },
            source: None,
        }
    })
}

struct Scope<'program, 'budget> {
    interpreter: Interpreter<'program>,
    budget: &'budget mut Budget,
}
impl Drop for Scope<'_, '_> {
    fn drop(&mut self) {
        // Keep the construction reservation even if host code replaced the VM
        // with another one. A scope must never replenish its caller's allowance.
        self.budget.steps = self.budget.steps.min(self.interpreter.remaining_steps());
    }
}

impl<'program> Interpreter<'program> {
    /// Prepare a fresh checked world, execute a host operation, and synchronize
    /// remaining steps on success, error, or unwinding. Construction failure
    /// retains the conservative reservation of for_comptime_in_budget.
    ///
    /// Consecutive scopes can use newly published packages without carrying
    /// function instances or evidence caches from the old world. Inputs and
    /// returned values retain ordinary invocation admission. This does not resume
    /// an active call stack or transfer its local storage/cleanup responsibilities.
    pub fn with_comptime_in_budget<T>(
        package: &'program Package,
        assumptions: &[TraitRequirement],
        budget: &mut Budget,
        operation: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let interpreter = Self::for_comptime_in_budget(package, assumptions, budget)?;
        let mut scope = Scope {
            interpreter,
            budget,
        };
        operation(&mut scope.interpreter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::{DeclarationId, Primitive, Term};

    #[test]
    fn concrete_trait_input_admission_keeps_budget_failures_distinct() {
        let id = |local| DeclarationId { package: 76, local };
        let mut package = Package {
            traits: vec![nir::program::Trait {
                declaration: id(1),
                self_parameter: id(2),
                name: "Indexed".into(),
                comptime_only: false,
                type_parameters: vec![id(3)],
                parameter_types: vec![Term::Primitive(Primitive::Bool)],
                associated: vec![],
                requirements: vec![],
                capabilities: vec![],
                parents: vec![],
                methods: vec![],
            }],
            implementations: vec![nir::program::Implementation {
                declaration: id(4),
                interface: Some(id(1)),
                subject: Term::Primitive(Primitive::Bool),
                interface_arguments: vec![Term::Bool(true)],
                phases: nir::program::ImplementationPhases {
                    runtime: true,
                    comptime: false,
                },
                type_parameters: vec![],
                parameter_types: vec![],
                associated: vec![],
                associated_initializers: vec![],
                capabilities: vec![],
                requirements: vec![],
                methods: vec![],
            }],
            ..Default::default()
        };
        let before = package.to_string();
        let vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10_000,
                depth: 128,
            },
        )
        .unwrap();
        let cost = 10_000 - vm.remaining_steps();
        for steps in 0..cost {
            assert_eq!(
                Interpreter::for_comptime(&package, Budget { steps, depth: 128 })
                    .err()
                    .expect("insufficient fuel")
                    .kind,
                ErrorKind::StepsExhausted,
                "steps {steps}"
            );
        }
        assert_eq!(
            Interpreter::for_comptime(
                &package,
                Budget {
                    steps: cost,
                    depth: 128
                }
            )
            .unwrap()
            .remaining_steps(),
            0
        );
        assert_eq!(package.to_string(), before);
        package.implementations[0].interface_arguments[0] = Term::Char('x');
        assert_eq!(
            Interpreter::new(&package, Budget::default())
                .err()
                .unwrap()
                .kind,
            ErrorKind::InvalidCallPlan
        );
        package.implementations[0].interface_arguments[0] = Term::Bool(false);
        assert!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .events()
                .is_empty()
        );
    }

    fn opaque_package() -> Package {
        Package {
            normal_forms: vec![nir::normal_forms::Head {
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                role: Default::default(),
                collector: None,
                parameters: vec![],
                requirements: vec![nir::program::TypePredicate::Bool(true); 100],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn opaque_metadata_reservations_keep_resource_errors_and_successful_costs() {
        let mut package = opaque_package();
        let mut short = Budget {
            steps: 5,
            depth: 128,
        };
        assert_eq!(
            validate_opaque_metadata(&package, &mut short)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(short.steps > 0 && short.steps < 5);
        assert_eq!(short.depth, 128);
        let mut generous = Budget {
            steps: 10_000,
            depth: 128,
        };
        validate_opaque_metadata(&package, &mut generous).unwrap();
        let cost = 10_000 - generous.steps;
        let mut exact = Budget {
            steps: cost,
            depth: 128,
        };
        validate_opaque_metadata(&package, &mut exact).unwrap();
        assert_eq!(exact.steps, 0);
        package.normal_forms[0].collector = Some(0);
        assert_eq!(
            validate_opaque_metadata(&package, &mut generous)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidCallPlan
        );
        package.normal_forms[0].collector = None;
        validate_opaque_metadata(&package, &mut generous).unwrap();

        let package = Package {
            newtypes: vec![nir::newtypes::Newtype {
                declaration: DeclarationId {
                    package: 1,
                    local: 2,
                },
                name: "Wide".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: Term::Tuple(vec![Term::Primitive(Primitive::Bool); 100]),
            }],
            ..Default::default()
        };
        let mut short = Budget {
            steps: 10,
            depth: 128,
        };
        assert_eq!(
            validate_opaque_metadata(&package, &mut short)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        validate_opaque_metadata(&package, &mut generous).unwrap();
    }

    #[test]
    fn fresh_constructors_charge_opaque_metadata_and_do_not_publish_failures() {
        let package = opaque_package();
        let before = package.to_string();
        let vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10_000,
                depth: 128,
            },
        )
        .unwrap();
        let cost = 10_000 - vm.remaining_steps();
        assert!(cost > 100);
        for steps in 0..cost {
            for result in [
                Interpreter::new(&package, Budget { steps, depth: 128 }),
                Interpreter::for_comptime(&package, Budget { steps, depth: 128 }),
            ] {
                assert_eq!(
                    result.err().expect("insufficient constructor fuel").kind,
                    ErrorKind::StepsExhausted,
                    "steps {steps}"
                );
            }
        }
        let vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: cost,
                depth: 128,
            },
        )
        .unwrap();
        assert_eq!(vm.remaining_steps(), 0);
        assert!(vm.events().is_empty());
        assert_eq!(package.to_string(), before);
    }

    #[test]
    fn declaration_domain_budget_failure_is_not_an_invalid_call_plan() {
        let parameter = nir::function_parameters::Declaration {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            parameters: vec![DeclarationId {
                package: 1,
                local: 2,
            }],
            domains: vec![Term::Optional(Box::new(Term::Primitive(Primitive::Bool)))],
        };
        let mut package = Package {
            function_parameters: vec![parameter.clone()],
            ..Default::default()
        };
        let mut succeeded = false;
        for steps in 0..256 {
            match Interpreter::for_comptime(&package, Budget { steps, depth: 128 }) {
                Ok(_) => {
                    succeeded = true;
                    break;
                }
                Err(error) => assert_eq!(error.kind, ErrorKind::StepsExhausted, "budget {steps}"),
            }
        }
        assert!(
            succeeded,
            "valid metadata must succeed with sufficient fuel"
        );
        package.function_parameters.push(parameter);
        assert_eq!(
            Interpreter::new(&package, Budget::default())
                .err()
                .unwrap()
                .kind,
            ErrorKind::InvalidCallPlan
        );
    }
}
