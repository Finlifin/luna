//! Concrete ABI metadata used when lowering effectful machine-call boundaries.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

fn graph() -> iir::Package {
    let package = support::compile(
        "^attrs.effects.direct_passing effect setting()->bool;
         effect invert(value:bool)->bool;
         fn request()->#[setting,invert] bool {invert(setting()#)#}
         fn run()->bool {request()# {setting()=>true,invert(value)=>not value,}}",
    );
    luna::instantiation::instantiate(&package, "run", 100000).unwrap()
}

#[test]
fn concrete_contract_table_preserves_row_occurrences_and_graph_identity() {
    let graph = graph();
    let table = iir::effects::Contracts::acquire(&graph, &mut 100000).unwrap();
    assert!(table.belongs_to(&graph));
    assert!(!table.belongs_to(&graph.clone()));
    let function = graph
        .functions
        .iter()
        .find(|f| f.function.name == "request")
        .unwrap();
    let nir::Term::Function { result, .. } = &function.function.signature else {
        panic!()
    };
    let nir::Term::Computation { effects, .. } = result.as_ref() else {
        panic!()
    };
    let row = table.row(effects, &mut 10000).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0].abi, nir::effects::Abi::DirectPassing);
    assert_eq!(row[1].abi, nir::effects::Abi::Direct);
    assert!(nir::effects::scalar_passing_signature(&row[0].signature));
    let reordered = vec![effects[1].clone(), effects[0].clone(), effects[1].clone()];
    let selected = table.row(&reordered, &mut 10000).unwrap();
    assert!(std::ptr::eq(selected[0], selected[2]));
    assert_eq!(
        selected.iter().map(|c| &c.operation).collect::<Vec<_>>(),
        reordered.iter().collect::<Vec<_>>()
    );
    assert!(table.row(effects, &mut 0).is_err());
    assert_eq!(table.row(effects, &mut 10000).unwrap(), row);
    let mut unregistered_instance = effects[0].clone();
    unregistered_instance
        .arguments
        .push(nir::Term::Primitive(nir::Primitive::Bool));
    assert!(
        table
            .get(&unregistered_instance, &mut 10000)
            .unwrap_err()
            .contains("contract missing")
    );
    let dump = table.to_string();
    assert!(
        dump.starts_with("(effect-contracts (effect-contract "),
        "{dump}"
    );
    assert_eq!(dump.matches("(effect-contract ").count(), 2);
    assert!(dump.contains("(abi direct-passing)"));
    assert!(dump.contains("(abi direct)"));
    assert_eq!(
        dump,
        iir::effects::Contracts::acquire(&graph.clone(), &mut 100000)
            .unwrap()
            .to_string()
    );
}

#[test]
fn conflicting_contracts_and_exhausted_acquisition_publish_no_table() {
    let mut graph = graph();
    assert!(iir::effects::Contracts::acquire(&graph, &mut 0).is_err());
    iir::effects::Contracts::acquire(&graph, &mut 100000).unwrap();
    for instance in &mut graph.functions {
        nir::visit::expression_mut(&mut instance.function.body, &mut 10000, |expr, _| {
            if let nir::program::ExprKind::EffectRequest { abi, .. } = &mut expr.kind {
                if *abi == nir::effects::Abi::DirectPassing {
                    *abi = nir::effects::Abi::Direct;
                }
            }
            Ok(())
        })
        .unwrap();
    }
    let error = match iir::effects::Contracts::acquire(&graph, &mut 100000) {
        Ok(_) => panic!("conflicting operation ABI published"),
        Err(error) => error,
    };
    assert!(error.contains("conflicting operation contracts"), "{error}");
}

#[test]
fn generic_operation_instances_keep_distinct_concrete_signatures() {
    let package = support::compile(
        "effect echo(value:T)->T where T;
         fn run()->bool {
             let first = echo(true)# {echo<bool>(value)=>value,};
             let input:.u8'builtin = 7;
             let second = echo(input)# {echo<.u8'builtin>(value)=>value,};
             first and second == 7
         }",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    let table = iir::effects::Contracts::acquire(&graph, &mut 100000).unwrap();
    let mut requests = Vec::new();
    for instance in &graph.functions {
        nir::visit::expressions(&instance.function, &mut 10000, |expr, _| {
            if let nir::program::ExprKind::EffectRequest {
                operation,
                signature,
                ..
            } = &expr.kind
            {
                requests.push((operation.clone(), signature.as_ref().clone()));
            }
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].0.declaration, requests[1].0.declaration);
    assert_ne!(requests[0].0.arguments, requests[1].0.arguments);
    assert_ne!(requests[0].1, requests[1].1);
    for (operation, signature) in requests {
        assert_eq!(
            table.get(&operation, &mut 10000).unwrap().signature,
            signature
        );
    }
    assert_eq!(table.to_string().matches("(effect-contract ").count(), 2);
}

#[test]
fn unused_generic_rows_retain_concrete_declared_contracts_without_occurrences() {
    let source = support::compile(
        "^attrs.effects.direct_passing effect setting()->T where T;
         fn advertised()->#setting<T> bool where T {true}",
    );
    let roots = [nir::Primitive::Bool, nir::Primitive::U128].map(|primitive| iir::InstanceKey {
        declaration: support::id(&source, "advertised"),
        arguments: (vec![nir::Term::Primitive(primitive)]).into(),
    });
    let graph = elaboration::materialization::instantiate(
        &source,
        &roots,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(graph.effect_contracts.len(), 2);
    let table = iir::effects::Contracts::acquire(&graph, &mut 100000).unwrap();
    for function in &graph.functions {
        let nir::Term::Function { result, .. } = &function.function.signature else {
            panic!()
        };
        let nir::Term::Computation { effects, .. } = result.as_ref() else {
            panic!()
        };
        assert_eq!(effects.len(), 1);
        let contract = table.get(&effects[0], &mut 10000).unwrap();
        assert_eq!(contract.abi, nir::effects::Abi::DirectPassing);
        let nir::Term::Function { parameters, result } = &contract.signature else {
            panic!()
        };
        assert!(parameters.is_empty());
        assert_eq!(result.as_ref(), &function.key.arguments[0]);
        nir::visit::expressions(&function.function, &mut 10000, |expr, _| {
            assert!(!matches!(
                expr.kind,
                nir::program::ExprKind::EffectRequest { .. }
                    | nir::program::ExprKind::EffectHandle { .. }
            ));
            Ok(())
        })
        .unwrap();
    }
    let again = elaboration::materialization::instantiate(
        &source,
        &roots,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(graph.to_string(), again.to_string());
    assert!(
        graph
            .to_string()
            .contains("(effect-contracts (effect-contract ")
    );
    let mut duplicate = graph.clone();
    duplicate
        .effect_contracts
        .push(duplicate.effect_contracts[0].clone());
    assert!(
        duplicate
            .validate(&mut 100000)
            .unwrap_err()
            .contains("duplicate IIR declared effect contract")
    );
    let mut malformed = graph.clone();
    malformed.effect_contracts[0].signature = nir::Term::Primitive(nir::Primitive::Bool);
    assert!(
        malformed
            .validate(&mut 100000)
            .unwrap_err()
            .contains("effect signature is not a function")
    );
}

#[test]
fn unused_effect_contract_materializes_its_result_layout() {
    let source = support::compile(
        "struct Payload {flag:bool} effect unused()->Payload;
        fn advertised()->#unused bool {true}",
    );
    let graph = luna::instantiation::instantiate(&source, "advertised", 100000).unwrap();
    assert_eq!(graph.effect_contracts.len(), 1);
    let contract = &graph.effect_contracts[0];
    let nir::Term::Function { result, .. } = &contract.signature else {
        panic!()
    };
    assert!(matches!(result.as_ref(), nir::Term::Nominal { .. }));
    assert_eq!(graph.nominals.len(), 1);
    graph.validate(&mut 100000).unwrap();
}

#[test]
fn passing_data_layout_checks_nested_variants_and_rejects_hidden_dependencies() {
    let source = support::compile(
        "struct Payload {flag:bool}
        enum Choice {empty,full(Payload),}
        struct Borrowed {target:*bool}
        struct Callback {action:fn()->bool}
        newtype Wrapper<T> = T;
        ^attrs.effects.direct_passing effect good()->Wrapper<Choice>;
        ^attrs.effects.direct_passing effect pointer()->Wrapper<Borrowed>;
        ^attrs.effects.direct_passing effect callable()->Wrapper<Callback>;
        fn advertised()->#[good,pointer,callable] bool {true}",
    );
    let graph = luna::instantiation::instantiate(&source, "advertised", 100000).unwrap();
    let contracts = iir::effects::Contracts::acquire(&graph, &mut 100000).unwrap();
    let nir::Term::Function { result, .. } = &graph
        .functions
        .iter()
        .find(|f| f.function.name == "advertised")
        .unwrap()
        .function
        .signature
    else {
        panic!()
    };
    let nir::Term::Computation { effects, .. } = result.as_ref() else {
        panic!()
    };
    for (operation, expected) in effects.iter().zip([true, false, false]) {
        let contract = contracts.get(operation, &mut 100000).unwrap();
        assert!(iir::effects::data_passing_signature(&graph, &contract.signature, &mut 0).is_err());
        assert_eq!(
            iir::effects::data_passing_signature(&graph, &contract.signature, &mut 100000).unwrap(),
            expected
        );
    }
    let signature = &contracts.get(&effects[0], &mut 100000).unwrap().signature;
    let nir::Term::Function {
        result: wrapped, ..
    } = signature
    else {
        panic!()
    };
    let mut cyclic = graph.clone();
    let instance = cyclic
        .nominals
        .iter_mut()
        .find(|instance| instance.ty == **wrapped)
        .unwrap();
    let iir::NominalDefinition::Newtype(definition) = &mut instance.definition else {
        panic!()
    };
    definition.representation = instance.ty.clone();
    let error = iir::effects::data_passing_signature(&cyclic, signature, &mut 100000).unwrap_err();
    assert!(error.contains("depth"), "{error}");
    assert!(iir::effects::data_passing_signature(&graph, signature, &mut 100000).unwrap());
}

#[test]
fn passing_error_result_layout_checks_all_inhabited_alternatives() {
    use nir::{Primitive as P, Term};
    let graph = iir::Package::default();
    let boolean = Term::Primitive(P::Bool);
    let pointer = Term::Pointer(Box::new(boolean.clone()));
    for (errors, result, accepted) in [
        (vec![boolean.clone()], boolean.clone(), true),
        (vec![boolean.clone()], Term::Primitive(P::Never), true),
        (vec![pointer.clone()], boolean.clone(), false),
        (vec![boolean.clone()], pointer.clone(), false),
        (
            vec![boolean.clone(), pointer.clone()],
            boolean.clone(),
            false,
        ),
    ] {
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::ErrorResult {
                errors,
                result: Box::new(result),
            }),
        };
        assert!(iir::effects::data_passing_signature(&graph, &signature, &mut 0).is_err());
        assert_eq!(
            iir::effects::data_passing_signature(&graph, &signature, &mut 100000).unwrap(),
            accepted
        );
    }
}
