//! Source preparation shares work without sharing user results or authority.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_executor::{Context, KeyProtocol};
use comptime_interpretor::Value;
use elaboration::{
    materialization::{DeclarationSnapshot, DeclarationSource},
    ptc::Budget,
};
use intrinsic::{LangItem, LangItems};
use luna::comptime_keys::Protocol;
use nir::{Primitive, Term, program::Package};

fn items(package: &Package) -> LangItems {
    let mut items = LangItems::new();
    for (item, name) in [
        (LangItem::Eq, "ComptimeEq"),
        (LangItem::Hash, "ComptimeHash"),
    ] {
        let id = package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration;
        items.bind_source_traits(package, &[(item, id)]).unwrap();
    }
    items
}

fn source(package: &Package) -> DeclarationSnapshot {
    DeclarationSource::Borrowed(package)
        .retain_declarations(&mut 1_000_000)
        .unwrap()
}

#[test]
fn key_preparation_returns_work_and_does_not_refill_after_context_construction() {
    let package = sysroot::compile("").unwrap();
    let items = items(&package);
    let source = source(&package);
    source.prepare(&mut Budget::default()).unwrap();
    let mut counter = 1_000_000;
    elaboration::ptc::Context::from_package_metered(source.package(), &mut counter).unwrap();
    let context_cost = 1_000_000 - counter;
    let subject = Term::Primitive(Primitive::Bool);
    for _ in 0..2 {
        let mut remaining = context_cost;
        let failure = Protocol::prepare_in_source_metered(
            source.clone(),
            &items,
            &subject,
            Budget::default(),
            &mut remaining,
        )
        .err()
        .unwrap();
        assert!(failure.contains("budget exhausted"), "{failure}");
        assert_eq!(remaining, 0);
    }
    let mut remaining = 1_000_000;
    let mut protocol = Protocol::prepare_in_source_metered(
        source.clone(),
        &items,
        &subject,
        Budget::default(),
        &mut remaining,
    )
    .unwrap();
    assert!(1_000_000 - remaining > context_cost);
    let input = protocol
        .admit(Value::Bool(true), &mut Context::new(1_000_000))
        .unwrap();
    assert!(protocol.hash(&input, &mut Context::new(1_000_000)).is_ok());
    let mut other =
        Protocol::prepare_in_source(source, &items, &subject, Budget::default()).unwrap();
    assert!(other.hash(&input, &mut Context::new(1_000_000)).is_err());
}

#[test]
fn many_key_body_requirements_share_one_selection_allowance() {
    use elaboration::key_protocols::Operation;
    let mut package = sysroot::compile("").unwrap();
    let items = items(&package);
    let subject = Term::Primitive(Primitive::Bool);
    let original = source(&package);
    let context = elaboration::ptc::Context::from_package(original.package()).unwrap();
    let world = original
        .capabilities(&context, &[], &mut Budget::default())
        .unwrap();
    let mut budget = Budget::default();
    let selected = luna::protocols::select_comptime_key_metered(
        original.package(),
        &world,
        &items,
        &subject,
        Operation::Eq,
        &mut budget,
    )
    .unwrap();
    let cost = Budget::default().steps - budget.steps;
    let body_id = selected.method.body;
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == body_id)
        .unwrap();
    function.requirements.extend(std::iter::repeat_n(
        nir::program::TypePredicate::Bool(true),
        4096,
    ));
    let changed = source(&package);
    let context = elaboration::ptc::Context::from_package(changed.package()).unwrap();
    let world = changed
        .capabilities(&context, &[], &mut Budget::default())
        .unwrap();
    for _ in 0..2 {
        let mut budget = Budget {
            steps: cost + 64,
            ..Default::default()
        };
        let error = luna::protocols::select_comptime_key_metered(
            changed.package(),
            &world,
            &items,
            &subject,
            Operation::Eq,
            &mut budget,
        )
        .unwrap_err();
        assert!(error.contains("budget exhausted"), "{error}");
        assert!(budget.steps < cost);
    }
    let selected = luna::protocols::select_comptime_key_metered(
        changed.package(),
        &world,
        &items,
        &subject,
        Operation::Eq,
        &mut Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(selected.method.body, body_id);
    assert!(selected.program.is_empty());
}

#[test]
fn cached_call_preparation_charges_all_input_protocols_and_retries() {
    use luna::memoization::Invocation;
    let package = sysroot::compile(
        "pure comptime fn Choose(left:bool, right:bool)->Type {if left and right {bool} else {u8}}",
    )
    .unwrap();
    let items = items(&package);
    let source = source(&package);
    let id = package
        .functions
        .iter()
        .find(|f| f.name == "Choose")
        .unwrap()
        .declaration;
    let mut remaining = 100_000;
    let mut call = Invocation::prepare_in_source_metered(
        source.clone(),
        &items,
        id,
        vec![],
        8,
        Budget::default(),
        &mut remaining,
    )
    .unwrap();
    let cost = 100_000 - remaining;
    assert!(cost > 1);
    let mut insufficient = 1;
    assert!(
        Invocation::prepare_in_source_metered(
            source,
            &items,
            id,
            vec![],
            8,
            Budget::default(),
            &mut insufficient
        )
        .is_err()
    );
    for (left, right, expected, reused) in [
        (true, true, Primitive::Bool, false),
        (false, true, Primitive::U8, false),
        (true, true, Primitive::Bool, true),
    ] {
        let result = call
            .invoke(
                vec![Value::Bool(left), Value::Bool(right)],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*result.value, Value::Type(Term::Primitive(expected)));
        assert_eq!(result.reused, reused);
    }
}

#[test]
fn failed_dynamic_key_preparation_spends_the_invocations_own_allowance() {
    use luna::memoization::Invocation;
    let package = sysroot::compile(
        "pure comptime trait LocalHash {pure comptime fn hash(self)->u64;}
         comptime impl LocalHash for Type {pure comptime fn hash(self)->u64 {0}}
         comptime impl LocalHash for bool {pure comptime fn hash(self)->u64 {0}}
         pure comptime fn Identity(comptime T:Type, value:T)->Type {T}",
    )
    .unwrap();
    let mut all_items = LangItems::new();
    all_items
        .set(LangItem::Eq, items(&package).get(LangItem::Eq).unwrap())
        .unwrap();
    let hash = package
        .traits
        .iter()
        .find(|t| t.name == "LocalHash")
        .unwrap()
        .declaration;
    // A constant hash is legal: collisions still require source equality.
    all_items
        .bind_source_traits(&package, &[(LangItem::Hash, hash)])
        .unwrap();
    let mut eq_only = LangItems::new();
    eq_only
        .set(LangItem::Eq, all_items.get(LangItem::Eq).unwrap())
        .unwrap();
    let source = source(&package);
    let id = package
        .functions
        .iter()
        .find(|f| f.name == "Identity")
        .unwrap()
        .declaration;
    let subject = Term::Primitive(Primitive::Type);
    // Warm metadata, then observe the real failing protocol preparation cost.
    source.prepare(&mut Budget::default()).unwrap();
    let mut remaining = 1_000_000;
    let error = Protocol::prepare_in_source_metered(
        source.clone(),
        &eq_only,
        &subject,
        Budget::default(),
        &mut remaining,
    )
    .err()
    .unwrap();
    assert!(error.contains("hash language item is not bound"), "{error}");
    let protocol_cost = 1_000_000 - remaining;
    assert!(protocol_cost > 0);
    let values = || {
        vec![
            Value::Type(Term::Primitive(Primitive::Bool)),
            Value::Bool(false),
        ]
    };
    let mut failed =
        Invocation::prepare_in_source(source.clone(), &eq_only, id, vec![], 8, Budget::default())
            .unwrap();
    for _ in 0..2 {
        let mut context = Context::new(1_000_000);
        let error = failed.invoke(values(), &mut context).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("hash language item is not bound"),
            "{error}"
        );
        assert!(1_000_000 - context.remaining() >= protocol_cost);
        assert_eq!(failed.cached_results(), 0);
    }
    let mut recovered =
        Invocation::prepare_in_source(source, &all_items, id, vec![], 8, Budget::default())
            .unwrap();
    for reused in [false, true] {
        let outcome = recovered
            .invoke(values(), &mut Context::new(1_000_000))
            .unwrap();
        assert_eq!(
            *outcome.value,
            Value::Type(Term::Primitive(Primitive::Bool))
        );
        assert_eq!(outcome.reused, reused);
    }
}
