use materialization::{Expansion, dispatch_instances_for_roots, instantiate};
use nir::{DeclarationId as Id, Primitive, Term, program as n};
use std::{cell::Cell, collections::BTreeSet};
use type_resolution::ptc::Budget;

fn id(local: u32) -> Id {
    Id { package: 1, local }
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
    fail: bool,
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
        _: &mut usize,
    ) -> Result<(usize, usize), String> {
        self.effects.set(self.effects.get() + 1);
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
