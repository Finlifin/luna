use super::*;
use crate::{DeclarationId, Primitive, code_interfaces as code};
use std::sync::Arc;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 31, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn expr() -> n::Expr {
    n::Expr {
        source: n::SourceRange { start: 0, end: 1 },
        ty: boolean(),
        kind: n::ExprKind::Literal(n::Literal::Bool(true)),
    }
}
fn text() -> n::Expr {
    n::Expr {
        ty: Term::Primitive(Primitive::Str),
        kind: n::ExprKind::Literal(n::Literal::String("x".repeat(2048))),
        ..expr()
    }
}
fn function() -> n::Function {
    n::Function {
        interface: crate::program::FunctionDeclaration {
            source: crate::program::SourceRange { start: 0, end: 0 },
            phase: n::FunctionPhase::Comptime,
            inline: false,
            pure: true,
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
            declaration: id(1),
            name: "run".into(),
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(boolean()),
            },
            parameters: vec![],
            env_parameters: vec![],
            defaults: vec![],
        },
        body: expr(),
        runtime_deliveries: Default::default(),
    }
}
fn interface() -> n::Trait {
    n::Trait {
        comptime_only: false,
        parameter_types: vec![],
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        declaration: id(2),
        parents: vec![],
        self_parameter: id(3),
        name: "Contract".into(),
        methods: vec![],
    }
}
fn method() -> n::MethodContract {
    n::MethodContract {
        source: n::SourceRange { start: 0, end: 0 },
        phase: n::TraitPhase::Runtime,
        is_quote: false,
        pure: false,
        is_unsafe: false,
        type_arguments: vec![],
        type_parameters: vec![],
        capabilities: vec![],
        program: Default::default(),
        parameter_bindings: vec![],
        by_value_receiver: false,
        by_reference_receiver: false,
        declaration: id(4),
        name: "run".into(),
        signature: function().interface.signature.into(),
        requirements: vec![],
        default: None,
    }
}
fn implementation() -> n::Implementation {
    n::Implementation {
        phases: Default::default(),
        parameter_types: vec![],
        associated: vec![],
        associated_initializers: vec![],
        interface_arguments: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        requirements: vec![],
        declaration: id(5),
        subject: boolean(),
        interface: Some(id(2)),
        methods: vec![],
    }
}
fn enumeration() -> n::Enum {
    n::Enum {
        is_intrinsic: false,
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: id(6),
        name: "Encoding".into(),
        variants: vec![],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
    }
}
fn packages() -> Vec<(&'static str, n::Package)> {
    let mut cases = vec![];
    let mut package = n::Package::default();
    let mut body = function();
    body.defaults.push((0, text()));
    package.functions.push(body);
    cases.push(("function defaults", package));

    let mut package = n::Package::default();
    let mut body = function();
    body.phase = n::FunctionPhase::Runtime;
    body.preconditions.push(text());
    package.quote_templates.push(body);
    cases.push(("quote contracts", package));

    let mut package = n::Package::default();
    package.structs.push(n::Struct {
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: id(7),
        name: "Record".into(),
        fields: vec![("field".repeat(512), boolean())],
        packed: false,
    });
    cases.push(("nominal labels", package));

    let mut package = n::Package::default();
    package.newtypes.push(crate::newtypes::Newtype {
        declaration: id(8),
        name: "Wrapper".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: Term::Tuple(vec![boolean(); 2048]),
    });
    cases.push(("newtype representation", package));

    let mut package = n::Package::default();
    package.alias_contracts.push(crate::type_aliases::Contract {
        declaration: id(9),
        parameters: vec![],
        parameter_types: vec![],
        requirements: vec![n::TypePredicate::Bool(false); 2048],
        capabilities: vec![],
    });
    cases.push(("alias predicates", package));

    let mut package = n::Package::default();
    package.normal_forms.push(crate::normal_forms::Head {
        collector: None,
        role: Default::default(),
        declaration: id(10),
        parameters: vec![],
        requirements: vec![n::TypePredicate::Bool(false); 2048],
    });
    cases.push(("opaque head predicates", package));

    let mut package = n::Package::default();
    package.effects.push(crate::effects::Declaration {
        declaration: id(11),
        group: None,
        name: "effect".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        signature: function().interface.signature,
        abi: crate::effects::Abi::Direct,
        inline: true,
        requirements: vec![],
        capabilities: vec![n::TraitRequirement {
            interface: id(2),
            interface_arguments: vec![Term::Tuple(vec![boolean(); 2048])],
            subject: boolean(),
        }],
    });
    cases.push(("effect capabilities", package));

    let mut package = n::Package::default();
    let mut contract = interface();
    let mut member = method();
    member.program.postconditions.push(text());
    contract.methods.push(member);
    package.traits.push(contract);
    cases.push(("method program contracts", package));

    let mut package = n::Package::default();
    let mut contract = interface();
    contract.associated.push(n::AssociatedContract {
        phase: n::TraitPhase::Comptime,
        declaration: id(12),
        name: "default".into(),
        ty: boolean(),
        default: Some(text()),
    });
    package.traits.push(contract);
    cases.push(("associated defaults", package));

    for pattern in [false, true] {
        let mut package = n::Package::default();
        let mut contract = interface();
        let mut member = method();
        let output = code::ExpressionCodeInterface {
            infer_context: false,
            context: Default::default(),
            ports: [("port".repeat(512), n::LocalId { owner: 1, index: 0 })].into(),
            defaults: Default::default(),
            result: boolean(),
        };
        member.signature = if pattern {
            code::MethodSignature::PatternGenerator(code::CodeFunctionSignature {
                clause_domains: vec![],
                input_binders: Default::default(),
                inputs: vec![],
                output: code::PatternCodeInterface {
                    subject: boolean(),
                    outputs: [("binding".repeat(512), boolean())].into(),
                },
            })
        } else {
            code::MethodSignature::Generator(code::CodeFunctionSignature {
                clause_domains: vec![],
                input_binders: Default::default(),
                inputs: vec![],
                output,
            })
        };
        contract.methods.push(member);
        package.traits.push(contract);
        cases.push((
            if pattern {
                "pattern outputs"
            } else {
                "context ports"
            },
            package,
        ));
    }
    let mut package = n::Package::default();
    package.runtime_instances.push(n::RuntimeInstance {
        declaration: id(1),
        arguments: vec![],
        function: function(),
        cleanup_accesses: vec![n::RuntimeCleanupAccess {
            parameter: 0,
            input_fields: vec![],
            through: vec![vec![(n::FieldOwner::Anonymous, 0); 2048]],
            fields: vec![],
            write: false,
            read_value: true,
            ty: boolean(),
            conditions: Default::default(),
        }],
    });
    cases.push(("runtime cleanup paths", package));

    let mut package = n::Package::default();
    let mut definition = enumeration();
    definition.symbol_encoding = Some(
        crate::enum_encoding::SymbolRepresentation::new(
            vec![crate::enum_encoding::SymbolVariant {
                name: "tag".into(),
                value: "x".repeat(2048),
            }],
            &mut comptime_runtime::Budget::default(),
        )
        .unwrap(),
    );
    package.enums.push(definition);
    cases.push(("symbol enum encoding", package));

    let mut bits = comptime_runtime::Budget::default();
    let width = 65_536;
    let mask =
        comptime_runtime::bitvec::BitVec::from_words(width, &vec![u32::MAX; width / 32], &mut bits)
            .unwrap();
    let value =
        comptime_runtime::bitvec::BitVec::from_words(width, &vec![0; width / 32], &mut bits)
            .unwrap();
    let variant =
        crate::enum_encoding::Variant::new("fixed".into(), mask, value, vec![], &mut bits).unwrap();
    let mut definition = enumeration();
    definition.encoding = Some(crate::enum_encoding::Representation {
        base_type: None,
        layout: crate::enum_encoding::Layout::new(width, vec![variant], &mut bits).unwrap(),
    });
    cases.push((
        "fixed enum encoding",
        n::Package {
            enums: vec![definition],
            ..Default::default()
        },
    ));

    let mut package = n::Package::default();
    package
        .function_parameters
        .push(crate::function_parameters::Declaration {
            declaration: id(1),
            parameters: vec![id(20)],
            domains: vec![Term::Tuple(vec![boolean(); 2048])],
        });
    cases.push(("declaration input domains", package));
    let mut package = n::Package::default();
    package
        .runtime_function_identities
        .push(n::RuntimeFunctionIdentity {
            source: crate::program::SourceRange { start: 0, end: 0 },
            requirements: vec![],
            capabilities: vec![],
            declaration: id(1),
            type_parameters: vec![],
            signature: Term::Tuple(vec![boolean(); 2048]),
        });
    cases.push(("runtime identity signatures", package));
    let mut package = n::Package::default();
    package.runtime_domains.insert(
        crate::RuntimeId {
            instance: 1,
            local: 0,
        },
        Term::Lift(Box::new(Term::Tuple(vec![boolean(); 2048]))),
    );
    cases.push(("runtime dependency domains", package));
    cases
}

#[test]
fn declaration_capture_charges_owned_payloads_and_preserves_opaque_data() {
    for (label, package) in packages() {
        let original = package.clone();
        assert_eq!(
            clone_declaration_source(&package, &mut 128),
            Err(Error::Steps),
            "{label}"
        );
        let mut fuel = 100_000;
        assert_eq!(
            clone_declaration_source(&package, &mut fuel).unwrap(),
            original,
            "{label}"
        );
        let cost = 100_000 - fuel;
        assert!(cost > 2048, "{label}");
        assert_eq!(
            clone_declaration_source(&package, &mut (cost - 1)),
            Err(Error::Steps),
            "{label}"
        );
        let mut exact = cost;
        assert_eq!(
            clone_declaration_source(&package, &mut exact).unwrap(),
            original,
            "{label}"
        );
        assert_eq!(exact, 0);
        assert_eq!(package, original, "{label}");
    }
}

#[test]
fn borrowed_representation_capture_accounts_for_every_retained_payload() {
    let mut tested = 0;
    for (label, package) in packages() {
        if package.structs.is_empty() && package.newtypes.is_empty() && package.enums.is_empty() {
            continue;
        }
        tested += 1;
        let charge = |fuel: &mut usize| {
            charge_representation_metadata(
                &package.structs,
                &package.newtypes,
                &package.enums,
                fuel,
            )
        };
        assert_eq!(charge(&mut 128), Err(Error::Steps), "{label}");
        let mut fuel = 100_000;
        charge(&mut fuel).unwrap();
        let cost = 100_000 - fuel;
        assert!(cost > 2048, "{label}");
        assert_eq!(charge(&mut (cost - 1)), Err(Error::Steps), "{label}");
        let mut exact = cost;
        charge(&mut exact).unwrap();
        assert_eq!(exact, 0, "{label}");
        // Whole-package capture and the borrowed representation view use the
        // same rules, including variant encodings and names, without needing
        // an owned intermediate Package for the latter.
        let mut whole = cost;
        assert_eq!(
            clone_declaration_source(&package, &mut whole).unwrap(),
            package,
            "{label}"
        );
        assert_eq!(whole, 0, "{label}");
    }
    assert!(tested >= 4);
}

#[test]
fn assembled_head_capture_charges_domains_and_synthesized_trait_predicates() {
    let domains = [Term::Tuple(vec![boolean(); 2048])];
    let predicates = [n::TypePredicate::Equal(boolean(), boolean())];
    let capabilities = [n::TraitRequirement {
        subject: boolean(),
        interface: id(8),
        interface_arguments: vec![Term::String("argument".repeat(512).into())],
    }];
    let charge =
        |fuel: &mut usize| charge_normal_form_payload(&domains, &predicates, &capabilities, fuel);
    assert_eq!(charge(&mut 128), Err(Error::Steps));
    let mut fuel = 100_000;
    charge(&mut fuel).unwrap();
    let cost = 100_000 - fuel;
    let package = n::Package {
        normal_forms: vec![crate::normal_forms::Head {
            collector: None,
            role: Default::default(),
            declaration: id(1),
            parameters: vec![crate::normal_forms::Parameter {
                declaration: id(2),
                binding: n::LocalId { owner: 1, index: 2 },
                kind: crate::normal_forms::Kind::Value,
                ty: domains[0].clone(),
            }],
            requirements: predicates
                .iter()
                .cloned()
                .chain(capabilities.iter().cloned().map(n::TypePredicate::Trait))
                .collect(),
        }],
        ..Default::default()
    };
    let mut whole = cost + 1; // The Package node is separate from the head.
    assert_eq!(
        clone_declaration_source(&package, &mut whole).unwrap(),
        package
    );
    assert_eq!(whole, 0);
    assert_eq!(charge(&mut (cost - 1)), Err(Error::Steps));
    let mut exact = cost;
    charge(&mut exact).unwrap();
    assert_eq!(exact, 0);
}

#[test]
fn representation_capture_keeps_depth_failures_distinct_from_steps() {
    let mut deep = boolean();
    for _ in 0..200 {
        deep = Term::Optional(Box::new(deep));
    }
    let mut definition = crate::newtypes::Newtype {
        declaration: id(1),
        name: "Wrapped".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: deep,
    };
    let mut fuel = 100_000;
    assert_eq!(
        charge_representation_metadata([], [&definition], [], &mut fuel),
        Err(Error::Depth)
    );
    assert!(fuel > 0 && fuel < 100_000);
    // Replacing the excessive shape allows a fresh capture without inheriting
    // any partially prepared metadata or an invalid cached rejection.
    definition.representation = boolean();
    charge_representation_metadata([], [&definition], [], &mut fuel).unwrap();
}

#[test]
fn selected_dispatch_is_not_copied_or_treated_as_source_authority() {
    let mut package = n::Package::default();
    package.dispatch_instances.push(n::DispatchInstance {
        interface: id(2),
        interface_arguments: vec![Term::Tuple(vec![boolean(); 100_000])],
        subject: boolean(),
        implementation: id(5),
        arguments: vec![],
        methods: vec![],
        associated: vec![],
        associated_initializers: vec![],
    });
    let mut fuel = 1;
    assert_eq!(
        clone_declaration_source(&package, &mut fuel).unwrap(),
        n::Package::default()
    );
    assert_eq!(fuel, 0);
    assert_eq!(package.dispatch_instances.len(), 1);
}

#[test]
fn shared_closure_bodies_are_retained_but_owned_depth_still_has_a_limit() {
    let mut nested = expr();
    for _ in 0..180 {
        nested = n::Expr {
            kind: n::ExprKind::Block {
                statements: vec![],
                result: Some(Box::new(nested)),
            },
            ..expr()
        };
    }
    let shared = Arc::new(nested);
    let mut body = function();
    body.body.kind = n::ExprKind::Closure {
        parameters: vec![],
        captures: vec![],
        body: shared.clone(),
    };
    let mut package = n::Package {
        functions: vec![body],
        ..Default::default()
    };
    let count = Arc::strong_count(&shared);
    let cloned = clone_declaration_source(&package, &mut 100).unwrap();
    assert_eq!(Arc::strong_count(&shared), count + 1);
    drop(cloned);
    assert_eq!(Arc::strong_count(&shared), count);
    let mut ty = boolean();
    for _ in 0..180 {
        ty = Term::Optional(Box::new(ty));
    }
    package
        .function_parameters
        .push(crate::function_parameters::Declaration {
            declaration: id(1),
            parameters: vec![id(13)],
            domains: vec![ty],
        });
    assert_eq!(
        clone_declaration_source(&package, &mut 100_000),
        Err(Error::Depth)
    );
    assert_eq!(Arc::strong_count(&shared), count);
}

#[test]
fn declaration_comparison_enters_shared_code_that_source_capture_only_retains() {
    let mut body = expr();
    for _ in 0..180 {
        body = n::Expr {
            kind: n::ExprKind::Block {
                statements: vec![],
                result: Some(Box::new(body)),
            },
            ..expr()
        };
    }
    let shared = n::Expr {
        kind: n::ExprKind::Closure {
            parameters: vec![],
            captures: vec![],
            body: Arc::new(body),
        },
        ..expr()
    };
    let mut contract = method();
    contract.program.preconditions.push(shared);
    let mut declaration = interface();
    declaration.methods.push(contract);
    let package = n::Package {
        traits: vec![declaration],
        ..Default::default()
    };
    assert!(clone_declaration_source(&package, &mut 100_000).is_ok());
    assert_eq!(
        charge_trait_metadata(&package.traits[0], &mut 100_000),
        Err(Error::Depth)
    );
}

#[test]
fn capturing_an_initializer_shares_its_program_but_comparing_rows_observes_it() {
    let initializer =
        crate::associated_initializers::AssociatedInitializer::new(text(), &mut 100_000).unwrap();
    let mut row = implementation();
    row.associated_initializers.push((id(8), initializer));
    let package = n::Package {
        implementations: vec![row],
        ..Default::default()
    };
    let captured = clone_declaration_source(&package, &mut 128).unwrap();
    let original = &package.implementations[0];
    let retained = &captured.implementations[0];
    assert!(std::ptr::eq(
        original.associated_initializers[0].1.expression(),
        retained.associated_initializers[0].1.expression()
    ));
    assert_eq!(
        charge_implementation_metadata(retained, &mut 128),
        Err(Error::Steps)
    );
    charge_implementation_metadata(retained, &mut 100_000).unwrap();
}

#[test]
fn capturing_attribute_worlds_retains_the_exact_publication_state() {
    use n::DefinitionAttributes;
    use std::collections::{BTreeMap, BTreeSet};
    let target = id(3);
    let plain = id(8);
    let mut package = n::Package {
        definition_attributes: DefinitionAttributes::Pending(BTreeSet::from([target]).into()),
        ..Default::default()
    };
    let before = clone_declaration_source(&package, &mut 1).unwrap();
    let payload = crate::objects::value(vec![crate::objects::entry(
        Some("data"),
        Term::Primitive(Primitive::Str),
        Term::String("x".repeat(100_000).into()),
    )]);
    package.definition_attributes =
        DefinitionAttributes::Frozen(BTreeMap::from([(target, payload)]).into());
    let after = clone_declaration_source(&package, &mut 1).unwrap();
    assert_eq!(before.definition_attributes.lookup(target), Err(target));
    assert_eq!(before.definition_attributes.lookup(plain), Ok(None));
    assert_eq!(after.definition_attributes.lookup(plain), Ok(None));
    assert!(std::ptr::eq(
        after.definition_attributes.lookup(target).unwrap().unwrap(),
        package
            .definition_attributes
            .lookup(target)
            .unwrap()
            .unwrap(),
    ));
    assert!(
        before
            .definition_attributes
            .to_string()
            .contains(&target.to_string())
    );
    assert_eq!(
        clone_declaration_source(&package, &mut 0),
        Err(Error::Steps)
    );
}
