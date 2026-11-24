use elaboration::ptc::{Budget, Context, Formula as F, Outcome, TypeKind as K};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn variable(local: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId { package: 9, local },
        sort: Sort::Type,
    }
}

fn prove(context: &Context, formula: F) {
    let outcome = context.prove(formula, Budget::default());
    let Outcome::Entailed(proof) = outcome else {
        panic!("expected proof: {outcome}")
    };
    assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
}

#[test]
fn closed_descriptors_have_overlapping_integer_classifications() {
    let fixtures = [
        (Term::Primitive(P::Bool), vec![K::Boolean]),
        (
            Term::Primitive(P::U32),
            vec![K::Integer, K::UnsignedInteger],
        ),
        (Term::Primitive(P::I64), vec![K::Integer, K::SignedInteger]),
        (Term::Primitive(P::I128), vec![K::Integer, K::SignedInteger]),
        (
            Term::Primitive(P::U128),
            vec![K::Integer, K::UnsignedInteger],
        ),
        (Term::Primitive(P::F64), vec![K::FloatingPoint]),
        (Term::Primitive(P::Char), vec![]),
        (Term::Tuple(vec![Term::Primitive(P::U32)]), vec![K::Tuple]),
        (
            Term::Pointer(Box::new(Term::Primitive(P::Bool))),
            vec![K::Pointer],
        ),
        (
            Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Void)),
            },
            vec![K::Function],
        ),
        (
            Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Void)),
            }
            .with_comptime_phase(true),
            vec![K::Function],
        ),
    ];
    for (ty, expected) in fixtures {
        for kind in [
            K::Boolean,
            K::Integer,
            K::UnsignedInteger,
            K::SignedInteger,
            K::FloatingPoint,
            K::Tuple,
            K::Pointer,
            K::Function,
        ] {
            let query = F::kind(ty.clone(), kind);
            prove(
                &Context::default(),
                if expected.contains(&kind) {
                    query
                } else {
                    query.negate()
                },
            );
        }
    }
}

#[test]
fn rigid_classification_is_propagated_through_equality_and_structure() {
    let (t, u) = (variable(0), variable(1));
    let mut context = Context::default();
    context.assume(F::equal(t.clone(), u.clone()));
    context.assume(F::kind(t.clone(), K::UnsignedInteger));
    prove(&context, F::kind(u.clone(), K::Integer));
    prove(&context, F::kind(u.clone(), K::SignedInteger).negate());
    context.assume(F::equal(
        Term::Pointer(Box::new(u)),
        Term::Pointer(Box::new(Term::Primitive(P::Bool))),
    ));
    prove(&context, F::False);
}

#[test]
fn closed_description_can_refute_an_incompatible_symbolic_classification() {
    let mut context = Context::default();
    let t = variable(1);
    context.assume(F::kind(t.clone(), K::Tuple));
    prove(&context, F::equal(t, Term::Primitive(P::U32)).negate());
}

#[test]
fn concrete_type_witnesses_do_not_turn_not_entailed_into_negation() {
    let context = Context::default();
    let t = variable(0);
    for query in [
        F::kind(t.clone(), K::Integer),
        F::kind(t.clone(), K::Integer).negate(),
    ] {
        assert!(matches!(
            context.prove(query, Budget::default()),
            Outcome::NotEntailed(_)
        ));
    }
    let mut integers = context.fork();
    integers.assume(F::kind(t.clone(), K::UnsignedInteger));
    let Outcome::NotEntailed(witness) = integers.prove(F::kind(t, K::Tuple), Budget::default())
    else {
        panic!("expected actual unsigned type witness")
    };
    assert_eq!(
        witness.types.values().next(),
        Some(&Term::Primitive(P::U32))
    );
    assert!(format!("{}", Outcome::NotEntailed(witness)).contains("(type-kind"));
}

#[test]
fn nominal_and_factory_descriptors_remain_unknown() {
    let id = DeclarationId {
        package: 1,
        local: 2,
    };
    for ty in [
        Term::Nominal {
            declaration: id,
            arguments: vec![],
        },
        Term::NormalForm {
            head: id,
            sort: Sort::Type,
            arguments: vec![Term::Primitive(P::U32)],
        },
    ] {
        for query in [
            F::kind(ty.clone(), K::Integer),
            F::kind(ty.clone(), K::Integer).negate(),
        ] {
            assert!(matches!(
                Context::default().prove(query, Budget::default()),
                Outcome::Unknown { .. }
            ));
        }
    }
}

#[test]
fn contradictory_kinds_and_branch_facts_are_scoped() {
    let t = variable(0);
    let mut context = Context::default();
    context.assume(F::Or(vec![
        F::kind(t.clone(), K::Tuple),
        F::kind(t.clone(), K::UnsignedInteger),
    ]));
    context.assume(F::kind(t.clone(), K::Integer).negate());
    prove(&context, F::kind(t.clone(), K::Tuple));
    let mut child = context.fork();
    child.assume(F::kind(t.clone(), K::Pointer));
    prove(&child, F::False);
    assert!(matches!(
        context.prove(F::False, Budget::default()),
        Outcome::NotEntailed(_)
    ));
    prove(&context, F::kind(t, K::Tuple));
}

#[test]
fn malformed_classification_inputs_are_rejected_before_tautology() {
    for ty in [
        Term::Bool(true),
        Term::Hole {
            index: 0,
            sort: Sort::Type,
        },
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ] {
        assert!(matches!(
            Context::default().prove(
                F::Or(vec![F::True, F::kind(ty, K::Integer)]),
                Budget::default()
            ),
            Outcome::IllFormed(_)
        ));
    }
}

#[test]
fn char_record_union_and_module_kinds_are_classified_and_disjoint() {
    let record_ty = Term::Record(vec![
        ("a".into(), Term::Primitive(P::Bool)),
        ("b".into(), Term::Primitive(P::U32)),
    ]);
    let fixtures = [
        (Term::Primitive(P::Char), vec![K::Char]),
        (record_ty.clone(), vec![K::Record]),
        (Term::Primitive(P::Bool), vec![K::Boolean]),
        (Term::Tuple(vec![Term::Primitive(P::U8)]), vec![K::Tuple]),
    ];
    let kinds = [
        K::Boolean,
        K::Char,
        K::Symbol,
        K::Integer,
        K::FloatingPoint,
        K::Record,
        K::Union,
        K::Module,
        K::Tuple,
        K::Pointer,
        K::Function,
    ];
    for (ty, expected) in fixtures {
        for kind in kinds {
            let query = F::kind(ty.clone(), kind);
            prove(
                &Context::default(),
                if expected.contains(&kind) {
                    query
                } else {
                    query.negate()
                },
            );
        }
    }
    // Display S-expression names for the new TypeKind variants.
    assert_eq!(K::Char.to_string(), "char");
    assert_eq!(K::Record.to_string(), "record");
    assert_eq!(K::Union.to_string(), "union");
    assert_eq!(K::Module.to_string(), "module");

    // Distinct record countermodels for `T != U` under `is_record(T) and is_record(U)`.
    let (t, u) = (variable(10), variable(11));
    let mut ctx = Context::default();
    ctx.assume(F::kind(t.clone(), K::Record));
    ctx.assume(F::kind(u.clone(), K::Record));
    let Outcome::NotEntailed(witness) =
        ctx.prove(F::equal(t.clone(), u.clone()), Budget::default())
    else {
        panic!("expected distinct record witnesses")
    };
    assert_eq!(ctx.verify_witness(&witness, Budget::default()), Ok(()));
    assert_ne!(
        witness.types[&DeclarationId {
            package: 9,
            local: 10
        }],
        witness.types[&DeclarationId {
            package: 9,
            local: 11
        }]
    );

    // Union and Module remain mutually exclusive with other kinds, and open inputs stay Unknown.
    let mut union_ctx = Context::default();
    union_ctx.assume(F::kind(t.clone(), K::Union));
    prove(&union_ctx, F::kind(t.clone(), K::Module).negate());
    prove(&union_ctx, F::kind(t.clone(), K::Record).negate());
    prove(&union_ctx, F::kind(t.clone(), K::Char).negate());
    assert!(matches!(
        Context::default().prove(F::kind(t, K::Union).negate(), Budget::default()),
        Outcome::Unknown { .. }
    ));
}

#[test]
fn not_sized_atom_respects_layout_domain_prerequisites_and_scoping() {
    use elaboration::ptc::{ProofError, UnknownReason};
    use nir::program::TypePredicate as TP;

    // Concrete runtime-representable fixed-size types satisfy `not is_not_sized`.
    let sized_types = [
        Term::Primitive(P::Bool),
        Term::Primitive(P::Char),
        Term::Primitive(P::U32),
        Term::Primitive(P::Isize),
        Term::Primitive(P::Str),
        Term::Primitive(P::Symbol),
        Term::Primitive(P::Void),
        Term::Pointer(Box::new(Term::Primitive(P::U8))),
        Term::Tuple(vec![Term::Primitive(P::U32), Term::Primitive(P::Bool)]),
        Term::Record(vec![("x".into(), Term::Primitive(P::F64))]),
        Term::Optional(Box::new(Term::Primitive(P::U64))),
        Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: Term::Primitive(P::U32),
            }],
            result: Box::new(Term::Primitive(P::Bool)),
        },
    ];
    for ty in sized_types {
        let pred = TP::Not(Box::new(TP::NotSized(ty.clone())));
        assert_eq!(pred.to_string(), format!("(not (not-sized {ty}))"));
        prove(&Context::default(), F::from_predicate(&pred));
    }

    // Comptime-only types must fail prerequisite validation BEFORE tautology!
    let comptime_only = [
        Term::Primitive(P::Type),
        Term::Primitive(P::Integer),
        Term::Primitive(P::Float),
        Term::Primitive(P::Object),
        Term::Primitive(P::Unit),
        Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(P::Bool)),
        })),
        Term::Lift(Box::new(Term::Primitive(P::U32))),
        Term::Tuple(vec![Term::Primitive(P::U32), Term::Primitive(P::Type)]),
        Term::Optional(Box::new(Term::Primitive(P::Integer))),
        Term::Record(vec![("bad".into(), Term::Primitive(P::Float))]),
    ];
    for ty in comptime_only {
        assert!(
            matches!(
                Context::default().prove(
                    F::Or(vec![F::True, F::not_sized(ty.clone())]),
                    Budget::default()
                ),
                Outcome::IllFormed(_)
            ),
            "expected IllFormed for comptime-only {ty}"
        );
        assert!(
            matches!(
                Context::default().prove(F::not_sized(ty.clone()).negate(), Budget::default()),
                Outcome::IllFormed(_)
            ),
            "expected IllFormed for negated comptime-only {ty}"
        );
    }

    // Unknown rigid parameter: `is_not_sized(T)` is NotEntailed (countermodel bool),
    // while `not is_not_sized(T)` is conservatively Unknown.
    let t = variable(20);
    let u = variable(21);
    let Outcome::NotEntailed(witness) =
        Context::default().prove(F::not_sized(t.clone()), Budget::default())
    else {
        panic!("expected countermodel for unconstrained is_not_sized(T)");
    };
    assert_eq!(
        Context::default().verify_witness(&witness, Budget::default()),
        Ok(())
    );
    assert!(matches!(
        Context::default().prove(F::not_sized(t.clone()).negate(), Budget::default()),
        Outcome::Unknown {
            reason: UnknownReason::UnvalidatedModel,
            ..
        }
    ));

    // Congruence, contradiction against fixed-size types, and fork isolation.
    let mut ctx = Context::default();
    ctx.assume(F::equal(t.clone(), u.clone()));
    ctx.assume(F::not_sized(t.clone()));
    prove(&ctx, F::not_sized(u.clone()));
    prove(&ctx, F::equal(u.clone(), Term::Primitive(P::U32)).negate());
    let mut child = ctx.fork();
    child.assume(F::equal(u.clone(), Term::Primitive(P::Bool)));
    prove(&child, F::False);
    assert!(!matches!(
        ctx.prove(F::False, Budget::default()),
        Outcome::Entailed(_)
    ));

    // Sized nominal contract participates in proof replay verification.
    let nom_id = DeclarationId {
        package: 50,
        local: 1,
    };
    let nom = Term::Nominal {
        declaration: nom_id,
        arguments: vec![],
    };
    let bare = Context::default();
    assert!(matches!(
        bare.prove(F::not_sized(nom.clone()).negate(), Budget::default()),
        Outcome::Unknown { .. }
    ));
    let mut sized_ctx = bare.fork();
    sized_ctx.admit_nominal_sized(nom_id);
    let Outcome::Entailed(proof) =
        sized_ctx.prove(F::not_sized(nom.clone()).negate(), Budget::default())
    else {
        panic!("expected proof for admitted sized nominal");
    };
    assert_eq!(sized_ctx.verify_proof(&proof, Budget::default()), Ok(()));
    assert_eq!(
        bare.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn not_sized_rejects_comptime_only_nominals_even_in_tautologies_and_compounds() {
    use elaboration::ptc::ProofError;
    use nir::program::{Package, PackedMode, Struct, TypePredicate as TP};

    let holder_id = DeclarationId {
        package: 60,
        local: 1,
    };
    let outer_id = DeclarationId {
        package: 60,
        local: 2,
    };
    let holder = Term::Nominal {
        declaration: holder_id,
        arguments: vec![],
    };
    let outer = Term::Nominal {
        declaration: outer_id,
        arguments: vec![],
    };

    // Place Outer before Holder in the package to verify order-independent fixed-point promotion.
    let mut package = Package::default();
    package.structs.push(Struct {
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: outer_id,
        name: "Outer".into(),
        fields: vec![("h".into(), holder.clone())],
        packed: PackedMode::None,
    });
    package.structs.push(Struct {
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: holder_id,
        name: "Holder".into(),
        fields: vec![("x".into(), Term::Primitive(P::Type))],
        packed: PackedMode::None,
    });

    let ctx = Context::from_package(&package).expect("package theory");
    let invalid_operands = [
        holder.clone(),
        outer.clone(),
        Term::Optional(Box::new(holder.clone())),
        Term::Tuple(vec![holder.clone(), Term::Primitive(P::U32)]),
        Term::Record(vec![("h".into(), outer.clone())]),
    ];
    for ty in invalid_operands {
        let tautology = F::Or(vec![
            F::not_sized(ty.clone()),
            F::not_sized(ty.clone()).negate(),
        ]);
        assert!(
            matches!(
                ctx.prove(tautology, Budget::default()),
                Outcome::IllFormed(_)
            ),
            "expected IllFormed for tautology over comptime-only nominal {ty}"
        );
        assert!(
            matches!(
                ctx.prove(
                    F::from_predicate(&TP::Not(Box::new(TP::NotSized(ty.clone())))),
                    Budget::default()
                ),
                Outcome::IllFormed(_)
            ),
            "expected IllFormed for negated NotSized over {ty}"
        );
    }

    // Proof replay must reject a proof generated under a different comptime_only_nominals set.
    let bare = Context::default();
    let Outcome::Entailed(proof) = ctx.prove(
        F::not_sized(Term::Primitive(P::U32)).negate(),
        Budget::default(),
    ) else {
        panic!("expected proof for u32");
    };
    assert_eq!(ctx.verify_proof(&proof, Budget::default()), Ok(()));
    assert_eq!(
        bare.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn malformed_proof_trees_return_invalid_instead_of_panicking() {
    use elaboration::ptc::{Atom, Equality, Proof, ProofError, ProofTree};
    use std::sync::Arc;

    let t = variable(30);
    let u = variable(31);
    let ctx = Context::default();
    let goal = F::equal(t.clone(), Term::Primitive(P::Bool));
    let unknown_atom = Atom::TypeEqual(Equality(u, Term::Primitive(P::U8)));

    let malformed_trees = [
        ProofTree::BooleanConflict,
        ProofTree::ClauseConflict(Arc::from([])),
        ProofTree::ClauseConflict(Arc::from([999_999usize])),
        ProofTree::TheoryConflict(Arc::from([(unknown_atom.clone(), true)])),
        ProofTree::Split {
            atom: Arc::new(unknown_atom.clone()),
            negative: Arc::new(ProofTree::BooleanConflict),
            positive: Arc::new(ProofTree::BooleanConflict),
        },
        ProofTree::Lemma {
            premises: Arc::from([(unknown_atom, false)]),
            proof: Arc::new(ProofTree::BooleanConflict),
        },
    ];
    for tree in malformed_trees {
        let proof = Proof {
            scope: vec![],
            assumptions: vec![],
            goal: goal.clone(),
            constructors: Default::default(),
            tree,
        };
        assert_eq!(
            ctx.verify_proof(&proof, Budget::default()),
            Err(ProofError::Invalid)
        );
    }
}
