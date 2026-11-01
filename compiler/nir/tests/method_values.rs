use nir::{DeclarationId, Primitive, Sort, Term, TraitMethodIdentity};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}
fn value(subject: Term, input: Term) -> Term {
    Term::MethodValue(Box::new(TraitMethodIdentity {
        interface: nir::program::InterfaceInstance {
            declaration: id(1),
            arguments: vec![input.clone()],
        },
        subject,
        member: id(2),
        method_arguments: vec![input],
    }))
}
#[test]
fn method_values_preserve_identity_scope_and_all_substitution_edges() {
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let open = value(bound.clone(), bound);
    assert_eq!(open.sort(), Sort::ComptimeValue);
    assert!(open.check_scope(&[]).is_err());
    open.check_scope(&[vec![Sort::Type]]).unwrap();
    let boolean = Term::Primitive(Primitive::Bool);
    let concrete = open.substitute(&[boolean.clone()]).unwrap();
    assert_eq!(concrete, value(boolean.clone(), boolean));
    concrete.check_scope(&[]).unwrap();
    let mut changed = concrete.clone();
    let Term::MethodValue(method) = &mut changed else {
        unreachable!()
    };
    method.member = id(3);
    assert_ne!(changed, concrete);
    let dump = concrete.to_string();
    assert!(dump.starts_with("(method-value (runtime-trait-method "));
    assert!(dump.contains(&format!("(subject {})", Term::Primitive(Primitive::Bool))));
    assert!(dump.contains(&format!(
        "(method-arguments {})",
        Term::Primitive(Primitive::Bool)
    )));
}
#[test]
fn method_inputs_cannot_hide_deep_or_wide_terms_from_preflight() {
    let boolean = Term::Primitive(Primitive::Bool);
    let deep = (0..65).fold(boolean.clone(), |ty, _| Term::Optional(Box::new(ty)));
    for input in [
        value(deep.clone(), boolean.clone()),
        value(boolean.clone(), deep),
    ] {
        assert!(nir::type_budget::charge_structure(&input, &mut 100_000).is_err());
    }
    let input = value(boolean, Term::String("x".repeat(1000).into()));
    assert!(nir::type_budget::charge_structure(&input, &mut 100).is_err());
    nir::type_budget::charge_structure(&input, &mut 10_000).unwrap();
}
