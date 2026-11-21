use super::*;
use nir::program::{Place, TextQueryKind};

#[test]
fn observation_admission_precedes_operand_effects() {
    let package = Package::default();
    let source = SourceRange { start: 60, end: 65 };
    let trace = LocalId { owner: 1, index: 0 };
    let expr = |ty, kind| Expr { source, ty, kind };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for kind in 0..3 {
            for valid in [false, true] {
                if kind == 2 && valid {
                    continue;
                }
                let ty = if valid {
                    Term::Primitive(Primitive::Symbol)
                } else {
                    Term::Primitive(Primitive::Bool)
                };
                let operand = expr(
                    ty,
                    K::Block {
                        statements: vec![expr(
                            Term::Primitive(Primitive::Void),
                            K::Assign {
                                place: Place {
                                    root: trace,
                                    fields: vec![],
                                },
                                value: Box::new(expr(
                                    Term::Primitive(Primitive::Bool),
                                    K::Literal(Literal::Bool(true)),
                                )),
                            },
                        )],
                        result: Some(Box::new(expr(
                            Term::Primitive(Primitive::Symbol),
                            K::Literal(Literal::Symbol("ready".into())),
                        ))),
                    },
                );
                let body = match kind {
                    0 => expr(
                        Term::Primitive(Primitive::Str),
                        K::SymbolName(Box::new(operand)),
                    ),
                    1 => expr(
                        Term::Primitive(Primitive::U64),
                        K::TextQuery {
                            subject: Box::new(expr(
                                Term::Tuple(vec![operand.ty.clone()]),
                                K::Tuple(vec![operand]),
                            )),
                            query: TextQueryKind::SymbolByteLength,
                        },
                    ),
                    _ => expr(
                        Term::Primitive(Primitive::Str),
                        K::EnumName(Box::new(operand)),
                    ),
                };
                let mut env = Environment::from([(trace, Value::Bool(false))]);
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                    assert!(code.to_string().contains("(check-observation "));
                    vm.execute_code(&code, &mut env)
                } else {
                    vm.eval(&body, &mut env)
                };
                assert_eq!(result.is_ok(), valid);
                assert_eq!(
                    vm.local_value(&env, trace, Some(source)).unwrap(),
                    Value::Bool(valid)
                );
            }
        }
    }
}

#[test]
fn attribute_queries_keep_target_identity_and_the_invocations_publication_state() {
    use nir::program::DefinitionAttributes;
    let id = |local| DeclarationId { package: 81, local };
    let target = id(1);
    let plain = id(2);
    let source = SourceRange { start: 25, end: 31 };
    let proj = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(3),
            member: id(4),
        }),
        arguments: vec![],
        subject: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let selected = Expr {
        source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::Nominal {
            declaration: target,
            arguments: vec![],
        }),
    };
    let nominal = |declaration| Term::Nominal {
        declaration,
        arguments: vec![],
    };
    let definition = |declaration| nir::program::Struct {
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration,
        name: format!("D{}", declaration.local),
        fields: vec![],
        packed: false,
    };
    let package = Package {
        definition_attributes: DefinitionAttributes::Pending(BTreeSet::from([target]).into()),
        structs: vec![definition(target), definition(plain)],
        ..Default::default()
    };
    for (compiled, supply) in [(false, false), (false, true), (true, false), (true, true)] {
        let calls = std::cell::Cell::new(0);
        let resolver = |requested, remaining: &mut u64| {
            assert_eq!(requested, target);
            *remaining -= 1;
            calls.set(calls.get() + 1);
            Ok(Some(Arc::new(nir::objects::value(vec![
                nir::objects::entry(
                    Some("ready"),
                    Term::Primitive(Primitive::Bool),
                    Term::Bool(true),
                ),
            ]))))
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        if supply {
            vm = vm.with_attribute_resolver(&resolver);
        }
        vm.associated_values.insert(
            (
                InterfaceInstance {
                    declaration: id(3),
                    arguments: vec![],
                },
                Term::Primitive(Primitive::Bool),
                id(4),
            ),
            &selected,
        );
        let mut read = |ty| {
            let expr = Expr {
                source,
                ty: Term::Primitive(Primitive::Object),
                kind: K::DefinitionAttributes(Box::new(Expr {
                    source,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(ty),
                })),
            };
            let mut env = Environment::new();
            if compiled {
                let code = Arc::new(comptime_compiler::compile(&expr, &mut 1000).unwrap());
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&expr, &mut env)
            }
        };
        for ty in [proj.clone(), nominal(target)] {
            if supply {
                assert!(
                    matches!(read(ty).unwrap(), Flow::Value(Value::Object(o)) if o.entries().len() == 1)
                );
            } else {
                let error = read(ty).unwrap_err();
                assert_eq!(error.kind, ErrorKind::PendingDefinitionAttributes(target));
                assert_eq!(error.source, Some(source));
            }
            assert!(
                matches!(read(nominal(plain)).unwrap(), Flow::Value(Value::Object(o)) if o.entries().is_empty())
            );
        }
        assert_eq!(calls.get(), if supply { 2 } else { 0 });
    }
}
