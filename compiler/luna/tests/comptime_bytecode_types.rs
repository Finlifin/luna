//! Type equality uses host proofs and retains opaque normal-form identity.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ContractOutcome, ErrorKind, Interpreter, Value};
use nir::{Primitive, Term};

#[test]
fn type_equality_uses_proofs_without_unfolding_heads() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        typealias char = .char'builtin;
        pure comptime fn Identity<T> -> Type { T }
        comptime fn compare(left: Type, right: Type) -> bool { left == right }
        comptime fn Opaque() -> Type { Identity<bool> }
        comptime fn ordered() -> bool {
            var trace = false;
            let same = (do { trace = true; bool }) == (do { if trace { bool } else { char } });
            same and trace
        }
        comptime fn early() -> bool {
            (do { return true; bool }) == (do { while true {} bool });
            false
        }",
    );
    let context = elaboration::ptc::Context::from_package(&package).unwrap();
    let solver = |predicate: &nir::program::TypePredicate| {
        use elaboration::ptc::{Budget, Formula, Outcome};
        match context.prove(Formula::from_predicate(predicate), Budget::default()) {
            Outcome::Entailed(_) => ContractOutcome::Satisfied,
            Outcome::NotEntailed(_) => ContractOutcome::Rejected,
            _ => ContractOutcome::Unknown,
        }
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_type_solver(&solver);
    let compare = support::id(&package, "compare");
    for (right, expected) in [(Primitive::Bool, true), (Primitive::Char, false)] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(
                compare,
                vec![
                    Value::Type(Term::Primitive(Primitive::Bool)),
                    Value::Type(Term::Primitive(right))
                ]
            )
            .unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
    // Construction still uses the normal-form input-plan evaluator. The
    // comparison below must compile independently and keep that head opaque.
    let opaque = vm.invoke(support::id(&package, "Opaque"), vec![]).unwrap();
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(
            compare,
            vec![opaque, Value::Type(Term::Primitive(Primitive::Bool))]
        )
        .unwrap_err()
        .kind,
        ErrorKind::TypeContractUnresolved
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
    for name in ["ordered", "early"] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}
