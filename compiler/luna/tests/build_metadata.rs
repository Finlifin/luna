use luna::{
    build_context::metadata::{Entry, Error, Input, Provider, Snapshot, VERSION},
    packages::SourcePackage,
};
use nir::{DeclarationId, Primitive, Sort, Term};
use resolve::DependencyEdge;

fn id(package: u64) -> DeclarationId {
    DeclarationId { package, local: 7 }
}
fn sources() -> Vec<SourcePackage> {
    [(1, 100, "app"), (2, 200, "cuda"), (3, 300, "cuda")]
        .into_iter()
        .map(|(resolution_id, identity, name)| SourcePackage {
            resolution_id,
            identity,
            name: name.into(),
            entry: format!("{identity}/lib.fl").into(),
        })
        .collect()
}
fn edges() -> Vec<DependencyEdge> {
    vec![
        DependencyEdge {
            requester: 1,
            alias: "cuda".into(),
            provider: 2,
        },
        DependencyEdge {
            requester: 2,
            alias: "cuda".into(),
            provider: 3,
        },
    ]
}
fn boolean(value: bool) -> Entry {
    Entry::new(
        Term::Primitive(Primitive::Bool),
        Term::Bool(value),
        &mut 1000,
    )
    .unwrap()
}
fn input(owner: u64, value: bool) -> Input {
    Input {
        identity: owner,
        fields: vec![("enabled".into(), boolean(value))],
    }
}
fn snapshot(inputs: Vec<Input>) -> Snapshot {
    Snapshot::new(VERSION, &sources(), &edges(), inputs, &mut 10000).unwrap()
}
fn dep() -> Provider {
    Provider::Dependency("cuda".into())
}

#[test]
fn metadata_queries_use_source_identity_and_direct_dependency_aliases() {
    let snapshot = snapshot(vec![input(100, false), input(200, true), input(300, false)]);
    let app = snapshot.read(id(100), dep(), "enabled").unwrap();
    assert_eq!(app.owner(), 100);
    assert_eq!(app.provider(), 200);
    assert_eq!(app.value(), Some(&boolean(true)));
    assert_eq!(
        snapshot.read(id(200), dep(), "enabled").unwrap().provider(),
        300
    );
    assert_eq!(
        snapshot
            .read(id(200), Provider::Local, "enabled")
            .unwrap()
            .value(),
        Some(&boolean(true))
    );
    assert_eq!(
        snapshot.read(id(1), dep(), "enabled"),
        Err(Error::MissingOwner(1))
    );
    assert!(matches!(
        snapshot.read(id(300), dep(), "enabled"),
        Err(Error::MissingDependency { owner: 300, .. })
    ));
}

#[test]
fn absence_false_missing_input_and_missing_dependency_are_distinct() {
    let snapshot = snapshot(vec![input(200, false)]);
    assert_eq!(
        snapshot.read(id(100), dep(), "enabled").unwrap().value(),
        Some(&boolean(false))
    );
    assert_eq!(
        snapshot.read(id(100), dep(), "absent").unwrap().value(),
        None
    );
    assert_eq!(
        snapshot.read(id(100), Provider::Local, "enabled"),
        Err(Error::MissingInput(100))
    );
    assert!(matches!(
        snapshot.read(id(100), Provider::Dependency("missing".into()), "enabled"),
        Err(Error::MissingDependency { .. })
    ));
    let empty = snapshot_with_empty();
    assert!(
        empty
            .read(id(100), dep(), "enabled")
            .unwrap()
            .value()
            .is_none()
    );
}
fn snapshot_with_empty() -> Snapshot {
    snapshot(vec![Input {
        identity: 200,
        fields: vec![],
    }])
}

#[test]
fn observations_track_field_presence_value_type_and_provider_identity() {
    let old = snapshot(vec![input(200, true)]);
    let present = old.read(id(100), dep(), "enabled").unwrap();
    let absent = old.read(id(100), dep(), "other").unwrap();
    let mut changed = input(200, true);
    changed.fields.push(("other".into(), boolean(false)));
    let new = snapshot(vec![changed]);
    assert!(new.unchanged(&present));
    assert!(!new.unchanged(&absent));
    assert!(!snapshot(vec![input(200, false)]).unchanged(&present));
    assert!(!snapshot_with_empty().unchanged(&present));
    assert!(!snapshot(vec![]).unchanged(&present));
    let changed_type = Entry::new(
        Term::Primitive(Primitive::Type),
        Term::Primitive(Primitive::Bool),
        &mut 1000,
    )
    .unwrap();
    assert!(
        !snapshot(vec![Input {
            identity: 200,
            fields: vec![("enabled".into(), changed_type)]
        }])
        .unchanged(&present)
    );
    let mut remapped = edges();
    remapped[0].provider = 3;
    let remapped = Snapshot::new(
        VERSION,
        &sources(),
        &remapped,
        vec![input(300, true)],
        &mut 10000,
    )
    .unwrap();
    assert!(!remapped.unchanged(&present));
    assert!(old.unchanged(&present));
    assert!(old.clone().unchanged(&present));
}

#[test]
fn metadata_preserves_nominal_values_and_opaque_normal_form_heads() {
    let ty = Term::Nominal {
        declaration: id(200),
        arguments: vec![],
    };
    let value = Term::NominalValue {
        ty: Box::new(ty.clone()),
        variant: Some(1),
        fields: vec![],
    };
    let entry = Entry::new(ty.clone(), value.clone(), &mut 1000).unwrap();
    let opaque = Term::NormalForm {
        head: id(300),
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let opaque_entry =
        Entry::new(Term::Primitive(Primitive::Type), opaque.clone(), &mut 1000).unwrap();
    let snapshot = snapshot(vec![Input {
        identity: 200,
        fields: vec![("arch".into(), entry), ("opaque".into(), opaque_entry)],
    }]);
    let read = snapshot.read(id(100), dep(), "arch").unwrap();
    assert_eq!(read.value().unwrap().ty(), &ty);
    assert_eq!(read.value().unwrap().value(), &value);
    assert_eq!(
        snapshot
            .read(id(100), dep(), "opaque")
            .unwrap()
            .value()
            .unwrap()
            .value(),
        &opaque
    );
    // This driver boundary intentionally does not assert nominal membership or
    // prove semantic type equality without the provider's compilation world.
}

#[test]
fn metadata_entries_reject_open_terms_and_account_for_budget() {
    for ty in [
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        Term::Hole {
            index: 0,
            sort: Sort::Type,
        },
        Term::Rigid {
            declaration: id(100),
            sort: Sort::Type,
        },
        Term::Bool(true),
    ] {
        assert_eq!(
            Entry::new(ty, Term::Bool(true), &mut 1000),
            Err(Error::OpenEntry)
        );
    }
    let nested = Term::TupleValue(vec![Term::Rigid {
        declaration: id(100),
        sort: Sort::ComptimeValue,
    }]);
    assert_eq!(
        Entry::new(
            Term::Tuple(vec![Term::Primitive(Primitive::Bool)]),
            nested,
            &mut 1000
        ),
        Err(Error::OpenEntry)
    );
    assert_eq!(
        Entry::new(Term::Primitive(Primitive::Bool), Term::Bool(true), &mut 0),
        Err(Error::Budget)
    );
    let mut fuel = 1000;
    Entry::new(
        Term::Primitive(Primitive::Bool),
        Term::Bool(true),
        &mut fuel,
    )
    .unwrap();
    assert!(fuel < 1000);
}

#[test]
fn metadata_inputs_reject_ambiguous_graphs_fields_and_versions() {
    assert!(matches!(
        Snapshot::new(2, &sources(), &edges(), vec![], &mut 1000),
        Err(Error::Version(2))
    ));
    let mut duplicate = sources();
    duplicate[1].identity = 100;
    assert!(matches!(
        Snapshot::new(VERSION, &duplicate, &edges(), vec![], &mut 1000),
        Err(Error::DuplicateIdentity(100))
    ));
    let mut duplicate = sources();
    duplicate[1].resolution_id = 1;
    assert!(matches!(
        Snapshot::new(VERSION, &duplicate, &[], vec![], &mut 1000),
        Err(Error::Graph(_))
    ));
    for edges in [
        vec![DependencyEdge {
            requester: 1,
            alias: "cuda".into(),
            provider: 99,
        }],
        vec![edges()[0].clone(), edges()[0].clone()],
    ] {
        assert!(matches!(
            Snapshot::new(VERSION, &sources(), &edges, vec![], &mut 1000),
            Err(Error::Graph(_))
        ));
    }
    assert!(matches!(
        Snapshot::new(
            VERSION,
            &sources(),
            &edges(),
            vec![input(999, true)],
            &mut 1000
        ),
        Err(Error::UnknownInput(999))
    ));
    assert!(matches!(
        Snapshot::new(
            VERSION,
            &sources(),
            &edges(),
            vec![input(200, true), input(200, false)],
            &mut 1000
        ),
        Err(Error::DuplicateInput(200))
    ));
    let mut duplicate = input(200, true);
    duplicate.fields.push(("enabled".into(), boolean(false)));
    assert!(matches!(
        Snapshot::new(VERSION, &sources(), &edges(), vec![duplicate], &mut 1000),
        Err(Error::DuplicateField { owner: 200, .. })
    ));
    assert!(matches!(
        Snapshot::new(
            VERSION,
            &sources(),
            &edges(),
            vec![Input {
                identity: 200,
                fields: vec![("".into(), boolean(true))]
            }],
            &mut 1000
        ),
        Err(Error::EmptyField(200))
    ));
    assert!(matches!(
        Snapshot::new(VERSION, &sources(), &edges(), vec![], &mut 0),
        Err(Error::Budget)
    ));
    // Failure owns only its candidate; an independently retained snapshot survives.
    assert!(
        snapshot(vec![input(200, true)])
            .read(id(100), dep(), "enabled")
            .is_ok()
    );
}

#[test]
fn metadata_dump_is_canonical_and_escaped() {
    let fields = vec![
        ("z".into(), boolean(false)),
        ("line\n\"".into(), boolean(true)),
    ];
    let first = snapshot(vec![Input {
        identity: 200,
        fields: fields.clone(),
    }]);
    let mut reversed = fields;
    reversed.reverse();
    let mut sources = sources();
    sources.reverse();
    let mut edges = edges();
    edges.reverse();
    let second = Snapshot::new(
        VERSION,
        &sources,
        &edges,
        vec![Input {
            identity: 200,
            fields: reversed,
        }],
        &mut 10000,
    )
    .unwrap();
    assert_eq!(first.to_string(), second.to_string());
    let dump = first.to_string();
    assert!(dump.contains("line\\n\\\""));
    assert!(!dump.contains('\n'));
    assert_eq!(
        first.read(id(100), dep(), "missing").unwrap().to_string(),
        "(metadata-read (owner 100) (provider (dependency \"cuda\") 200) (field \"missing\") (value absent))"
    );
}

#[test]
fn explicit_null_metadata_is_not_an_absent_field() {
    let optional = Term::Optional(Box::new(Term::Primitive(Primitive::Bool)));
    let null = Entry::new(
        optional.clone(),
        Term::OptionalValue {
            ty: Box::new(optional),
            value: None,
        },
        &mut 1000,
    )
    .unwrap();
    let snapshot = snapshot(vec![Input {
        identity: 200,
        fields: vec![("enabled".into(), null.clone())],
    }]);
    assert_eq!(
        snapshot.read(id(100), dep(), "enabled").unwrap().value(),
        Some(&null)
    );
    assert!(
        snapshot
            .read(id(100), dep(), "absent")
            .unwrap()
            .value()
            .is_none()
    );
    assert!(!snapshot_with_empty().unchanged(&snapshot.read(id(100), dep(), "enabled").unwrap()));
}

#[test]
fn metadata_rejects_runtime_dependencies_inside_types_and_values() {
    let runtime = Term::Runtime(nir::RuntimeId {
        instance: 1,
        local: 2,
    });
    let dependent = Term::Nominal {
        declaration: id(200),
        arguments: vec![runtime.clone()],
    };
    assert_eq!(
        Entry::new(dependent, Term::Bool(true), &mut 1000),
        Err(Error::OpenEntry)
    );
    assert_eq!(
        Entry::new(Term::Primitive(Primitive::Bool), runtime, &mut 1000),
        Err(Error::OpenEntry)
    );
}

fn metadata_world() -> nir::program::Package {
    nir::program::Package {
        enums: vec![nir::program::Enum {
            declaration: id(200),
            name: "TargetArch".into(),
            is_intrinsic: false,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            variants: vec![nir::program::Variant {
                name: "gpu".into(),
                shape: nir::program::VariantShape::Named(vec!["enabled".into()]),
                fields: vec![Term::Primitive(Primitive::Bool)],
            }],
        }],
        ..Default::default()
    }
}
#[test]
fn metadata_admission_checks_nominal_world_payload_and_variant_before_use() {
    let world = metadata_world();
    let ty = Term::Nominal {
        declaration: id(200),
        arguments: vec![],
    };
    let entry = |index, payload| {
        Entry::new(
            ty.clone(),
            Term::NominalValue {
                ty: Box::new(ty.clone()),
                variant: Some(index),
                fields: vec![payload],
            },
            &mut 1000,
        )
        .unwrap()
    };
    let valid = entry(0, Term::Bool(true));
    let snapshot = snapshot(vec![Input {
        identity: 200,
        fields: vec![
            ("arch".into(), valid.clone()),
            ("bad_index".into(), entry(1, Term::Bool(true))),
            ("bad_payload".into(), entry(0, Term::Symbol("wrong".into()))),
        ],
    }]);
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &world,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    for field in ["bad_index", "bad_payload"] {
        let error = snapshot
            .read(id(100), dep(), field)
            .unwrap()
            .admit(&mut vm)
            .unwrap_err();
        assert_eq!(
            (error.owner, error.provider, error.field.as_str()),
            (100, 200, field)
        );
        assert!(error.to_string().contains("metadata-admission-error"));
    }
    assert!(matches!(
        snapshot
            .read(id(100), dep(), "arch")
            .unwrap()
            .admit(&mut vm)
            .unwrap(),
        Some(comptime_interpretor::Value::Variant { index: 0, .. })
    ));
    assert_eq!(
        snapshot
            .read(id(100), dep(), "absent")
            .unwrap()
            .admit(&mut vm)
            .unwrap(),
        None
    );
    assert!(vm.events().is_empty());
    // An observation is not an admission certificate for a different world.
    let absent_world = nir::program::Package::default();
    let mut foreign = comptime_interpretor::Interpreter::for_comptime(
        &absent_world,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    assert!(valid.admit(&mut foreign).is_err());
}
#[test]
fn metadata_admission_rechecks_nominal_formation_requirements() {
    let mut world = metadata_world();
    world.enums[0]
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    let ty = Term::Nominal {
        declaration: id(200),
        arguments: vec![],
    };
    let entry = Entry::new(Term::Primitive(Primitive::Type), ty, &mut 1000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &world,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    assert!(entry.admit(&mut vm).is_err());
    assert!(vm.events().is_empty());
}

#[test]
fn metadata_objects_validate_stored_domains_without_erasing_types() {
    let world = metadata_world();
    let object = Term::Primitive(Primitive::Object);
    let ty = Term::Nominal {
        declaration: id(200),
        arguments: vec![],
    };
    let value = Term::NominalValue {
        ty: Box::new(ty.clone()),
        variant: Some(0),
        fields: vec![Term::Bool(true)],
    };
    let valid = Entry::new(
        object.clone(),
        nir::objects::value(vec![nir::objects::entry(Some("arch"), ty, value)]),
        &mut 10000,
    )
    .unwrap();
    let invalid = Entry::new(
        object,
        nir::objects::value(vec![nir::objects::entry(
            Some("enabled"),
            Term::Primitive(Primitive::Bool),
            Term::Symbol("wrong".into()),
        )]),
        &mut 10000,
    )
    .unwrap();
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &world,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    assert!(invalid.admit(&mut vm).is_err());
    assert!(matches!(
        valid.admit(&mut vm).unwrap(),
        comptime_interpretor::Value::Object(_)
    ));
    assert!(vm.events().is_empty());
}
