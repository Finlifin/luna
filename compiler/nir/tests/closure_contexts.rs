use nir::{
    DeclarationId, DeclarationParameter, Parameter, Primitive as P, RuntimeId, Sort, Term,
    TypeError,
};

fn boolean() -> Term {
    Term::Primitive(P::Bool)
}
fn callback() -> Term {
    Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: boolean(),
        }],
        result: Box::new(boolean()),
    }
}
fn bound(depth: u32, index: usize, sort: Sort) -> Term {
    Term::Bound { depth, index, sort }
}
fn context_parameter() -> DeclarationParameter {
    DeclarationParameter {
        declaration: DeclarationId {
            package: 1,
            local: 7,
        },
        sort: Sort::ComptimeValue,
    }
}
fn template() -> Term {
    Term::contextual_closure(
        Term::Rigid {
            declaration: context_parameter().declaration,
            sort: Sort::ComptimeValue,
        },
        callback(),
    )
    .unwrap()
}

#[test]
fn context_parameters_complete_to_existing_abi_or_empty_signature() {
    let parameter = context_parameter();
    let generic = template();
    let scoped = generic.abstract_parameters(&[parameter]).unwrap();
    scoped.check_scope(&[vec![Sort::ComptimeValue]]).unwrap();
    assert_eq!(
        scoped
            .substitute(&[Term::ClosureContextValue(vec![])])
            .unwrap(),
        callback()
    );
    assert_eq!(
        scoped
            .substitute(&[Term::ClosureContextValue(vec![boolean()])])
            .unwrap(),
        Term::Closure {
            environment: vec![boolean()],
            signature: Box::new(callback())
        }
    );
    assert_eq!(
        generic
            .instantiate_parameters(&[parameter], &[Term::ClosureContextValue(vec![])])
            .unwrap(),
        callback()
    );
    assert_eq!(generic, template());
}

#[test]
fn same_sort_is_not_the_closure_context_domain() {
    let scoped = template()
        .abstract_parameters(&[context_parameter()])
        .unwrap();
    for wrong in [
        Term::Bool(true),
        Term::TypeListValue(vec![boolean()]),
        Term::TupleValue(vec![]),
        Term::Unit,
    ] {
        assert_eq!(
            scoped.substitute(&[wrong]),
            Err(TypeError::InvalidClosureContext)
        );
    }
    assert!(Term::contextual_closure(boolean(), callback()).is_err());
    assert!(
        Term::ClosureContextValue(vec![Term::Bool(true)])
            .check_scope(&[])
            .is_err()
    );
    assert_eq!(Term::ClosureContextType.sort(), Sort::Type);
    assert_eq!(
        Term::ClosureContextValue(vec![]).sort(),
        Sort::ComptimeValue
    );
}

#[test]
fn contextual_types_validate_context_scope_and_signature_shape() {
    let valid = Term::contextual_closure(bound(0, 0, Sort::ComptimeValue), callback()).unwrap();
    assert!(valid.check_scope(&[]).is_err());
    assert!(valid.check_scope(&[vec![Sort::Type]]).is_err());
    valid.check_scope(&[vec![Sort::ComptimeValue]]).unwrap();
    let malformed = Term::ContextualClosure {
        context: Box::new(Term::Bool(true)),
        signature: Box::new(callback()),
    };
    assert_eq!(
        malformed.check_scope(&[]),
        Err(TypeError::InvalidClosureContext)
    );
    assert!(Term::contextual_closure(Term::ClosureContextValue(vec![]), boolean()).is_err());
    assert!(
        Term::contextual_closure(Term::ClosureContextValue(vec![boolean()]), template()).is_err()
    );
}

#[test]
fn substitution_does_not_capture_nested_function_binders() {
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(P::Type),
        }],
        result: Box::new(Term::Tuple(vec![
            bound(0, 0, Sort::Type),
            bound(1, 1, Sort::Type),
        ])),
    };
    let ty = Term::contextual_closure(bound(0, 0, Sort::ComptimeValue), signature).unwrap();
    ty.check_scope(&[vec![Sort::ComptimeValue, Sort::Type]])
        .unwrap();
    let replaced = ty
        .substitute(&[
            Term::ClosureContextValue(vec![bound(0, 0, Sort::Type)]),
            Term::Primitive(P::Char),
        ])
        .unwrap();
    replaced.check_scope(&[vec![Sort::Type]]).unwrap();
    let Term::Closure {
        environment,
        signature,
    } = replaced
    else {
        panic!("lost context")
    };
    assert_eq!(environment, vec![bound(0, 0, Sort::Type)]);
    let Term::Function { result, .. } = *signature else {
        panic!("lost signature")
    };
    assert_eq!(
        *result,
        Term::Tuple(vec![bound(0, 0, Sort::Type), Term::Primitive(P::Char)])
    );
}

#[test]
fn context_fields_preserve_order_opaque_heads_and_runtime_identity() {
    let id = RuntimeId {
        instance: 3,
        local: 2,
    };
    let head = DeclarationId {
        package: 1,
        local: 8,
    };
    let dependent = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::Runtime(id)],
    };
    let context = Term::ClosureContextValue(vec![dependent.clone(), boolean()]);
    assert_eq!(context.runtime_dependencies(), [id].into_iter().collect());
    let completed = Term::contextual_closure(context.clone(), callback()).unwrap();
    assert_eq!(completed.runtime_dependencies(), [id].into_iter().collect());
    assert_ne!(
        context,
        Term::ClosureContextValue(vec![boolean(), dependent.clone()])
    );
    assert_eq!(context.substitute(&[]).unwrap(), context);
    let mut pool = nir::TypeInterner::default();
    let reference = pool
        .abstract_runtime(&completed, &[(id, boolean())])
        .unwrap();
    assert_eq!(
        reference.runtime_dependencies().unwrap(),
        [id].into_iter().collect()
    );
}

#[test]
fn callable_helpers_preserve_symbolic_environment_and_phase() {
    let context = bound(0, 0, Sort::ComptimeValue);
    let ty =
        Term::contextual_closure(context.clone(), callback().with_comptime_phase(true)).unwrap();
    assert!(ty.is_comptime_callable());
    assert_eq!(ty.callable_signature(), &callback());
    let changed = ty.with_callable_result(Term::Primitive(P::Char)).unwrap();
    assert!(changed.is_comptime_callable());
    let Term::ContextualClosure {
        context: retained, ..
    } = changed
    else {
        panic!("lost context")
    };
    assert_eq!(*retained, context);
}

#[test]
fn unfinished_contexts_and_context_values_cannot_be_runtime_storage() {
    let package = nir::program::Package::default();
    for ty in [template(), Term::ClosureContextType] {
        assert!(nir::phases::validate_runtime_storage(&ty, &package, &[]).is_err());
    }
    let completed =
        Term::contextual_closure(Term::ClosureContextValue(vec![boolean()]), callback()).unwrap();
    nir::phases::validate_runtime_storage(&completed, &package, &[]).unwrap();
}

#[test]
fn captured_context_values_require_the_context_domain() {
    let context = Term::ClosureContextValue(vec![boolean()]);
    nir::comptime_values::validate(&context, &Term::ClosureContextType, &mut 1000).unwrap();
    assert!(
        nir::comptime_values::validate(&Term::Bool(true), &Term::ClosureContextType, &mut 1000)
            .is_err()
    );
    assert!(nir::comptime_values::validate(&context, &boolean(), &mut 1000).is_err());
    let scoped = template()
        .abstract_parameters(&[context_parameter()])
        .unwrap();
    nir::phases::validate_runtime_storage(
        &scoped,
        &Default::default(),
        &[vec![Sort::ComptimeValue]],
    )
    .unwrap();
    assert!(nir::phases::validate_runtime_storage(&scoped, &Default::default(), &[]).is_err());
}

#[test]
fn declaration_admission_checks_context_domain_fields_scope_and_budget() {
    let parameter = context_parameter().declaration;
    let validate = |value: Term, domain: Term, fuel: &mut usize| {
        nir::validate_declaration_arguments_with_layouts(
            &[parameter],
            &[domain],
            &[value],
            fuel,
            |_, _, _| {
                Err(nir::NominalAdmissionError::from(
                    "unexpected nominal layout",
                ))
            },
        )
    };
    for fields in [vec![], vec![boolean(), Term::Primitive(P::Char)]] {
        validate(
            Term::ClosureContextValue(fields),
            Term::ClosureContextType,
            &mut 1000,
        )
        .unwrap();
    }
    for value in [
        Term::Bool(true),
        Term::TypeListValue(vec![boolean()]),
        Term::ClosureContextValue(vec![Term::Bool(true)]),
        Term::ClosureContextValue(vec![bound(0, 0, Sort::Type)]),
    ] {
        assert!(validate(value, Term::ClosureContextType, &mut 1000).is_err());
    }
    assert!(validate(Term::ClosureContextValue(vec![]), boolean(), &mut 1000).is_err());
    assert!(matches!(
        validate(
            Term::ClosureContextValue(vec![boolean(); 100]),
            Term::ClosureContextType,
            &mut 10
        ),
        Err(nir::NominalAdmissionError::Budget)
    ));
}

#[test]
fn context_traversal_charges_fields_and_nested_signatures() {
    let context = Term::ClosureContextValue(vec![boolean(); 100]);
    assert!(nir::type_computations::charge_term(&context, &mut 10).is_err());
    let mut fuel = 1000;
    nir::type_computations::charge_term(&context, &mut fuel).unwrap();
    assert!(fuel <= 899);
    let symbolic = Term::contextual_closure(
        Term::NormalForm {
            head: DeclarationId {
                package: 1,
                local: 9,
            },
            sort: Sort::ComptimeValue,
            arguments: vec![context],
        },
        callback(),
    )
    .unwrap();
    assert!(nir::type_computations::charge_term(&symbolic, &mut 10).is_err());
}

#[test]
fn contexts_dump_as_stable_sexpressions() {
    assert_eq!(
        Term::ClosureContextType.to_string(),
        "(closure-context-type)"
    );
    assert_eq!(
        Term::ClosureContextValue(vec![boolean()]).to_string(),
        "(closure-context (fields ((primitive bool))))"
    );
    let dump = template().to_string();
    assert!(dump.starts_with("(contextual-closure (context "));
    assert!(dump.contains("(signature (function "));
    assert_eq!(dump, template().to_string());
}
