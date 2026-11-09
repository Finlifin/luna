//! Key protocols share source code, never user results or invocation storage.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_executor::{Context, KeyProtocol};
use comptime_interpretor::Value;
use luna::comptime_keys::Protocol;
use nir::{Primitive, Term, program::Package};

fn bindings(package: &Package) -> intrinsic::LangItems {
    let mut items = intrinsic::LangItems::new();
    for (item, name) in [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
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
    items
}

fn prepare(package: &Package, subject: &Term) -> Protocol {
    Protocol::prepare(package, &bindings(package), subject, Default::default()).unwrap()
}

#[test]
fn borrowed_key_capture_is_bounded_and_shared_sources_remain_separate_bindings() {
    use elaboration::materialization::DeclarationSource;
    let package = sysroot::compile("").unwrap();
    let items = bindings(&package);
    let subject = Term::Primitive(Primitive::Bool);
    let error = Protocol::prepare(
        &package,
        &items,
        &subject,
        elaboration::ptc::Budget {
            steps: 1,
            ..Default::default()
        },
    )
    .err()
    .unwrap();
    assert!(
        error.contains("key source retention budget exhausted"),
        "{error}"
    );
    let source = DeclarationSource::Borrowed(&package)
        .retain_declarations(&mut 100_000)
        .unwrap();
    let weak = std::sync::Arc::downgrade(source.package());
    let mut first =
        Protocol::prepare_in_source(source.clone(), &items, &subject, Default::default()).unwrap();
    let mut second =
        Protocol::prepare_in_source(source.clone(), &items, &subject, Default::default()).unwrap();
    drop(source);
    let value = first
        .admit(Value::Bool(true), &mut Context::new(1_000_000))
        .unwrap();
    let hash = first.hash(&value, &mut Context::new(1_000_000)).unwrap();
    assert!(second.hash(&value, &mut Context::new(1_000_000)).is_err());
    let other = second
        .admit(Value::Bool(true), &mut Context::new(1_000_000))
        .unwrap();
    assert_eq!(
        second.hash(&other, &mut Context::new(1_000_000)).unwrap(),
        hash
    );
    drop(first);
    assert!(weak.upgrade().is_some());
    drop(second);
    assert!(weak.upgrade().is_none());
}

#[test]
fn key_code_reuse_preserves_fresh_inputs_contracts_and_receiver_storage() {
    let package = sysroot::compile(
        "struct Key { value: bool, allowed: bool }
        comptime impl ComptimeEq for Key {
            comptime fn eq(self, other: Self) -> bool {
                assumes self.allowed and other.allowed;
                let previous = self.value;
                self.value = not previous;
                previous == other.value
            }
        }
        comptime impl ComptimeHash for Key {
            comptime fn hash(self) -> u64 {
                assumes self.allowed;
                var fresh = false;
                fresh = not fresh;
                let previous = self.value;
                self.value = not previous;
                if fresh and previous { 1 } else { 0 }
            }
        }",
    )
    .unwrap();
    let declaration = package
        .structs
        .iter()
        .find(|s| s.name == "Key")
        .unwrap()
        .declaration;
    let ty = Term::Nominal {
        declaration,
        arguments: vec![],
    };
    let key = |value, allowed| Value::Record {
        declaration,
        arguments: vec![],
        fields: vec![Value::Bool(value), Value::Bool(allowed)],
    };
    let mut protocol = prepare(&package, &ty);
    let saved = key(true, true);
    let snapshot = protocol
        .admit(saved.clone(), &mut Context::new(1_000_000))
        .unwrap();
    let first = protocol
        .hash(&snapshot, &mut Context::new(1_000_000))
        .unwrap();
    assert!(protocol.last_execution_stats().bytecode_compilations > 0);
    for _ in 0..2 {
        assert_eq!(
            protocol
                .hash(&snapshot, &mut Context::new(1_000_000))
                .unwrap(),
            first
        );
        assert_eq!(protocol.last_execution_stats().bytecode_compilations, 0);
        assert!(protocol.last_execution_stats().bytecode_invocations > 0);
        assert_eq!(snapshot.value(), &saved);
    }
    let other = key(false, true);
    assert_ne!(
        protocol.hash(&other, &mut Context::new(1_000_000)).unwrap(),
        first
    );
    for expected in [false, true, false, true] {
        let right = if expected { &saved } else { &other };
        assert_eq!(
            protocol
                .equivalent(&saved, right, &mut Context::new(1_000_000))
                .unwrap(),
            expected
        );
        assert!(protocol.last_execution_stats().bytecode_invocations > 0);
    }
    assert_eq!(protocol.last_execution_stats().bytecode_compilations, 0);
    let poison = key(true, false);
    // Formation succeeds; the user method must still check its current input.
    let poison = protocol
        .admit(poison, &mut Context::new(1_000_000))
        .unwrap();
    for _ in 0..2 {
        let error = protocol
            .hash(&poison, &mut Context::new(1_000_000))
            .unwrap_err();
        assert!(
            error.to_string().contains("ProgramContractRejected"),
            "{error}"
        );
        assert_eq!(protocol.last_execution_stats().bytecode_compilations, 0);
        assert_eq!(
            protocol
                .hash(&snapshot, &mut Context::new(1_000_000))
                .unwrap(),
            first
        );
        assert!(
            protocol
                .admit(Value::Bool(true), &mut Context::new(1_000_000))
                .is_err()
        );
    }
}

#[test]
fn protocol_code_and_snapshots_belong_to_their_exact_source_world() {
    let mut package = sysroot::compile("").unwrap();
    let subject = Term::Primitive(Primitive::Bool);
    let mut old = prepare(&package, &subject);
    let snapshot = old
        .admit(Value::Bool(true), &mut Context::new(1_000_000))
        .unwrap();
    let digest = old.hash(&snapshot, &mut Context::new(1_000_000)).unwrap();
    let hash = package
        .traits
        .iter()
        .find(|t| t.name == "ComptimeHash")
        .unwrap()
        .declaration;
    let body = package
        .implementations
        .iter()
        .find(|i| i.interface == Some(hash) && i.subject == subject)
        .unwrap()
        .methods
        .iter()
        .find(|(name, _)| name == "hash")
        .unwrap()
        .1;
    package
        .functions
        .iter_mut()
        .find(|f| f.declaration == body)
        .unwrap()
        .body
        .kind = nir::program::ExprKind::Literal(nir::program::Literal::UnsignedInteger(9));
    let mut changed = prepare(&package, &subject);
    let changed_snapshot = changed
        .admit(Value::Bool(true), &mut Context::new(1_000_000))
        .unwrap();
    let changed_digest = changed
        .hash(&changed_snapshot, &mut Context::new(1_000_000))
        .unwrap();
    assert!(changed.last_execution_stats().bytecode_compilations > 0);
    assert_ne!(digest, changed_digest);
    assert!(
        changed
            .hash(&snapshot, &mut Context::new(1_000_000))
            .is_err()
    );
    assert!(
        old.hash(&changed_snapshot, &mut Context::new(1_000_000))
            .is_err()
    );
    for _ in 0..2 {
        assert_eq!(
            old.hash(&snapshot, &mut Context::new(1_000_000)).unwrap(),
            digest
        );
        assert_eq!(old.last_execution_stats().bytecode_compilations, 0);
        assert_eq!(
            changed
                .hash(&changed_snapshot, &mut Context::new(1_000_000))
                .unwrap(),
            changed_digest
        );
        assert_eq!(changed.last_execution_stats().bytecode_compilations, 0);
    }
}

#[test]
fn warm_protocols_charge_execution_and_recover_after_budget_exhaustion() {
    let package = sysroot::compile("").unwrap();
    let mut protocol = prepare(&package, &Term::Primitive(Primitive::Bool));
    let mut cold = Context::new(1_000_000);
    let digest = protocol.hash(&Value::Bool(true), &mut cold).unwrap();
    let mut warm = Context::new(1_000_000);
    assert_eq!(
        protocol.hash(&Value::Bool(true), &mut warm).unwrap(),
        digest
    );
    let work = 1_000_000 - warm.remaining();
    assert!(work > 1 && warm.remaining() > cold.remaining());
    assert_eq!(protocol.last_execution_stats().bytecode_compilations, 0);
    let input_reservation = Value::Bool(true).immutable_clone_work().unwrap();
    for allowance in [0, 1, work / 2, work - 1] {
        let mut limited = Context::new(allowance);
        assert!(protocol.hash(&Value::Bool(true), &mut limited).is_err());
        if allowance < input_reservation {
            // An unaffordable input reservation has not entered the VM.
            assert_eq!(limited.remaining(), allowance);
        } else {
            assert!(limited.remaining() < allowance);
        }
        let mut recovered = Context::new(1_000_000);
        assert_eq!(
            protocol.hash(&Value::Bool(true), &mut recovered).unwrap(),
            digest
        );
        assert_eq!(1_000_000 - recovered.remaining(), work);
        assert_eq!(protocol.last_execution_stats().bytecode_compilations, 0);
        assert!(protocol.last_execution_stats().bytecode_invocations > 0);
    }
}

#[test]
fn shared_declaration_sources_do_not_merge_protocol_value_authority() {
    use elaboration::materialization::DeclarationSnapshot;
    use std::sync::Arc;
    let mut package = sysroot::compile("").unwrap();
    let items = bindings(&package);
    let subject = Term::Primitive(Primitive::Bool);
    let closed = DeclarationSnapshot::new(Arc::new(package.clone()));
    assert!(Protocol::prepare_in_source(closed, &items, &subject, Default::default()).is_err());
    // Convert once before sharing the exact source with several bindings.
    // A closed dispatch table cannot supply authority to resolving execution.
    package.dispatch_instances.clear();
    let package = Arc::new(package);
    let weak = Arc::downgrade(&package);
    let source = DeclarationSnapshot::new(package);
    let mut first =
        Protocol::prepare_in_source(source.clone(), &items, &subject, Default::default()).unwrap();
    let mut second =
        Protocol::prepare_in_source(source.clone(), &items, &subject, Default::default()).unwrap();
    drop(source);
    assert!(weak.upgrade().is_some());
    let admitted = first
        .admit(Value::Bool(true), &mut Context::new(1_000_000))
        .unwrap();
    let digest = first.hash(&admitted, &mut Context::new(1_000_000)).unwrap();
    assert!(
        second
            .hash(&admitted, &mut Context::new(1_000_000))
            .is_err()
    );
    let own = second
        .admit(Value::Bool(true), &mut Context::new(1_000_000))
        .unwrap();
    assert_eq!(
        second.hash(&own, &mut Context::new(1_000_000)).unwrap(),
        digest
    );
    assert!(first.hash(&own, &mut Context::new(1_000_000)).is_err());
    assert_eq!(
        first.hash(&admitted, &mut Context::new(1_000_000)).unwrap(),
        digest
    );
    drop(first);
    assert!(weak.upgrade().is_some());
    drop(second);
    // Value snapshots retain the protocol token, not compiler source/code.
    assert!(weak.upgrade().is_none());
    assert_eq!(admitted.value(), &Value::Bool(true));
    assert_eq!(own.value(), &Value::Bool(true));
}
