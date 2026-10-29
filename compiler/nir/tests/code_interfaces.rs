use nir::code_interfaces::ExpressionCodeInterface;
use nir::program::LocalId;
use nir::type_computations::Error;
use nir::{DeclarationId, Primitive, Sort, Term};
use std::collections::BTreeMap;

fn interface() -> ExpressionCodeInterface {
    let input = LocalId { owner: 2, index: 1 };
    let fixed = LocalId { owner: 2, index: 2 };
    let ty = Term::Rigid {
        declaration: DeclarationId {
            package: 1,
            local: 3,
        },
        sort: Sort::Type,
    };
    ExpressionCodeInterface {
        infer_context: false,
        context: BTreeMap::from([
            (input, ty.clone()),
            (fixed, Term::Primitive(Primitive::Bool)),
        ]),
        ports: BTreeMap::from([("input\"\n".into(), input)]),
        defaults: BTreeMap::from([(input, LocalId { owner: 8, index: 4 })]),
        result: Term::NormalForm {
            head: DeclarationId {
                package: 1,
                local: 4,
            },
            sort: Sort::Type,
            arguments: vec![ty],
        },
    }
}

#[test]
fn substitution_preserves_normal_form_head_and_fixed_dependencies() {
    let original = interface();
    let output = original
        .try_map_types(|ty| {
            ty.instantiate_types(
                &[DeclarationId {
                    package: 1,
                    local: 3,
                }],
                &[Term::Primitive(Primitive::Char)],
            )
        })
        .unwrap();
    assert_eq!(
        output.context[&LocalId { owner: 2, index: 1 }],
        Term::Primitive(Primitive::Char)
    );
    assert_eq!(
        output.context[&LocalId { owner: 2, index: 2 }],
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(output.ports, original.ports);
    assert_eq!(output.defaults, original.defaults);
    assert_eq!(
        output.result,
        Term::NormalForm {
            head: DeclarationId {
                package: 1,
                local: 4
            },
            sort: Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Char)]
        }
    );
    assert_ne!(output.result, Term::Primitive(Primitive::Char));
    assert_eq!(original, interface());
}

#[test]
fn failed_type_mapping_does_not_publish_partial_interface() {
    let original = interface();
    let mut seen = 0;
    let result = original.try_map_types(|_| {
        seen += 1;
        if seen == 3 {
            Err("rejected context field")
        } else {
            Ok(Term::Primitive(Primitive::Char))
        }
    });
    assert_eq!(result, Err("rejected context field"));
    assert_eq!(original, interface());
    original
        .try_map_types(|ty| Ok::<_, ()>(ty.clone()))
        .unwrap();
}

#[test]
fn interface_budget_accounts_for_port_names_and_connections() {
    let original = interface();
    let mut remaining = 10000;
    original.charge(&mut remaining).unwrap();
    let needed = 10000 - remaining;
    assert_eq!(original.charge(&mut (needed - 1)), Err(Error::Budget));
    let mut exact = needed;
    original.charge(&mut exact).unwrap();
    assert_eq!(exact, 0);
    let mut long_name = original.clone();
    let binding = *long_name.ports.values().next().unwrap();
    long_name.ports = BTreeMap::from([("x".repeat(needed + 1), binding)]);
    assert_eq!(long_name.charge(&mut needed.clone()), Err(Error::Budget));
    let mut more_defaults = original.clone();
    more_defaults.defaults.insert(
        LocalId { owner: 2, index: 2 },
        LocalId { owner: 8, index: 5 },
    );
    assert_eq!(
        more_defaults.charge(&mut needed.clone()),
        Err(Error::Budget)
    );
    original.charge(&mut 10000).unwrap();
}

#[test]
fn interface_dump_is_deterministic_and_escapes_port_names() {
    let original = interface();
    let text = original.to_string();
    assert!(text.starts_with("(code-interface (infer-context false) (context"));
    assert!(text.contains("(port \"input\\\"\\n\" "));
    assert!(text.contains("(defaults (connect "));
    assert!(!text.contains('\n'));
    let mut reversed = original.clone();
    reversed.context = original
        .context
        .iter()
        .rev()
        .map(|(id, ty)| (*id, ty.clone()))
        .collect();
    assert_eq!(text, reversed.to_string());
}

fn signature() -> nir::code_interfaces::CodeFunctionSignature {
    use nir::code_interfaces::{CodeFunctionSignature, CodeInput};
    let code = interface();
    CodeFunctionSignature {
        clause_domains: vec![],
        inputs: vec![
            (
                LocalId { owner: 9, index: 0 },
                CodeInput::Data(code.context[&LocalId { owner: 2, index: 1 }].clone()),
            ),
            (
                LocalId { owner: 9, index: 1 },
                CodeInput::Code(code.clone()),
            ),
        ],
        output: code,
    }
}

#[test]
fn whole_signature_substitution_keeps_data_and_code_distinct() {
    use nir::code_interfaces::CodeInput;
    let original = signature();
    let instantiated = original
        .try_map_types(|ty| {
            ty.instantiate_types(
                &[DeclarationId {
                    package: 1,
                    local: 3,
                }],
                &[Term::Primitive(Primitive::Char)],
            )
        })
        .unwrap();
    assert_eq!(
        instantiated.inputs[0],
        (
            LocalId { owner: 9, index: 0 },
            CodeInput::Data(Term::Primitive(Primitive::Char))
        )
    );
    let CodeInput::Code(code) = &instantiated.inputs[1].1 else {
        panic!("code parameter erased to data");
    };
    assert_eq!(code, &instantiated.output);
    assert_eq!(
        code.context[&LocalId { owner: 2, index: 1 }],
        Term::Primitive(Primitive::Char)
    );
    assert_eq!(code.ports, original.output.ports);
    assert_eq!(code.defaults, original.output.defaults);
    assert_eq!(original, signature());
}

#[test]
fn whole_signature_failure_in_output_leaves_inputs_unchanged() {
    let original = signature();
    // Data input, then code-input result + context, then output result.
    let mut visited = 0;
    let result = original.try_map_types(|ty| {
        visited += 1;
        if visited == 5 {
            Err("rejected output")
        } else {
            Ok(ty.clone())
        }
    });
    assert_eq!(result, Err("rejected output"));
    assert_eq!(original, signature());
    assert_eq!(
        original
            .try_map_types(|ty| Ok::<_, ()>(ty.clone()))
            .unwrap(),
        original
    );
}

#[test]
fn whole_signature_budget_includes_inputs_and_nested_interfaces() {
    let original = signature();
    let mut remaining = 10000;
    original.charge(&mut remaining).unwrap();
    let required = 10000 - remaining;
    assert_eq!(original.charge(&mut (required - 1)), Err(Error::Budget));
    let mut exact = required;
    original.charge(&mut exact).unwrap();
    assert_eq!(exact, 0);
    let mut extended = original.clone();
    extended.inputs.push((
        LocalId { owner: 9, index: 2 },
        nir::code_interfaces::CodeInput::Data(Term::Primitive(Primitive::Bool)),
    ));
    assert_eq!(extended.charge(&mut required.clone()), Err(Error::Budget));
}

#[test]
fn whole_signature_dump_keeps_categories_and_context_structure() {
    let text = signature().to_string();
    assert!(text.starts_with("(code-function-signature (clauses) (inputs"));
    assert!(text.contains(" (data "));
    assert!(text.contains(" (code (code-interface "));
    assert!(text.contains("(output (code-interface "));
    assert_eq!(text.matches("(defaults (connect ").count(), 2);
    assert_eq!(text.matches("(port \"input\\\"\\n\" ").count(), 2);
}

#[test]
fn method_signature_preserves_category_through_specialization() {
    use nir::code_interfaces::MethodSignature;
    let original = MethodSignature::Generator(signature());
    assert_eq!(
        original.ordinary(),
        Err(nir::TypeError::ContextualCodeNotCallable)
    );
    assert_eq!(
        original.callable_signature(),
        Err(nir::TypeError::ContextualCodeNotCallable)
    );
    let specialized = original
        .instantiate_types(
            &[DeclarationId {
                package: 1,
                local: 3,
            }],
            &[Term::Primitive(Primitive::Char)],
        )
        .unwrap();
    let MethodSignature::Generator(code) = specialized else {
        panic!("code signature erased")
    };
    assert_eq!(
        code.output.context[&LocalId { owner: 2, index: 1 }],
        Term::Primitive(Primitive::Char)
    );
    let ordinary = MethodSignature::Callable(Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    })));
    assert!(ordinary.is_comptime_callable());
    assert!(matches!(
        ordinary.callable_signature().unwrap(),
        Term::Function { .. }
    ));
    assert_ne!(original, ordinary);
    assert_eq!(original.to_string(), signature().to_string());
}

#[test]
fn code_method_contract_cannot_publish_an_ordinary_program_summary() {
    let mut method = nir::program::MethodContract {
        phase: Default::default(),
        is_quote: false,
        pure: true,
        is_unsafe: false,
        type_arguments: vec![],
        type_parameters: vec![],
        capabilities: vec![],
        program: Default::default(),
        parameter_bindings: signature().inputs.iter().map(|(id, _)| *id).collect(),
        by_value_receiver: false,
        by_reference_receiver: false,
        declaration: DeclarationId {
            package: 1,
            local: 8,
        },
        name: "build".into(),
        signature: nir::code_interfaces::MethodSignature::Generator(signature()),
        requirements: vec![],
        default: None,
    };
    assert!(matches!(
        method.program_summary(&[], &[]),
        Err(nir::TypeError::ContextualCodeNotCallable)
    ));
    assert!(
        method
            .to_string()
            .contains("(signature (code-function-signature")
    );
    assert!(matches!(
        method.program_summary(
            &[DeclarationId {
                package: 1,
                local: 3
            }],
            &[]
        ),
        Err(nir::TypeError::ContextualCodeNotCallable)
    ));
    let mut ordinary = method.clone();
    ordinary.parameter_bindings.clear();
    ordinary.signature = Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    }))
    .into();
    assert_eq!(
        ordinary.program_summary(&[], &[]).unwrap().phase,
        nir::program::FunctionPhase::Comptime
    );
    ordinary.is_unsafe = true;
    ordinary.signature =
        Term::UnsafeFunction(Box::new(ordinary.signature.ordinary().unwrap().clone())).into();
    let summary = ordinary.program_summary(&[], &[]).unwrap();
    assert!(summary.is_unsafe);
    assert!(summary.callable_type().is_comptime_callable());
    assert_eq!(
        &summary.callable_type(),
        ordinary.signature.ordinary().unwrap()
    );
    assert!(ordinary.to_string().contains("(unsafe true)"));
    assert!(method.program_summary(&[], &[]).is_err());
    let consumer = consumer_signature();
    method.parameter_bindings = consumer.inputs.iter().map(|(id, _)| *id).collect();
    method.signature = nir::code_interfaces::MethodSignature::DataConsumer(consumer);
    assert!(matches!(
        method.program_summary(&[], &[]),
        Err(nir::TypeError::ContextualCodeNotCallable)
    ));
    assert!(
        method
            .to_string()
            .contains("(signature (data-consumer-signature")
    );
}

#[test]
fn clause_domains_participate_in_mapping_budget_and_dump() {
    let mut original = signature();
    original.clause_domains = vec![
        Term::Primitive(Primitive::Type),
        Term::Primitive(Primitive::Bool),
    ];
    let mapped = original
        .try_map_types(|ty| {
            Ok::<_, ()>(if *ty == Term::Primitive(Primitive::Bool) {
                Term::Primitive(Primitive::Char)
            } else {
                ty.clone()
            })
        })
        .unwrap();
    assert_eq!(
        mapped.clause_domains,
        vec![
            Term::Primitive(Primitive::Type),
            Term::Primitive(Primitive::Char)
        ]
    );
    assert_eq!(original.clause_domains[1], Term::Primitive(Primitive::Bool));
    let mut fuel = 10000;
    signature().charge(&mut fuel).unwrap();
    let base_cost = 10000 - fuel;
    assert_eq!(original.charge(&mut base_cost.clone()), Err(Error::Budget));
    let dump = mapped.to_string();
    assert!(
        dump.contains("(clauses (domain (primitive Type)) (domain (primitive char)))"),
        "{dump}"
    );
}

#[test]
fn pattern_input_interfaces_preserve_category_names_and_type_dependencies() {
    use nir::code_interfaces::{CodeFunctionSignature, CodeInput, PatternCodeInterface};
    let original = interface();
    let ty = original.context[&LocalId { owner: 2, index: 1 }].clone();
    let input = PatternCodeInterface {
        subject: ty.clone(),
        outputs: BTreeMap::from([("value\"\n".into(), ty)]),
    };
    let signature = CodeFunctionSignature {
        clause_domains: vec![],
        inputs: vec![(
            LocalId { owner: 1, index: 1 },
            CodeInput::Pattern(input.clone()),
        )],
        output: original,
    };
    let mapped = signature
        .try_map_types(|ty| {
            ty.instantiate_types(
                &[DeclarationId {
                    package: 1,
                    local: 3,
                }],
                &[Term::Primitive(Primitive::Bool)],
            )
        })
        .unwrap();
    let CodeInput::Pattern(result) = &mapped.inputs[0].1 else {
        panic!("lost pattern input category")
    };
    assert_eq!(result.subject, Term::Primitive(Primitive::Bool));
    assert!(result.outputs.keys().eq(input.outputs.keys()));
    assert_eq!(
        mapped.inputs[0].1.type_terms().collect::<Vec<_>>(),
        vec![&result.subject, result.outputs.values().next().unwrap()]
    );
    assert!(signature.charge(&mut 0).is_err());
    signature.charge(&mut 10000).unwrap();
    let dump = mapped.to_string();
    assert!(dump.contains("(pattern (pattern-interface"), "{dump}");
    assert!(dump.contains("value\\\"\\n"), "{dump}");
}

#[test]
fn pattern_method_signatures_preserve_output_category_names_and_type_identity() {
    use nir::code_interfaces::{
        CodeInput, MethodSignature, PatternCodeInterface, PatternFunctionSignature,
    };
    let parameter = DeclarationId {
        package: 1,
        local: 3,
    };
    let ty = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let signature = PatternFunctionSignature {
        clause_domains: vec![Term::Primitive(Primitive::Type)],
        inputs: vec![(LocalId { owner: 2, index: 1 }, CodeInput::Data(ty.clone()))],
        output: PatternCodeInterface {
            subject: ty.clone(),
            outputs: BTreeMap::from([("value\"\n".into(), ty)]),
        },
    };
    assert_eq!(signature.charge(&mut 0), Err(Error::Budget));
    signature.charge(&mut 1000).unwrap();
    let method = MethodSignature::PatternGenerator(signature);
    assert_eq!(
        method.ordinary(),
        Err(nir::TypeError::ContextualCodeNotCallable)
    );
    assert!(method.is_comptime_callable());
    assert_eq!(method.type_terms().count(), 4);
    let concrete = Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: 9,
        },
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let MethodSignature::PatternGenerator(specialized) = method
        .instantiate_types(&[parameter], &[concrete.clone()])
        .unwrap()
    else {
        panic!("lost pattern signature")
    };
    assert_eq!(specialized.output.subject, concrete);
    assert_eq!(specialized.output.outputs["value\"\n"], concrete);
    let dump = specialized.to_string();
    assert!(dump.starts_with("(pattern-function-signature"));
    assert!(dump.contains("value\\\"\\n"), "{dump}");
}

fn consumer_signature() -> nir::code_interfaces::DataFunctionSignature {
    use nir::code_interfaces::{CodeInput, DataFunctionSignature, PatternCodeInterface};
    let code = interface();
    let subject = code.result.clone();
    DataFunctionSignature {
        clause_domains: vec![Term::Primitive(Primitive::Type)],
        inputs: vec![
            (
                LocalId { owner: 9, index: 0 },
                CodeInput::Data(Term::Primitive(Primitive::Bool)),
            ),
            (LocalId { owner: 9, index: 1 }, CodeInput::Code(code)),
            (
                LocalId { owner: 9, index: 2 },
                CodeInput::Pattern(PatternCodeInterface {
                    subject: subject.clone(),
                    outputs: BTreeMap::from([("success\"\n".into(), subject)]),
                }),
            ),
        ],
        output: Term::Primitive(Primitive::Type),
    }
}

#[test]
fn contextual_data_results_do_not_erase_input_categories() {
    use nir::code_interfaces::CodeInput;
    let signature = consumer_signature();
    let before = signature.clone();
    let mapped = signature
        .try_map_types(|ty| {
            ty.instantiate_types(
                &[DeclarationId {
                    package: 1,
                    local: 3,
                }],
                &[Term::Primitive(Primitive::Char)],
            )
        })
        .unwrap();
    assert_eq!(mapped.output, Term::Primitive(Primitive::Type));
    assert!(matches!(
        mapped.inputs[0].1,
        CodeInput::Data(Term::Primitive(Primitive::Bool))
    ));
    let CodeInput::Code(expr) = &mapped.inputs[1].1 else {
        panic!()
    };
    let CodeInput::Pattern(pattern) = &mapped.inputs[2].1 else {
        panic!()
    };
    assert_eq!(
        expr.context[&LocalId { owner: 2, index: 1 }],
        Term::Primitive(Primitive::Char)
    );
    assert!(matches!(expr.result, Term::NormalForm { .. }));
    assert_eq!(expr.result, pattern.subject);
    assert_eq!(pattern.outputs.values().next().unwrap(), &expr.result);
    assert_eq!(expr.ports, interface().ports);
    assert_eq!(expr.defaults, interface().defaults);
    assert_eq!(signature, before);
    let dump = mapped.to_string();
    assert!(
        dump.starts_with("(data-consumer-signature (clauses"),
        "{dump}"
    );
    assert!(dump.contains("(code (code-interface"), "{dump}");
    assert!(dump.contains("(pattern (pattern-interface"), "{dump}");
    assert!(dump.contains("\"success\\\"\\n\""), "{dump}");
    assert!(!dump.contains('\n'));
}

#[test]
fn data_consumer_transforms_are_atomic_through_the_output() {
    let signature = consumer_signature();
    let original = signature.clone();
    let mut visited = Vec::new();
    let failure = signature.try_map_types(|ty| {
        visited.push(ty.clone());
        if visited.len() == 8 {
            Err("output rejected")
        } else {
            Ok(ty.clone())
        }
    });
    assert_eq!(failure, Err("output rejected"));
    assert_eq!(visited.last(), Some(&signature.output));
    assert_eq!(signature, original);
    assert_eq!(
        signature
            .try_map_types(|ty| Ok::<_, ()>(ty.clone()))
            .unwrap(),
        original
    );
}

#[test]
fn data_consumer_budgets_and_dependency_walk_include_code_interfaces() {
    let signature = consumer_signature();
    let mut fuel = 10_000;
    signature.charge(&mut fuel).unwrap();
    let work = 10_000 - fuel;
    assert_eq!(signature.charge(&mut (work - 1)), Err(Error::Budget));
    let mut exact = work;
    signature.charge(&mut exact).unwrap();
    assert_eq!(exact, 0);
    let terms: Vec<_> = signature.type_terms().cloned().collect();
    assert_eq!(terms.len(), 8);
    assert_eq!(terms[0], Term::Primitive(Primitive::Type));
    assert_eq!(terms.last(), Some(&Term::Primitive(Primitive::Type)));
    assert!(terms.iter().any(|ty| matches!(ty, Term::Rigid { .. })));
    assert_eq!(
        terms
            .iter()
            .filter(|ty| matches!(ty, Term::NormalForm { .. }))
            .count(),
        3
    );
}

#[test]
fn data_method_signatures_preserve_categories_and_dependencies() {
    use nir::code_interfaces::MethodSignature;
    let mut signature = consumer_signature();
    signature.output = Term::Rigid {
        declaration: DeclarationId {
            package: 1,
            local: 3,
        },
        sort: Sort::Type,
    };
    let original = MethodSignature::DataConsumer(signature.clone());
    assert!(original.is_comptime_callable());
    assert_eq!(
        original.ordinary(),
        Err(nir::TypeError::ContextualCodeNotCallable)
    );
    assert_eq!(
        original.callable_signature(),
        Err(nir::TypeError::ContextualCodeNotCallable)
    );
    assert_eq!(
        original.clause_domains(),
        Some(signature.clause_domains.as_slice())
    );
    assert_eq!(
        original.type_terms().collect::<Vec<_>>(),
        signature.type_terms().collect::<Vec<_>>()
    );
    let concrete = Term::Primitive(Primitive::Char);
    let mapped = original
        .instantiate_types(
            &[DeclarationId {
                package: 1,
                local: 3,
            }],
            &[concrete.clone()],
        )
        .unwrap();
    let MethodSignature::DataConsumer(mapped) = mapped else {
        panic!("lost data method category")
    };
    assert!(
        !mapped
            .type_terms()
            .any(|ty| ty.mentions_rigid(DeclarationId {
                package: 1,
                local: 3
            }))
    );
    assert!(mapped.type_terms().any(|ty| ty == &concrete));
    assert_eq!(mapped.output, concrete);
    assert_eq!(original.to_string(), signature.to_string());
    assert!(original.to_string().starts_with("(data-consumer-signature"));
    let before = original.clone();
    assert!(
        original
            .try_map_types(|_| Err::<Term, _>("rejected"))
            .is_err()
    );
    assert_eq!(original, before);
}
