use nir::{
    Primitive as P, Term,
    program::{Expr, ExprKind as K, Literal, LocalId, SourceRange, checkable_predicate},
};

fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 0, end: 1 },
        ty,
        kind,
    }
}
fn boolean() -> Term {
    Term::Primitive(P::Bool)
}

#[test]
fn associated_predicate_operands_do_not_admit_callable_invocation() {
    let id = nir::DeclarationId {
        package: 1,
        local: 1,
    };
    let value = expr(
        boolean(),
        K::AssociatedValue {
            interface: nir::program::InterfaceInstance {
                declaration: id,
                arguments: vec![],
            },
            member: nir::DeclarationId {
                package: 1,
                local: 2,
            },
            subject: boolean(),
        },
    );
    assert!(checkable_predicate(&value));
    assert!(checkable_predicate(&expr(
        boolean(),
        K::Not(Box::new(value.clone()))
    )));
    let old = LocalId { owner: 7, index: 0 };
    let new = LocalId { owner: 8, index: 3 };
    let mut predicate = expr(
        boolean(),
        K::And(
            Box::new(expr(boolean(), K::Local(old))),
            Box::new(value.clone()),
        ),
    );
    nir::contracts::remap_predicate(&mut predicate, &[(old, new)].into_iter().collect()).unwrap();
    let K::And(local, associated) = &predicate.kind else {
        panic!("expected conjunction")
    };
    assert!(matches!(local.kind, K::Local(id) if id == new));
    assert_eq!(**associated, value);
    assert!(checkable_predicate(&predicate));
    let callable = Expr {
        ty: Term::Function {
            parameters: vec![],
            result: Box::new(boolean()),
        },
        ..value
    };
    assert!(!checkable_predicate(&callable));
    assert!(!checkable_predicate(&expr(
        boolean(),
        K::Call {
            callee: Box::new(callable),
            arguments: vec![],
            parameter_bindings: vec![],
        }
    )));
}
fn pointer() -> Expr {
    expr(
        Term::Pointer(Box::new(boolean())),
        K::Local(LocalId { owner: 0, index: 0 }),
    )
}

#[test]
fn predicate_shape_admits_typed_reads_without_proving_pointer_validity() {
    let read = expr(boolean(), K::ReadPointer(Box::new(pointer())));
    assert!(checkable_predicate(&read));
    let local = expr(boolean(), K::Local(LocalId { owner: 0, index: 1 }));
    let address = expr(
        Term::Pointer(Box::new(boolean())),
        K::AddressOf(Box::new(local)),
    );
    assert!(checkable_predicate(&expr(
        boolean(),
        K::ReadPointer(Box::new(address))
    )));
    let mut malformed = read;
    let K::ReadPointer(pointer) = &mut malformed.kind else {
        unreachable!()
    };
    pointer.ty = Term::Pointer(Box::new(Term::Primitive(P::U8)));
    assert!(!checkable_predicate(&malformed));
}

#[test]
fn predicate_shape_rejects_writes_and_owned_pointer_delivery() {
    let yes = expr(boolean(), K::Literal(Literal::Bool(true)));
    let write = expr(
        Term::Primitive(P::Void),
        K::WritePointer {
            pointer: Box::new(pointer()),
            value: Box::new(yes.clone()),
        },
    );
    assert!(!checkable_predicate(&expr(
        boolean(),
        K::Block {
            statements: vec![write],
            result: Some(Box::new(yes))
        }
    )));
    let owned = Term::Record(vec![("flag".into(), boolean())]);
    let pointer = expr(
        Term::Pointer(Box::new(owned.clone())),
        K::Local(LocalId { owner: 0, index: 0 }),
    );
    let value = expr(owned, K::ReadPointer(Box::new(pointer)));
    assert!(!checkable_predicate(&expr(
        boolean(),
        K::AnonymousField {
            value: Box::new(value),
            name: "flag".into(),
            index: 0
        }
    )));
}

#[test]
fn predicate_substitution_rebinds_read_addresses_and_field_pointers() {
    let original = LocalId { owner: 0, index: 0 };
    let target = LocalId { owner: 9, index: 3 };
    let bindings = std::collections::BTreeMap::from([(original, target)]);
    let record = Term::Record(vec![("flag".into(), boolean())]);
    let record_pointer = expr(Term::Pointer(Box::new(record)), K::Local(original));
    let field = expr(
        Term::Pointer(Box::new(boolean())),
        K::PointerAnonymousField {
            pointer: Box::new(record_pointer),
            name: "flag".into(),
            index: 0,
        },
    );
    let address = expr(
        Term::Pointer(Box::new(boolean())),
        K::AddressOf(Box::new(expr(boolean(), K::Local(original)))),
    );
    for pointer in [pointer(), address, field] {
        let mut predicate = expr(boolean(), K::ReadPointer(Box::new(pointer)));
        assert!(checkable_predicate(&predicate));
        nir::contracts::remap_predicate(&mut predicate, &bindings).unwrap();
        let mut locals = Vec::new();
        nir::visit::expression(&predicate, &mut 100, |expr, _| {
            if let K::Local(id) = expr.kind {
                locals.push(id);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(locals, vec![target]);
        assert!(checkable_predicate(&predicate));
    }
    let mut write = expr(
        Term::Primitive(P::Void),
        K::WritePointer {
            pointer: Box::new(pointer()),
            value: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
        },
    );
    assert!(nir::contracts::remap_predicate(&mut write, &bindings).is_err());
}

#[test]
fn predicate_destructuring_rebinds_pure_patterns_and_rejects_writes() {
    use nir::program::Pattern;
    let input = LocalId { owner: 1, index: 0 };
    let field = LocalId { owner: 1, index: 1 };
    let mapped_input = LocalId { owner: 2, index: 0 };
    let mapped_field = LocalId { owner: 2, index: 1 };
    let binding = Pattern::Tuple(vec![Pattern::Binding {
        binding: field,
        inner: Box::new(Pattern::Wild),
    }]);
    let make = |pattern| {
        expr(
            boolean(),
            K::Block {
                statements: vec![expr(
                    Term::Primitive(P::Void),
                    K::LetPattern {
                        pattern,
                        value: Box::new(expr(Term::Tuple(vec![boolean()]), K::Local(input))),
                        guard: None,
                        otherwise: None,
                    },
                )],
                result: Some(Box::new(expr(boolean(), K::Local(field)))),
            },
        )
    };
    let mut predicate = make(binding.clone());
    assert!(checkable_predicate(&predicate));
    let mapping = std::collections::BTreeMap::from([(input, mapped_input), (field, mapped_field)]);
    nir::contracts::remap_predicate(&mut predicate, &mapping).unwrap();
    let K::Block { statements, result } = &predicate.kind else {
        panic!("block")
    };
    let K::LetPattern {
        pattern: Pattern::Tuple(fields),
        value,
        ..
    } = &statements[0].kind
    else {
        panic!("binding")
    };
    assert!(matches!(fields[0], Pattern::Binding { binding, .. } if binding == mapped_field));
    assert!(matches!(value.kind, K::Local(id) if id == mapped_input));
    assert!(matches!(result.as_ref().unwrap().kind, K::Local(id) if id == mapped_field));
    assert!(checkable_predicate(&predicate));
    for pattern in [
        Pattern::Guard {
            pattern: Box::new(binding.clone()),
            condition: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
        },
        Pattern::Then {
            first: Box::new(binding.clone()),
            value: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
            next: Box::new(Pattern::Wild),
        },
    ] {
        let mut predicate = make(pattern);
        assert!(checkable_predicate(&predicate));
        nir::contracts::remap_predicate(&mut predicate, &mapping).unwrap();
    }
    let mut invalid = make(Pattern::Then {
        first: Box::new(binding),
        value: Box::new(expr(
            Term::Primitive(P::Void),
            K::WritePointer {
                pointer: Box::new(pointer()),
                value: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
            },
        )),
        next: Box::new(Pattern::Wild),
    });
    assert!(!checkable_predicate(&invalid));
    assert!(nir::contracts::remap_predicate(&mut invalid, &mapping).is_err());
}

#[test]
fn predicate_match_rebinds_pattern_guards_and_rejects_guard_writes() {
    use nir::program::{MatchArm, Pattern};
    let input = LocalId { owner: 1, index: 0 };
    let field = LocalId { owner: 1, index: 1 };
    let mapped = LocalId { owner: 2, index: 1 };
    let make = |condition| {
        expr(
            boolean(),
            K::Match {
                value: Box::new(expr(boolean(), K::Local(input))),
                arms: vec![MatchArm {
                    pattern: Pattern::Guard {
                        pattern: Box::new(Pattern::Binding {
                            binding: field,
                            inner: Box::new(Pattern::Wild),
                        }),
                        condition: Box::new(condition),
                    },
                    guard: Some(expr(boolean(), K::Local(field))),
                    body: expr(boolean(), K::Local(field)),
                }],
            },
        )
    };
    let mut predicate = make(expr(boolean(), K::Local(field)));
    assert!(checkable_predicate(&predicate));
    let mapping = std::collections::BTreeMap::from([(field, mapped)]);
    nir::contracts::remap_predicate(&mut predicate, &mapping).unwrap();
    let K::Match { arms, .. } = &predicate.kind else {
        panic!("match")
    };
    let Pattern::Guard { pattern, condition } = &arms[0].pattern else {
        panic!("guard")
    };
    assert!(matches!(pattern.as_ref(),Pattern::Binding {binding,..} if *binding==mapped));
    for expr in [
        condition.as_ref(),
        arms[0].guard.as_ref().unwrap(),
        &arms[0].body,
    ] {
        assert!(matches!(expr.kind,K::Local(id) if id==mapped));
    }
    let mut invalid = make(expr(
        boolean(),
        K::Block {
            statements: vec![expr(
                Term::Primitive(P::Void),
                K::WritePointer {
                    pointer: Box::new(pointer()),
                    value: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
                },
            )],
            result: Some(Box::new(expr(boolean(), K::Literal(Literal::Bool(true))))),
        },
    ));
    assert!(!checkable_predicate(&invalid));
    assert!(nir::contracts::remap_predicate(&mut invalid, &mapping).is_err());
}

#[test]
fn predicate_or_patterns_remap_candidate_and_exported_bindings_once() {
    use nir::program::{MatchArm, Pattern};
    let candidate = LocalId { owner: 1, index: 1 };
    let exported = LocalId { owner: 1, index: 2 };
    let target = LocalId { owner: 2, index: 1 };
    let mut predicate = expr(
        boolean(),
        K::Match {
            value: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
            arms: vec![MatchArm {
                pattern: Pattern::Or(vec![Pattern::Remap {
                    pattern: Box::new(Pattern::Binding {
                        binding: candidate,
                        inner: Box::new(Pattern::Wild),
                    }),
                    bindings: vec![(candidate, exported)],
                }]),
                guard: Some(expr(boolean(), K::Local(exported))),
                body: expr(boolean(), K::Local(exported)),
            }],
        },
    );
    assert!(checkable_predicate(&predicate));
    let mapping = std::collections::BTreeMap::from([(candidate, exported), (exported, target)]);
    nir::contracts::remap_predicate(&mut predicate, &mapping).unwrap();
    let K::Match { arms, .. } = &predicate.kind else {
        panic!("match")
    };
    let Pattern::Or(alternatives) = &arms[0].pattern else {
        panic!("or")
    };
    let Pattern::Remap { pattern, bindings } = &alternatives[0] else {
        panic!("remap")
    };
    assert_eq!(bindings, &vec![(exported, target)]);
    assert!(matches!(pattern.as_ref(),Pattern::Binding {binding,..} if *binding==exported));
    assert!(matches!(arms[0].body.kind,K::Local(binding) if binding==target));
    assert!(matches!(arms[0].guard.as_ref().unwrap().kind,K::Local(binding) if binding==target));
    assert!(checkable_predicate(&predicate));
}

#[test]
fn captured_predicate_data_preserves_identity_and_rejects_malformed_payloads() {
    use std::collections::BTreeMap;
    let captured = expr(boolean(), K::ComptimeValue(Term::Bool(true)));
    assert!(checkable_predicate(&captured));
    let mut remapped = captured.clone();
    nir::contracts::remap_predicate(&mut remapped, &BTreeMap::new()).unwrap();
    assert_eq!(captured, remapped);
    for payload in [
        Term::Char('x'),
        Term::Primitive(P::Bool),
        Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::ComptimeValue,
        },
    ] {
        let mut invalid = expr(boolean(), K::ComptimeValue(payload));
        assert!(!checkable_predicate(&invalid));
        assert!(nir::contracts::remap_predicate(&mut invalid, &BTreeMap::new()).is_err());
    }
}

#[test]
fn reconstructed_aggregate_predicates_reject_effectful_fields() {
    let declaration = nir::DeclarationId {
        package: 1,
        local: 9,
    };
    let nominal = Term::Nominal {
        declaration,
        arguments: vec![],
    };
    let pure = expr(boolean(), K::Literal(Literal::Bool(true)));
    let call = expr(
        boolean(),
        K::Call {
            callee: Box::new(expr(
                Term::Function {
                    parameters: vec![],
                    result: Box::new(boolean()),
                },
                K::Function(declaration),
            )),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    );
    let write = expr(
        Term::Primitive(P::Void),
        K::WritePointer {
            pointer: Box::new(pointer()),
            value: Box::new(pure.clone()),
        },
    );
    for (field, accepted) in [(pure.clone(), true), (call, false), (write, false)] {
        for aggregate in [
            expr(
                nominal.clone(),
                K::Record {
                    declaration,
                    fields: vec![field.clone()],
                },
            ),
            expr(
                nominal.clone(),
                K::Variant {
                    declaration,
                    index: 0,
                    fields: vec![field.clone()],
                },
            ),
            expr(
                Term::Record(vec![("flag".into(), boolean())]),
                K::AnonymousRecord {
                    fields: vec![field.clone()],
                },
            ),
            expr(Term::Tuple(vec![boolean()]), K::Tuple(vec![field.clone()])),
        ] {
            let predicate = expr(
                boolean(),
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(P::Void),
                        K::Let {
                            binding: LocalId { owner: 1, index: 0 },
                            value: Box::new(aggregate),
                        },
                    )],
                    result: Some(Box::new(pure.clone())),
                },
            );
            assert_eq!(checkable_predicate(&predicate), accepted, "{predicate}");
        }
    }
}
