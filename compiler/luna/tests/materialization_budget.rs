//! Cleanup may discover new roots, but graph preparation has one stage budget.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use elaboration::materialization::{DeclarationSnapshot, DeclarationSource};
use elaboration::ptc::{Budget, Context};
use std::sync::Arc;

#[test]
fn cleanup_root_growth_spends_one_graph_allowance_and_recovers() {
    let source = DeclarationSnapshot::new(Arc::new(support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         struct Held {} impl Retire for Held {fn drop(*self) {}}
         fn run() {let held=Held {};}",
    )));
    let package = source.package();
    let entry = iir::InstanceKey {
        declaration: support::id(package, "run"),
        arguments: vec![].into(),
    };
    let mut first_budget = Budget::default();
    let initial = first_budget.steps;
    let first = elaboration::materialization::instantiate_with_types_in_source_in_budget(
        DeclarationSource::Borrowed(package),
        std::slice::from_ref(&entry),
        &[],
        Default::default(),
        &mut first_budget,
    )
    .unwrap();
    assert_eq!(
        first.functions.len(),
        1,
        "the initial graph has not discovered its destructor"
    );
    let first_cost = initial - first_budget.steps;
    let context = Context::from_package(package).unwrap();
    let world = source
        .capabilities(&context, &[], &mut Budget::default())
        .unwrap();
    let mut items = intrinsic::LangItems::new();
    for (item, name) in [
        (intrinsic::LangItem::Copy, "Duplicate"),
        (intrinsic::LangItem::Drop, "Retire"),
    ] {
        let declaration = package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration;
        items
            .bind_source_traits(package, &[(item, declaration)])
            .unwrap();
    }
    let mut planning = initial;
    let failure = luna::cleanup::prepare_source(
        package,
        &world,
        &items,
        entry.clone(),
        &mut planning,
        Budget {
            steps: first_cost,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(failure, "IIR phase evidence access budget exhausted");
    assert!(
        planning > 0 && planning < initial,
        "resource planning has its own stage counter"
    );
    let mut planning = initial;
    let completed = luna::cleanup::prepare_source(
        package,
        &world,
        &items,
        entry,
        &mut planning,
        Budget::default(),
    )
    .unwrap();
    assert!(completed.graph.functions.len() > first.functions.len());
    let mut vm = comptime_interpretor::Interpreter::from_iir(&completed.graph, Default::default())
        .unwrap()
        .with_cleanup_schedule(&completed.schedule)
        .unwrap();
    assert_eq!(
        vm.invoke_instance(&completed.entry, vec![]).unwrap(),
        comptime_interpretor::Value::Unit
    );
}
