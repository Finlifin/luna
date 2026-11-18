use super::*;
use nir::normal_forms::{Head, Kind, Parameter, Role};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 94, local }
}
const SOURCE: SourceRange = SourceRange { start: 10, end: 20 };

#[test]
fn active_description_checks_distinguish_limits_from_invalid_types() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let boolean = Term::Primitive(Primitive::Bool);
    let output = Term::Primitive(Primitive::Type);
    vm.budget.steps = 1;
    assert_eq!(
        vm.check_value_type(&Value::Type(boolean.clone()), &output, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    let wide = Term::Record(vec![("x".repeat(100_001), boolean.clone())]);
    vm.budget.steps = 1_000_000;
    assert_eq!(
        vm.type_constant(&wide, &output, SOURCE).unwrap_err(),
        Error {
            kind: ErrorKind::RepresentationSizeLimit,
            source: Some(SOURCE)
        }
    );
    assert_eq!(
        vm.remaining_steps(),
        999_999,
        "size failure must not retry selection"
    );
    let deep = (0..130).fold(boolean.clone(), |inner, _| Term::Optional(Box::new(inner)));
    assert_eq!(
        vm.check_type_description(&deep, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::DepthExhausted
    );
    let invalid = Term::Bound {
        depth: 0,
        index: 0,
        sort: nir::Sort::Type,
    };
    assert_eq!(
        vm.check_type_description(&invalid, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(
        vm.normal_pattern_value(&boolean, &output).unwrap(),
        Value::Type(boolean)
    );
    assert!(vm.events.is_empty());
}

fn value_indexed_head() -> (BTreeMap<DeclarationId, Head>, Term) {
    let domain = Term::Nominal {
        declaration: id(10),
        arguments: vec![],
    };
    let head = Head {
        declaration: id(1),
        role: Role::Opaque,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(2),
            binding: LocalId { owner: 1, index: 0 },
            kind: Kind::Value,
            ty: domain.clone(),
        }],
    };
    let ty = Term::NormalForm {
        head: id(1),
        sort: nir::Sort::Type,
        arguments: vec![Term::NominalValue {
            ty: Box::new(domain),
            variant: None,
            fields: vec![Term::Bool(true)],
        }],
    };
    (BTreeMap::from([(id(1), head)]), ty)
}

#[test]
fn nf_description_queries_preserve_typed_failures_with_positive_fuel() {
    let (owned_heads, ty) = value_indexed_head();
    let heads: BTreeMap<_, _> = owned_heads.iter().map(|(id, head)| (*id, head)).collect();
    for (failure, expected) in [
        (AdmissionError::Budget, ErrorKind::StepsExhausted),
        (
            AdmissionError::Unresolved("waiting for layout".into()),
            ErrorKind::TypeContractUnresolved,
        ),
        (
            AdmissionError::Invalid("bad layout".into()),
            ErrorKind::Type,
        ),
    ] {
        let mut fuel = 10_000;
        let calls = std::cell::Cell::new(0);
        let result = inspect(
            &ty,
            &[],
            &heads,
            &BTreeSet::new(),
            &BTreeMap::new(),
            |_, _, fuel| {
                calls.set(calls.get() + 1);
                *fuel -= 7;
                Err(failure.clone())
            },
            |_, _, _| Ok(None),
            None,
            &mut fuel,
        );
        assert_eq!(result, Err(expected));
        assert_eq!(calls.get(), 1);
        assert!(fuel > 0 && fuel < 10_000 - 7);
    }
    let mut fuel = 10_000;
    assert_eq!(
        inspect(
            &ty,
            &[],
            &heads,
            &BTreeSet::new(),
            &BTreeMap::new(),
            |_, _, _| Ok(vec![Term::Primitive(Primitive::Bool)]),
            |_, _, _| Ok(None),
            None,
            &mut fuel
        ),
        Ok(())
    );
}

#[test]
fn expensive_queries_do_not_change_the_structural_size_of_an_nf_description() {
    let (owned_heads, ty) = value_indexed_head();
    let heads: BTreeMap<_, _> = owned_heads.iter().map(|(id, head)| (*id, head)).collect();
    let mut fuel = 1_000_000;
    inspect(
        &ty,
        &[],
        &heads,
        &BTreeSet::new(),
        &BTreeMap::new(),
        |_, _, fuel| {
            *fuel = fuel.checked_sub(150_000).ok_or(AdmissionError::Budget)?;
            Ok(vec![Term::Primitive(Primitive::Bool)])
        },
        |_, _, _| Ok(None),
        None,
        &mut fuel,
    )
    .unwrap();
    assert!(1_000_000 - fuel > 150_000);
    let mut short = 100_000;
    assert_eq!(
        inspect(
            &ty,
            &[],
            &heads,
            &BTreeSet::new(),
            &BTreeMap::new(),
            |_, _, fuel| {
                *fuel = fuel.checked_sub(150_000).ok_or(AdmissionError::Budget)?;
                Ok(vec![Term::Primitive(Primitive::Bool)])
            },
            |_, _, _| Ok(None),
            None,
            &mut short
        ),
        Err(ErrorKind::StepsExhausted)
    );
    assert!(short > 0);
}

#[test]
fn active_nf_admission_preserves_failed_reservations_and_can_retry() {
    let descriptor = Head {
        declaration: id(1),
        role: Role::Opaque,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(2),
            binding: LocalId { owner: 1, index: 0 },
            kind: Kind::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
    };
    let package = Package {
        normal_forms: vec![descriptor],
        ..Package::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ty = Term::NormalForm {
        head: id(1),
        sort: nir::Sort::Type,
        arguments: vec![Term::Record(vec![(
            "field".repeat(400),
            Term::Primitive(Primitive::Bool),
        )])],
    };
    vm.budget.steps = 100;
    assert_eq!(
        vm.admit_type(&ty, 0).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 100);
    vm.budget.steps = 100_000;
    vm.admit_type(&ty, 0).unwrap();
    let invalid = Term::NormalForm {
        head: id(1),
        sort: nir::Sort::Type,
        arguments: vec![Term::Bool(true)],
    };
    assert_eq!(
        vm.admit_type(&invalid, 0).unwrap_err().kind,
        ErrorKind::Type
    );
    assert!(vm.events.is_empty());
}

#[test]
fn successive_dependent_domains_do_not_rescan_the_parameter_prefix() {
    let descriptor = Head {
        declaration: id(1),
        role: Role::Opaque,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(2),
            binding: LocalId { owner: 1, index: 0 },
            kind: Kind::Value,
            ty: Term::Primitive(Primitive::Bool),
        }],
    };
    let package = Package {
        normal_forms: vec![descriptor],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let measure = |vm: &mut Interpreter<'_>, width| {
        let mut parameters = Vec::new();
        for _ in 0..width {
            let index = parameters.len();
            parameters.push(nir::Parameter {
                sort: nir::Sort::ComptimeValue,
                ty: Term::Primitive(Primitive::Bool),
            });
            parameters.push(nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: Term::NormalForm {
                    head: id(1),
                    sort: nir::Sort::Type,
                    arguments: vec![Term::Bound {
                        depth: 0,
                        index,
                        sort: nir::Sort::ComptimeValue,
                    }],
                },
            });
        }
        let signature = Term::Function {
            parameters,
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let start = 1_000_000;
        vm.budget.steps = start;
        vm.check_type_description(&signature, None).unwrap();
        let inspected = start - vm.remaining_steps();
        vm.budget.steps = start;
        vm.admit_type(&signature, 0).unwrap();
        (inspected, start - vm.remaining_steps())
    };
    let (inspect, admit) = measure(&mut vm, 128);
    let (twice_inspect, twice_admit) = measure(&mut vm, 256);
    assert!(twice_inspect <= 2 * inspect, "{inspect} -> {twice_inspect}");
    assert!(twice_admit <= 2 * admit, "{admit} -> {twice_admit}");
}
