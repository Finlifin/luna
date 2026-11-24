use nir::{Parameter, Primitive as P, Sort, Term, TypeError};

fn signature() -> Term {
    Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Primitive(P::Bool),
        }],
        result: Box::new(Term::Primitive(P::U32)),
    }
}

#[test]
fn pure_identity_survives_staging_binding_and_result_replacement() {
    let ordinary = signature();
    assert_eq!(ordinary.clone().with_pure_qualifier(false), ordinary);
    let pure_type = Term::PureFunction(Box::new(ordinary.clone()));
    assert!(pure_type.is_pure_callable());
    assert!(!pure_type.is_unsafe_callable());
    assert!(!pure_type.is_comptime_callable());
    assert_eq!(ordinary.clone().with_pure_qualifier(true), pure_type);
    assert_eq!(pure_type.clone().with_pure_qualifier(true), pure_type);
    assert_eq!(pure_type.callable_signature(), &ordinary);
    assert_eq!(pure_type.sort(), Sort::Type);
    assert_eq!(
        format!("{pure_type}"),
        "(pure-fn (function (parameters ((parameter (sort runtime) (type (primitive bool))))) (result (primitive u32))))"
    );
    pure_type.check_scope(&[]).expect("pure runtime callable");

    for comptime in [false, true] {
        let expected = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: Term::Primitive(P::Bool),
            }],
            result: Box::new(Term::Primitive(P::I32)),
        };
        assert_eq!(
            pure_type
                .clone()
                .with_comptime_phase(comptime)
                .with_callable_result(Term::Primitive(P::I32)),
            Some(Term::PureFunction(Box::new(
                expected.with_comptime_phase(comptime)
            )))
        );
    }

    let staged = Term::PureFunction(Box::new(signature())).with_comptime_phase(true);
    assert!(staged.is_pure_callable());
    assert!(staged.is_comptime_callable());
    staged.check_scope(&[]).expect("pure comptime callable");

    let raw = Term::PureFunction(Box::new(signature().with_comptime_phase(true)));
    assert_eq!(staged, raw);
}

#[test]
fn pure_qualification_cannot_combine_with_unsafe_or_non_callable_terms() {
    let raw = signature();
    let closure = Term::Closure {
        environment: vec![],
        signature: Box::new(raw.clone()),
    };
    for invalid in [
        Term::PureFunction(Box::new(Term::Primitive(P::Bool))),
        Term::PureFunction(Box::new(Term::PureFunction(Box::new(raw.clone())))),
        Term::PureFunction(Box::new(Term::UnsafeFunction(Box::new(raw.clone())))),
        Term::PureFunction(Box::new(closure)),
    ] {
        assert_eq!(
            invalid.check_scope(&[]),
            Err(TypeError::InvalidPureFunctionSignature)
        );
    }
    let unsafe_wrapping_pure = Term::UnsafeFunction(Box::new(Term::PureFunction(Box::new(raw))));
    assert_eq!(
        unsafe_wrapping_pure.check_scope(&[]),
        Err(TypeError::InvalidUnsafeFunctionSignature)
    );
}

#[test]
fn pure_function_type_id_differs_from_ordinary_and_unsafe_function() {
    let mut pkg = nir::program::Package::default();
    pkg.target = Some(nir::target::TargetData::new(64).expect("target data"));
    let sig = signature();
    let mut fuel = 10_000;
    let ordinary_id = nir::type_id::compute_type_id(&pkg, &sig, &mut fuel).expect("ordinary id");
    let pure_id =
        nir::type_id::compute_type_id(&pkg, &Term::PureFunction(Box::new(sig.clone())), &mut fuel)
            .expect("pure id");
    let unsafe_id =
        nir::type_id::compute_type_id(&pkg, &Term::UnsafeFunction(Box::new(sig)), &mut fuel)
            .expect("unsafe id");
    assert_ne!(ordinary_id, pure_id);
    assert_ne!(pure_id, unsafe_id);
}
