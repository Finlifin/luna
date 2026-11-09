use materialization::{Expansion, dispatch_instances_for_roots, instantiate};
use nir::{DeclarationId as Id, Primitive, Term, program as n};
use std::{cell::Cell, collections::BTreeSet};
use type_resolution::ptc::Budget;

fn id(local: u32) -> Id {
    Id { package: 1, local }
}

#[test]
fn nominal_publication_charges_metadata_and_recovers_after_failure() {
    for kind in 0..3 {
        let mut package = n::Package::default();
        let name = "x".repeat(100_000);
        if kind == 1 {
            package.enums.push(n::Enum {
                declaration: id(10),
                name: "Choice".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                groups: vec![],
                encoding: None,
                symbol_encoding: None,
                is_intrinsic: false,
                variants: vec![n::Variant {
                    name,
                    shape: n::VariantShape::Unit,
                    fields: vec![],
                }],
            });
        } else if kind == 2 {
            package.newtypes.push(nir::newtypes::Newtype {
                declaration: id(10),
                name,
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: Term::Primitive(Primitive::Bool),
            });
        } else {
            package.structs.push(n::Struct {
                declaration: id(10),
                name,
                type_parameters: vec![],
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                fields: vec![],
            });
        }
        let ty = Term::Nominal {
            declaration: id(10),
            arguments: vec![],
        };
        let materialize = |steps| {
            let mut budget = Budget::default();
            budget.steps = steps;
            materialization::instantiate_with_types(
                &package,
                &[],
                std::slice::from_ref(&ty),
                BTreeSet::new(),
                budget,
                &Provider::default(),
            )
        };
        let failure = materialize(5000)
            .err()
            .expect("nominal metadata copies must consume budget");
        assert!(failure.contains("budget"), "{failure}");
        let graph = materialize(1_000_000).unwrap();
        assert_eq!(graph.nominals.len(), 1);
        assert_eq!(graph.nominals[0].ty, ty);
        match &graph.nominals[0].definition {
            iir::NominalDefinition::Struct(definition) => {
                assert_eq!(definition.name, package.structs[0].name)
            }
            iir::NominalDefinition::Enum(definition) => {
                assert_eq!(definition.variants, package.enums[0].variants)
            }
            iir::NominalDefinition::Newtype(definition) => {
                assert_eq!(definition.name, package.newtypes[0].name);
                assert_eq!(
                    definition.representation,
                    package.newtypes[0].representation
                );
            }
        }
    }
}
fn fixture() -> (n::Package, iir::InstanceKey) {
    let boolean = Term::Primitive(Primitive::Bool);
    let function = n::Function {
        declaration: id(1),
        name: "run".into(),
        phase: n::FunctionPhase::Runtime,
        inline: false,
        pure: true,
        is_unsafe: false,
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        parameters: vec![],
        defaults: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        },
        body: n::Expr {
            ty: boolean,
            source: n::SourceRange { start: 1, end: 2 },
            kind: n::ExprKind::Literal(n::Literal::Bool(true)),
        },
    };
    (
        n::Package {
            functions: vec![function],
            ..Default::default()
        },
        iir::InstanceKey {
            declaration: id(1),
            arguments: vec![].into(),
        },
    )
}

#[derive(Default)]
struct Provider {
    quotes: Cell<usize>,
    effects: Cell<usize>,
    first_allowance: Cell<Option<usize>>,
    fail: bool,
    remaining_after_effects: Option<usize>,
}
impl Expansion for Provider {
    fn quotes(
        &self,
        _: &n::Package,
        expression: &mut n::Expr,
        next: &mut u32,
        sources: &mut Vec<n::ExpansionSource>,
        _: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        if self.first_allowance.get().is_none() {
            self.first_allowance.set(Some(*fuel));
        }
        self.quotes.set(self.quotes.get() + 1);
        *fuel = fuel.checked_sub(1).ok_or("provider fuel exhausted")?;
        *next = next.checked_add(1).ok_or("provider identity exhausted")?;
        sources.push(n::ExpansionSource {
            original: expression.source,
            site: expression.source,
        });
        if self.fail {
            Err("injected expansion failure".into())
        } else {
            Ok(())
        }
    }
    fn effects(
        &self,
        _: &n::Package,
        _: &BTreeSet<Id>,
        _: &mut n::Expr,
        _: &mut u32,
        _: &mut Vec<n::ExpansionSource>,
        _: &mut Vec<n::Enum>,
        fuel: &mut usize,
    ) -> Result<(usize, usize), String> {
        self.effects.set(self.effects.get() + 1);
        if let Some(remaining) = self.remaining_after_effects {
            *fuel = (*fuel).min(remaining);
        }
        Ok((0, 0))
    }
    fn quote(
        &self,
        _: &n::Package,
        _: &n::Function,
        _: &mut n::Expr,
        _: &mut u32,
        _: &mut Vec<n::ExpansionSource>,
        _: &mut Vec<n::Enum>,
        _: &mut usize,
    ) -> Result<(), String> {
        panic!("plain literal must not request bound quote expansion")
    }
    fn inline(
        &self,
        _: &n::Package,
        _: &n::Function,
        _: &mut n::Expr,
        _: &mut u32,
        _: &mut Vec<n::ExpansionSource>,
        _: &mut Vec<n::Enum>,
        _: &mut usize,
    ) -> Result<bool, String> {
        panic!("non-inline function must not request bound expansion")
    }
}

#[test]
fn retained_source_reuses_the_directory_but_rebuilds_each_instance_graph() {
    use materialization::{DeclarationSnapshot, DeclarationSource, instantiate_in_source};
    use std::sync::Arc;
    let (mut package, root) = fixture();
    let prototype = package.functions[0].clone();
    for local in 10..522 {
        let mut function = prototype.clone();
        function.declaration = id(local);
        function.body.kind = n::ExprKind::Literal(n::Literal::Bool(false));
        package.functions.push(function);
    }
    let source = DeclarationSnapshot::new(Arc::new(package));
    source.prepare(&mut Budget::default()).unwrap();
    let provider = Provider::default();
    let borrowed = instantiate(
        source.package(),
        std::slice::from_ref(&root),
        BTreeSet::new(),
        Budget::default(),
        &provider,
    )
    .unwrap();
    let cold_allowance = provider.first_allowance.get().unwrap();
    let before = source.to_string();
    for _ in 0..2 {
        let provider = Provider::default();
        let graph = instantiate_in_source(
            DeclarationSource::Snapshot(&source),
            std::slice::from_ref(&root),
            BTreeSet::new(),
            Budget::default(),
            &provider,
        )
        .unwrap();
        assert!(provider.first_allowance.get().unwrap() > cold_allowance);
        assert!(provider.quotes.get() > 0 && provider.effects.get() > 0);
        assert_eq!(graph.to_string(), borrowed.to_string());
    }
    let failed = Provider {
        fail: true,
        ..Default::default()
    };
    assert_eq!(
        instantiate_in_source(
            DeclarationSource::Snapshot(&source),
            std::slice::from_ref(&root),
            BTreeSet::new(),
            Budget::default(),
            &failed
        )
        .unwrap_err(),
        "injected expansion failure"
    );
    assert_eq!(failed.quotes.get(), 1);
    assert!(
        instantiate_in_source(
            DeclarationSource::Snapshot(&source),
            std::slice::from_ref(&root),
            BTreeSet::new(),
            Budget {
                steps: 0,
                ..Default::default()
            },
            &Provider::default()
        )
        .is_err()
    );
    let other = iir::InstanceKey {
        declaration: id(10),
        arguments: vec![].into(),
    };
    let graph = instantiate_in_source(
        DeclarationSource::Snapshot(&source),
        std::slice::from_ref(&other),
        BTreeSet::new(),
        Budget::default(),
        &Provider::default(),
    )
    .unwrap();
    assert_eq!(graph.roots, vec![other.clone()]);
    assert_eq!(graph.functions.len(), 1);
    assert_eq!(graph.functions[0].key, other);
    assert_eq!(source.to_string(), before);

    // The same declaration number in edited source is a different input, even
    // when a previous snapshot successfully published an executable graph.
    let mut changed = Arc::clone(source.package());
    Arc::make_mut(&mut changed).functions[0].body.kind = n::ExprKind::Call {
        callee: Box::new(n::Expr {
            source: n::SourceRange { start: 1, end: 2 },
            ty: prototype.signature,
            kind: n::ExprKind::Function(id(900)),
        }),
        arguments: vec![],
        parameter_bindings: vec![],
    };
    let changed = DeclarationSnapshot::new(changed);
    assert!(
        instantiate_in_source(
            DeclarationSource::Snapshot(&changed),
            std::slice::from_ref(&root),
            BTreeSet::new(),
            Budget::default(),
            &Provider::default()
        )
        .is_err()
    );
    let recovered = instantiate_in_source(
        DeclarationSource::Snapshot(&source),
        &[root],
        BTreeSet::new(),
        Budget::default(),
        &Provider::default(),
    )
    .unwrap();
    assert_eq!(recovered.to_string(), borrowed.to_string());
}

#[test]
fn dispatch_closes_transitive_calls_without_a_source_provider() {
    let (mut package, root) = fixture();
    assert!(dispatch_instances_for_roots(&package, &[root.clone()], Budget::default()).is_ok());
    let signature = package.functions[0].signature.clone();
    let body = &mut package.functions[0].body;
    body.kind = n::ExprKind::Call {
        callee: Box::new(n::Expr {
            source: body.source,
            ty: signature,
            kind: n::ExprKind::Function(id(99)),
        }),
        arguments: vec![],
        parameter_bindings: vec![],
    };
    assert!(
        dispatch_instances_for_roots(&package, &[root], Budget::default()).is_err(),
        "dispatch must still validate the missing transitive body"
    );
}

#[test]
fn iir_service_is_scoped_and_failed_candidates_do_not_publish() {
    let (package, root) = fixture();
    let before = package.clone();
    let failed = Provider {
        fail: true,
        ..Default::default()
    };
    let error = instantiate(
        &package,
        &[root.clone()],
        BTreeSet::new(),
        Budget::default(),
        &failed,
    )
    .unwrap_err();
    assert_eq!(error, "injected expansion failure");
    assert_eq!(failed.quotes.get(), 1);
    assert_eq!(failed.effects.get(), 0);
    assert_eq!(package, before);
    let successful = Provider::default();
    let output = instantiate(
        &package,
        &[root],
        BTreeSet::new(),
        Budget::default(),
        &successful,
    )
    .unwrap();
    assert_eq!(output.functions.len(), 1);
    assert!(successful.quotes.get() > 0);
    assert!(successful.effects.get() > 0);
    assert_eq!(output.expansion_sources.len(), successful.quotes.get());
    assert_eq!(package, before);
}

#[test]
fn dispatch_roots_do_not_visit_unrelated_function_bodies() {
    let (mut package, root) = fixture();
    let mut unrelated = package.functions[0].clone();
    unrelated.declaration = id(2);
    unrelated.name = "unrelated".into();
    unrelated.body.kind = n::ExprKind::Call {
        callee: Box::new(n::Expr {
            source: unrelated.body.source,
            ty: unrelated.signature.clone(),
            kind: n::ExprKind::Function(id(99)),
        }),
        arguments: vec![],
        parameter_bindings: vec![],
    };
    package.functions.push(unrelated);
    for materialize in [
        dispatch_instances_for_roots,
        materialization::dispatch_instances_for_invocation,
    ] {
        assert!(materialize(&package, &[root.clone()], Budget::default()).is_ok());
        assert!(
            materialize(
                &package,
                &[iir::InstanceKey {
                    declaration: id(2),
                    arguments: vec![].into(),
                }],
                Budget::default()
            )
            .is_err()
        );
    }
}

#[test]
fn closure_context_inputs_retain_capture_layouts() {
    let (mut package, root) = fixture();
    let nominal = Term::Nominal {
        declaration: id(50),
        arguments: vec![],
    };
    package.structs.push(n::Struct {
        declaration: id(50),
        name: "Capture".into(),
        parameter_types: vec![],
        type_parameters: vec![],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![("value".into(), Term::Primitive(Primitive::Bool))],
    });
    package.structs.push(n::Struct {
        declaration: id(51),
        name: "Carrier".into(),
        parameter_types: vec![Term::ClosureContextType],
        type_parameters: vec![id(52)],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![],
    });
    let context = Term::ClosureContextValue(vec![nominal.clone()]);
    let carrier = Term::Nominal {
        declaration: id(51),
        arguments: vec![context],
    };
    let graph = materialization::instantiate_with_types(
        &package,
        &[root],
        &[carrier],
        BTreeSet::new(),
        Budget::default(),
        &Provider::default(),
    )
    .unwrap();
    assert!(graph.nominals.iter().any(|instance| instance.ty == nominal));
    graph.validate(&mut 100_000).unwrap();
}

#[test]
fn publication_cannot_replenish_the_expansion_allowance() {
    let (package, root) = fixture();
    let before = package.clone();
    let mut rejected = 0;
    let mut accepted = 0;
    for remaining in 0..256 {
        let provider = Provider {
            remaining_after_effects: Some(remaining),
            ..Default::default()
        };
        match instantiate(
            &package,
            std::slice::from_ref(&root),
            BTreeSet::new(),
            Budget::default(),
            &provider,
        ) {
            Ok(output) => {
                accepted += 1;
                // Publication includes final graph validation. Even that pass
                // alone must fit in the allowance left by source expansion;
                // graph construction and template storage also draw from it.
                let mut validation = remaining;
                output
                    .validate(&mut validation)
                    .expect("publication silently replenished the expansion allowance");
                assert_eq!(output.roots, vec![root.clone()]);
            }
            Err(error) => {
                rejected += 1;
                assert!(
                    error.contains("budget") || error.ends_with("within the type inspection limit"),
                    "{error}"
                );
            }
        }
        assert_eq!(package, before);
    }
    assert!(rejected > 0 && accepted > 0);
    let output = instantiate(
        &package,
        &[root],
        BTreeSet::new(),
        Budget::default(),
        &Provider::default(),
    )
    .unwrap();
    assert_eq!(output.functions.len(), 1);
}
