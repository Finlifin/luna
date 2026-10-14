use nir::{DeclarationId, DeclarationParameter, Parameter, Primitive, Sort, Term};
use nir::program::{TraitRequirement, TypePredicate};

fn id(local: u32) -> DeclarationId { DeclarationId { package: 9, local } }
fn bound(depth: u32, index: usize, sort: Sort) -> Term {
    Term::Bound { depth, index, sort }
}
fn rigid(local: u32, sort: Sort) -> Term {
    Term::Rigid { declaration: id(local), sort }
}
fn inputs() -> Vec<DeclarationParameter> {
    vec![
        DeclarationParameter { declaration: id(1), sort: Sort::Type },
        DeclarationParameter { declaration: id(2), sort: Sort::ComptimeValue },
    ]
}
fn indexed(ty: Term, value: Term) -> Term {
    Term::NormalForm { head: id(3), sort: Sort::Type, arguments: vec![ty, value] }
}

#[test]
fn scoped_contracts_preserve_mixed_inputs_and_nested_function_binders() {
    let signature = Term::Function {
        parameters: vec![Parameter { sort: Sort::Type, ty: Term::Primitive(Primitive::Type) }],
        result: Box::new(Term::Tuple(vec![
            indexed(rigid(1, Sort::Type), rigid(2, Sort::ComptimeValue)),
            bound(0, 0, Sort::Type),
        ])),
    };
    let requirement = TraitRequirement {
        interface: id(4), subject: rigid(1, Sort::Type),
        interface_arguments: vec![signature],
    };
    let predicate = TypePredicate::And(vec![
        TypePredicate::Trait(requirement.clone()),
        TypePredicate::Or(vec![TypePredicate::Bool(false), TypePredicate::Not(Box::new(
            TypePredicate::Equal(rigid(1, Sort::Type), Term::Primitive(Primitive::Bool))))]),
    ]);
    let frames = vec![vec![Sort::Type, Sort::ComptimeValue]];
    let args = vec![bound(0, 0, Sort::Type), bound(0, 1, Sort::ComptimeValue)];
    let actual = predicate.instantiate_parameters_in_scope(&inputs(), &args, &frames).unwrap();
    let expected_signature = Term::Function {
        parameters: vec![Parameter { sort: Sort::Type, ty: Term::Primitive(Primitive::Type) }],
        result: Box::new(Term::Tuple(vec![
            indexed(bound(1, 0, Sort::Type), bound(1, 1, Sort::ComptimeValue)),
            bound(0, 0, Sort::Type),
        ])),
    };
    let expected_requirement = TraitRequirement {
        interface: id(4), subject: args[0].clone(), interface_arguments: vec![expected_signature],
    };
    assert_eq!(actual, TypePredicate::And(vec![
        TypePredicate::Trait(expected_requirement.clone()),
        TypePredicate::Or(vec![TypePredicate::Bool(false), TypePredicate::Not(Box::new(
            TypePredicate::Equal(args[0].clone(), Term::Primitive(Primitive::Bool))))]),
    ]));
    assert_eq!(requirement.instantiate_parameters_in_scope(&inputs(), &args, &frames).unwrap(), expected_requirement);
    assert!(predicate.instantiate_parameters(&inputs(), &args).is_err());
    assert!(requirement.instantiate_parameters(&inputs(), &args).is_err());
    let dump = actual.to_string();
    assert!(dump.contains("(normal-form"));
    assert!(dump.contains("(depth 1)"));
}

#[test]
fn scoped_contracts_validate_unused_inputs_and_output_scope() {
    let frames = vec![vec![Sort::Type, Sort::ComptimeValue]];
    let args = vec![bound(0, 0, Sort::Type), bound(0, 1, Sort::ComptimeValue)];
    for predicate in [TypePredicate::Bool(true), TypePredicate::And(vec![]), TypePredicate::Or(vec![])] {
        assert_eq!(predicate.instantiate_parameters_in_scope(&inputs(), &args, &frames).unwrap(), predicate);
        assert!(predicate.instantiate_parameters_in_scope(&inputs(), &args[..1], &frames).is_err());
        assert!(predicate.instantiate_parameters_in_scope(&inputs(), &args, &[]).is_err());
        assert!(predicate.instantiate_parameters_in_scope(&inputs(), &[args[1].clone(), args[0].clone()], &frames).is_err());
        assert!(predicate.instantiate_parameters_in_scope(&[inputs()[0], inputs()[0]], &[args[0].clone(), args[0].clone()], &frames).is_err());
    }
    let invalid = TypePredicate::Equal(bound(0, 2, Sort::Type), Term::Primitive(Primitive::Bool));
    assert!(invalid.instantiate_parameters_in_scope(&[], &[], &frames).is_err());
    let wrong_occurrence = TypePredicate::Equal(
        indexed(rigid(1, Sort::Type), rigid(2, Sort::Runtime)),
        Term::Primitive(Primitive::Bool),
    );
    assert!(wrong_occurrence.instantiate_parameters_in_scope(&inputs(), &args, &frames).is_err());
}

#[test]
fn partial_scoped_contract_instantiation_composes_without_unfolding_heads() {
    let template = TypePredicate::Equal(
        indexed(rigid(1, Sort::Type), rigid(2, Sort::ComptimeValue)),
        indexed(rigid(1, Sort::Type), rigid(2, Sort::ComptimeValue)),
    );
    let frames = vec![vec![Sort::Type, Sort::ComptimeValue]];
    let args = vec![bound(0, 0, Sort::Type), bound(0, 1, Sort::ComptimeValue)];
    let first = template.instantiate_parameters_in_scope(&inputs()[..1], &args[..1], &frames).unwrap();
    let second = first.instantiate_parameters_in_scope(&inputs()[1..], &args[1..], &frames).unwrap();
    assert_eq!(second, template.instantiate_parameters_in_scope(&inputs(), &args, &frames).unwrap());
    assert!(second.to_string().contains("(normal-form"));
}

#[test]
fn nominal_fields_and_variants_instantiate_in_explicit_input_frames() {
    let field=Term::Function {parameters:vec![Parameter {sort:Sort::Runtime,ty:rigid(1,Sort::Type)}],
        result:Box::new(rigid(1,Sort::Type))};
    let record=nir::program::Struct {declaration:id(20),name:"Box".into(),type_parameters:vec![id(1)],
        fields:vec![("value".into(),field.clone())],requirements:vec![],capabilities:vec![]};
    let frames=vec![vec![Sort::Type]];
    let arguments=vec![bound(0,0,Sort::Type)];
    let actual=record.instantiate_fields_in_scope(&arguments,&frames).unwrap();
    let Term::Function {parameters,result}=&actual[0].1 else {panic!()};
    assert_eq!(parameters[0].ty,bound(1,0,Sort::Type));
    assert_eq!(**result,bound(1,0,Sort::Type));
    assert!(record.instantiate_fields(&arguments).is_err());
    assert!(record.instantiate_fields_in_scope(&arguments,&[vec![Sort::Runtime]]).is_err());
    assert!(record.instantiate_fields_in_scope(&[],&frames).is_err());
    let enumeration=nir::program::Enum {declaration:id(21),name:"Choice".into(),type_parameters:vec![id(1)],
        variants:vec![nir::program::Variant {name:"some".into(),shape:nir::program::VariantShape::Tuple,fields:vec![field]}],
        requirements:vec![],capabilities:vec![],encoding:None,symbol_encoding:None,groups:vec![]};
    assert_eq!(enumeration.instantiate_variants_in_scope(&arguments,&frames).unwrap()[0].fields[0],actual[0].1);
    assert!(enumeration.instantiate_variants(&arguments).is_err());
}
