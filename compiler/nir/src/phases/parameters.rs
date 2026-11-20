//! Shared parameter admission for one phase-validation pass. This owns no
//! allowance: every index, domain and default consumes the caller's fuel.
use crate::{
    Term,
    program::{Package, SourceRange},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Budget(super::Error),
    Invalid(super::Error),
}
impl Failure {
    pub(super) fn into_diagnostic(self) -> super::Error {
        match self {
            Self::Budget(error) | Self::Invalid(error) => error,
        }
    }
}

pub(super) fn domains(
    package: &Package,
    fuel: &mut usize,
) -> Result<BTreeMap<crate::DeclarationId, Term>, Failure> {
    let source = SourceRange { start: 0, end: 0 };
    let domains = package
        .declaration_parameter_domains_checked(fuel)
        .map_err(|error| match error {
            crate::type_computations::Error::Budget => Failure::Budget(super::Error {
                kind: super::ErrorKind::Budget,
                source,
                message: "declaration parameter domain analysis budget exhausted",
            }),
            _ => Failure::Invalid(super::Error {
                kind: super::ErrorKind::Invalid,
                source,
                message: "invalid declaration parameter domains",
            }),
        })?;
    Ok(domains)
}

/// Per-callable inspection data. It borrows the original telescope and owns
/// only the sort frame and immutable snapshot-binding membership evidence.
pub(super) struct InspectionFrame<'a> {
    pub(super) signature: Option<&'a Term>,
    pub(super) frames: Vec<Vec<crate::Sort>>,
    pub(super) frozen: BTreeSet<crate::program::LocalId>,
}
impl<'a> InspectionFrame<'a> {
    pub(super) fn new(
        function: crate::contracts::CallableProgram<'a>,
        fuel: &mut usize,
    ) -> Result<Self, Failure> {
        Self::for_parts(function.parts, Some(function.signature), fuel)
    }

    pub(super) fn for_parts(
        function: crate::contracts::ProgramParts<'a>,
        ordinary: Option<&'a Term>,
        fuel: &mut usize,
    ) -> Result<Self, Failure> {
        let budget = || {
            Failure::Budget(super::Error {
                kind: super::ErrorKind::Budget,
                source: function.source,
                message: "function inspection frame budget exhausted",
            })
        };
        let invalid = |message| {
            Failure::Invalid(super::Error {
                kind: super::ErrorKind::Invalid,
                source: function.source,
                message,
            })
        };
        // Follow only callable wrappers. This view does not clone or unfold
        // any type payload, captured environment or normal-form definition.
        let mut signature = ordinary;
        while let Some(current) = signature {
            *fuel = fuel.checked_sub(1).ok_or_else(budget)?;
            match current {
                Term::ComptimeFunction(inner)
                | Term::NormalFormFunction(inner, _)
                | Term::UnsafeFunction(inner)
                | Term::Closure {
                    signature: inner, ..
                }
                | Term::ContextualClosure {
                    signature: inner, ..
                } => signature = Some(inner),
                _ => break,
            }
        }
        let frames = if let Some(Term::Function { parameters, .. }) = signature {
            *fuel = fuel
                .checked_sub(parameters.len().saturating_add(1))
                .ok_or_else(budget)?;
            vec![parameters.iter().map(|parameter| parameter.sort).collect()]
        } else {
            vec![]
        };
        let mut frozen = BTreeSet::new();
        if function.result_binding.is_some() || !function.entry_snapshots.is_empty() {
            *fuel = fuel
                .checked_sub(function.parameters.len())
                .ok_or_else(budget)?;
            *fuel = fuel
                .checked_sub(function.entry_snapshots.len())
                .ok_or_else(budget)?;
            let inputs: BTreeSet<_> = function.parameters.iter().copied().collect();
            if function
                .result_binding
                .is_some_and(|binding| inputs.contains(&binding))
            {
                return Err(invalid("result binding aliases an input parameter"));
            }
            for snapshot in function.entry_snapshots {
                if !inputs.contains(&snapshot.parameter)
                    || inputs.contains(&snapshot.binding)
                    || function.result_binding == Some(snapshot.binding)
                    || !frozen.insert(snapshot.binding)
                {
                    return Err(invalid("invalid entry snapshot binding"));
                }
            }
        }
        Ok(Self {
            signature,
            frames,
            frozen,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DeclarationId, Parameter, Primitive, Sort,
        program::{Expr, ExprKind, Function, FunctionPhase, LocalId},
    };

    fn function(local: u32) -> Function {
        let ty = Term::Primitive(Primitive::Bool);
        Function {
            interface: crate::program::FunctionDeclaration {
                source: SourceRange { start: 5, end: 20 },
                declaration: DeclarationId { package: 1, local },
                name: "input".into(),
                phase: FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                signature: Term::Function {
                    parameters: vec![Parameter {
                        sort: Sort::Runtime,
                        ty: ty.clone(),
                    }],
                    result: Box::new(ty.clone()),
                },
                parameters: vec![LocalId {
                    owner: local,
                    index: 0,
                }],
                env_parameters: vec![],
                defaults: vec![(
                    0,
                    Expr {
                        source: SourceRange {
                            start: local,
                            end: local + 1,
                        },
                        ty: ty.clone(),
                        kind: ExprKind::Literal(crate::program::Literal::Bool(true)),
                    },
                )],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
            },
            body: Expr {
                source: SourceRange {
                    start: local,
                    end: local + 1,
                },
                ty,
                kind: ExprKind::Literal(crate::program::Literal::Bool(true)),
            },
            runtime_deliveries: Default::default(),
        }
    }

    #[test]
    fn inspection_borrows_the_telescope_and_preserves_sort_order() {
        let mut value = function(10);
        value.signature =
            Term::UnsafeFunction(Box::new(Term::ComptimeFunction(Box::new(Term::Function {
                parameters: [Sort::Type, Sort::Runtime, Sort::ComptimeValue]
                    .into_iter()
                    .map(|sort| Parameter {
                        sort,
                        ty: Term::Primitive(Primitive::Bool),
                    })
                    .collect(),
                result: Box::new(Term::Primitive(Primitive::Bool)),
            }))));
        let program = crate::contracts::CallableProgram::function(&value);
        let frame = InspectionFrame::new(program, &mut 100).unwrap();
        assert!(std::ptr::eq(
            frame.signature.unwrap(),
            value.signature.callable_signature()
        ));
        assert_eq!(
            frame.frames,
            vec![vec![Sort::Type, Sort::Runtime, Sort::ComptimeValue]]
        );
        assert!(frame.frozen.is_empty());
        // Wrapper inspection is paid even before any frame can be allocated.
        assert!(matches!(
            InspectionFrame::new(program, &mut 2),
            Err(Failure::Budget(_))
        ));
    }

    #[test]
    fn snapshot_membership_is_reserved_before_validation_and_retry_is_fresh() {
        let mut value = function(10);
        value.parameters = (0..64).map(|index| LocalId { owner: 10, index }).collect();
        let Term::Function { parameters, .. } = &mut value.signature else {
            unreachable!()
        };
        parameters.resize(64, parameters[0].clone());
        value.entry_snapshots = value
            .parameters
            .iter()
            .map(|parameter| crate::program::EntrySnapshot {
                parameter: *parameter,
                binding: LocalId {
                    owner: 10,
                    index: parameter.index + 64,
                },
            })
            .collect();
        let program = crate::contracts::CallableProgram::function(&value);
        let mut generous = 1000;
        let frame = InspectionFrame::new(program, &mut generous).unwrap();
        assert_eq!(frame.frozen.len(), 64);
        let cost = 1000 - generous;
        let mut exact = cost;
        InspectionFrame::new(program, &mut exact).unwrap();
        assert_eq!(exact, 0);
        let mut short = cost - 1;
        let Err(Failure::Budget(error)) = InspectionFrame::new(program, &mut short) else {
            panic!("snapshot membership must consume the caller's budget");
        };
        assert!(short > 0);
        assert_eq!(error.source, value.source);
        InspectionFrame::new(program, &mut cost.clone()).unwrap();

        // Invalid metadata cannot bypass the reservation or become a budget
        // failure cached across independent inspection attempts.
        value.entry_snapshots[0].parameter.index = 1000;
        let program = crate::contracts::CallableProgram::function(&value);
        assert!(matches!(
            InspectionFrame::new(program, &mut (cost - 1)),
            Err(Failure::Budget(_))
        ));
        assert!(matches!(
            InspectionFrame::new(program, &mut cost.clone()),
            Err(Failure::Invalid(_))
        ));
    }

    #[test]
    fn snapshot_and_result_alias_rejections_remain_distinct_from_budget() {
        let mut value = function(10);
        let input = value.parameters[0];
        let snapshot = LocalId {
            owner: 10,
            index: 1,
        };
        let result = LocalId {
            owner: 10,
            index: 2,
        };
        value.result_binding = Some(result);
        value.entry_snapshots.push(crate::program::EntrySnapshot {
            parameter: input,
            binding: snapshot,
        });
        for case in 0..5 {
            let mut invalid = value.clone();
            match case {
                0 => invalid.result_binding = Some(input),
                1 => invalid.interface.entry_snapshots[0].parameter = result,
                2 => invalid.interface.entry_snapshots[0].binding = input,
                3 => invalid.interface.entry_snapshots[0].binding = result,
                _ => invalid
                    .interface
                    .entry_snapshots
                    .push(invalid.interface.entry_snapshots[0].clone()),
            }
            let Err(Failure::Invalid(error)) = InspectionFrame::new(
                crate::contracts::CallableProgram::function(&invalid),
                &mut 1000,
            ) else {
                panic!("invalid binding must be rejected");
            };
            assert_eq!(error.source, value.source);
            assert_eq!(
                error.message,
                if case == 0 {
                    "result binding aliases an input parameter"
                } else {
                    "invalid entry snapshot binding"
                }
            );
        }
        InspectionFrame::new(
            crate::contracts::CallableProgram::function(&value),
            &mut 1000,
        )
        .unwrap();
    }

    #[test]
    fn frame_indices_defaults_and_signatures_share_the_callers_budget() {
        use crate::callable_source::validate_frame;
        use crate::phases::{Error, ErrorKind};
        let value = function(10);
        let mut generous = 1000;
        validate_frame(
            crate::contracts::CallableProgram::declaration(&value.interface),
            false,
            &mut generous,
        )
        .unwrap();
        let cost = 1000 - generous;
        assert!(cost > 0);
        let mut fuel = cost;
        validate_frame(
            crate::contracts::CallableProgram::declaration(&value.interface),
            false,
            &mut fuel,
        )
        .unwrap();
        assert_eq!(fuel, 0);
        assert!(matches!(
            validate_frame(
                crate::contracts::CallableProgram::declaration(&value.interface),
                true,
                &mut fuel
            ),
            Err(Error {
                kind: ErrorKind::Budget,
                ..
            })
        ));
        let mut fuel = cost - 1;
        let failure = validate_frame(
            crate::contracts::CallableProgram::declaration(&value.interface),
            false,
            &mut fuel,
        )
        .unwrap_err();
        assert!(matches!(failure.kind, ErrorKind::Budget));
        assert_eq!(failure.source, value.defaults[0].1.source);
        assert_eq!(fuel, 0);
        validate_frame(
            crate::contracts::CallableProgram::declaration(&value.interface),
            true,
            &mut 1000,
        )
        .unwrap();

        let mut duplicate = value.clone();
        duplicate
            .interface
            .parameters
            .push(duplicate.interface.parameters[0]);
        let Term::Function { parameters, .. } = &mut duplicate.signature else {
            unreachable!()
        };
        parameters.push(parameters[0].clone());
        for quote in [false, true] {
            assert!(matches!(
                validate_frame(
                    crate::contracts::CallableProgram::declaration(&duplicate.interface),
                    quote,
                    &mut 1000
                ),
                Err(Error {
                    kind: ErrorKind::Invalid,
                    ..
                })
            ));
        }
    }

    #[test]
    fn production_pass_keeps_one_parameter_allowance_across_callables_and_aliases() {
        let package = Package {
            functions: vec![function(20), function(30)],
            quote_templates: vec![function(40)],
            alias_contracts: vec![crate::type_aliases::Contract {
                declaration: DeclarationId {
                    package: 1,
                    local: 50,
                },
                parameters: vec![],
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
            }],
            ..Default::default()
        };
        let run = |package, fuel: &mut usize| {
            super::super::arithmetic_obligations_with_budgets(package, fuel, &mut 1_000_000)
        };
        let mut fuel = 1000;
        run(&package, &mut fuel).unwrap();
        let cost = 1000 - fuel;
        let mut exact = cost;
        run(&package, &mut exact).unwrap();
        assert_eq!(exact, 0);
        let mut insufficient = cost - 1;
        let error = run(&package, &mut insufficient).unwrap_err();
        assert_eq!(error.source, package.functions[1].defaults[0].1.source);
        assert_eq!(
            error.message,
            "function default binding analysis budget exhausted"
        );
        assert_eq!(insufficient, 0);
        run(&package, &mut 1000).unwrap();
    }
}
