use nir::intrinsics::{Operation, Registry, Tag};
use nir::program::{
    Expr, ExprKind, Function, FunctionDeclaration, FunctionPhase, Package, SourceRange,
};
use nir::{DeclarationId, Primitive, Sort, Term};
use program_verification::memory::{bound_type, infer_in_package};

#[test]
fn memory_type_admission_checks_closure_contexts_and_record_names() {
    let deep = (0..64).fold(Term::Primitive(Primitive::Bool), |ty, _| {
        Term::Pointer(Box::new(ty))
    });
    let context = Term::ClosureContextValue(vec![deep]);
    assert!(
        bound_type(&context, &mut 10_000)
            .unwrap_err()
            .contains("budget")
    );
    let term = Term::Record(vec![("x".repeat(1000), Term::Primitive(Primitive::Bool))]);
    assert!(bound_type(&term, &mut 100).is_err());
    bound_type(&term, &mut 2000).unwrap();
}

#[test]
fn infer_in_package_validates_specialized_pending_and_payload_intrinsics() {
    let id = |local| DeclarationId { package: 1, local };
    let source = SourceRange { start: 0, end: 1 };
    let sqrt_tag = Tag {
        family: id(10),
        variant: 0,
    };
    let read_tag = Tag {
        family: id(11),
        variant: 0,
    };
    let intrinsic_enum = |decl, var_name: &str| nir::program::Enum {
        is_intrinsic: true,
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: decl,
        name: var_name.into(),
        variants: vec![nir::program::Variant {
            name: var_name.into(),
            shape: nir::program::VariantShape::Unit,
            fields: vec![],
        }],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
    };
    let sqrt_family = intrinsic_enum(sqrt_tag.family, "sqrt");
    let read_family = intrinsic_enum(read_tag.family, "raw_read");
    let sqrt_registry = Registry::for_family(&sqrt_family, 0, Operation::SquareRoot).unwrap();
    let read_registry = Registry::for_family(&read_family, 0, Operation::MemRawRead).unwrap();

    let sqrt_param = id(20);
    let sqrt_t = Term::Rigid {
        declaration: sqrt_param,
        sort: Sort::Type,
    };
    let deferred_sqrt = sqrt_registry
        .defer(sqrt_tag, std::slice::from_ref(&sqrt_t), &sqrt_t)
        .unwrap();

    let read_param = id(21);
    let read_t = Term::Rigid {
        declaration: read_param,
        sort: Sort::Type,
    };
    let usize_ty = Term::Primitive(Primitive::Usize);
    let deferred_read = read_registry
        .defer(read_tag, std::slice::from_ref(&usize_ty), &read_t)
        .unwrap();

    let sqrt_decl = id(100);
    let read_decl = id(101);
    let nominal_decl = id(200);
    let p0 = nir::program::LocalId { owner: 1, index: 0 };
    let p1 = nir::program::LocalId { owner: 2, index: 0 };

    let package = Package {
        enums: vec![sqrt_family, read_family],
        structs: vec![nir::program::Struct {
            declaration: nominal_decl,
            name: "Item".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            fields: vec![("v".into(), Term::Primitive(Primitive::U64))],
            requirements: vec![],
            capabilities: vec![],
            packed: nir::program::PackedMode::None,
        }],
        functions: vec![
            Function {
                interface: FunctionDeclaration {
                    source,
                    declaration: sqrt_decl,
                    name: "generic_sqrt".into(),
                    phase: FunctionPhase::Runtime,
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    type_parameters: vec![sqrt_param],
                    parameters: vec![p0],
                    env_parameters: vec![],
                    defaults: vec![],
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    signature: Term::Function {
                        parameters: vec![nir::Parameter {
                            sort: Sort::Runtime,
                            ty: sqrt_t.clone(),
                        }],
                        result: Box::new(sqrt_t.clone()),
                    },
                },
                body: Expr {
                    source,
                    ty: sqrt_t.clone(),
                    kind: ExprKind::Intrinsic {
                        tag: sqrt_tag,
                        inputs: vec![Expr {
                            source,
                            ty: sqrt_t,
                            kind: ExprKind::Local(p0),
                        }],
                        binding: Some(deferred_sqrt),
                    },
                },
                runtime_deliveries: Default::default(),
            },
            Function {
                interface: FunctionDeclaration {
                    source,
                    declaration: read_decl,
                    name: "generic_read".into(),
                    phase: FunctionPhase::Runtime,
                    inline: false,
                    pure: false,
                    is_unsafe: true,
                    type_parameters: vec![read_param],
                    parameters: vec![p1],
                    env_parameters: vec![],
                    defaults: vec![],
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    signature: Term::Function {
                        parameters: vec![nir::Parameter {
                            sort: Sort::Runtime,
                            ty: usize_ty.clone(),
                        }],
                        result: Box::new(read_t.clone()),
                    },
                },
                body: Expr {
                    source,
                    ty: read_t,
                    kind: ExprKind::Intrinsic {
                        tag: read_tag,
                        inputs: vec![Expr {
                            source,
                            ty: usize_ty,
                            kind: ExprKind::Local(p1),
                        }],
                        binding: Some(deferred_read),
                    },
                },
                runtime_deliveries: Default::default(),
            },
        ],
        ..Default::default()
    };

    let nominal_ty = Term::Nominal {
        declaration: nominal_decl,
        arguments: vec![],
    };
    infer_in_package(
        &package,
        read_decl,
        std::slice::from_ref(&nominal_ty),
        &mut 100_000,
    )
    .unwrap();
    infer_in_package(
        &package,
        sqrt_decl,
        &[Term::Primitive(Primitive::F64)],
        &mut 100_000,
    )
    .unwrap();

    // Unspecialized rigid payload or invalid scalar specialization must fail.
    assert!(
        infer_in_package(
            &package,
            sqrt_decl,
            &[Term::Primitive(Primitive::Bool)],
            &mut 100_000,
        )
        .is_err()
    );
    let rigid_foreign = Term::Rigid {
        declaration: id(999),
        sort: Sort::Type,
    };
    assert!(infer_in_package(&package, read_decl, &[rigid_foreign], &mut 100_000).is_err());
    assert!(infer_in_package(&package, read_decl, &[nominal_ty], &mut 0).is_err());
}
