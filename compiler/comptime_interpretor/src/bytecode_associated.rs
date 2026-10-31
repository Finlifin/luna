//! Associated initialization uses isolated environments and explicit activations.
use super::*;
use nir::program::Literal;
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn key(member: u32) -> (InterfaceInstance, Term, DeclarationId) {
    (
        InterfaceInstance {
            declaration: id(1),
            arguments: vec![],
        },
        Term::Primitive(Primitive::Bool),
        id(member),
    )
}
fn expr(kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(Primitive::Bool),
        kind,
    }
}
fn read(member: u32) -> Expr {
    let (interface, subject, member) = key(member);
    expr(K::AssociatedValue {
        interface,
        subject,
        member,
    })
}
fn run(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    env: &mut Environment,
    compiled: bool,
) -> Result<Flow, Error> {
    if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        assert!(code.to_string().contains("(read-associated "));
        vm.execute_code(&code, env)
    } else {
        vm.eval(body, env)
    }
}
#[test]
fn associated_cycles_and_failed_initializers_release_every_active_key() {
    let package = Package::default();
    let chain = [read(3), read(4), read(2)];
    let good = expr(K::Literal(Literal::Bool(true)));
    let caller = LocalId { owner: 7, index: 0 };
    let missing = expr(K::Local(caller));
    let wrong = expr(K::Literal(Literal::Char('x')));
    let returned = expr(K::Return(Some(Box::new(good.clone()))));
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (i, initializer) in chain.iter().enumerate() {
            vm.associated_initializers
                .insert(key(i as u32 + 2), initializer);
        }
        let mut env = Environment::from([(caller, Value::Bool(true))]);
        for (initializer, error) in [
            (&chain[2], ErrorKind::InvalidControlFlow),
            (&missing, ErrorKind::MissingBinding),
            (&wrong, ErrorKind::Type),
            (&returned, ErrorKind::InvalidControlFlow),
        ] {
            vm.associated_initializers.insert(key(4), initializer);
            assert_eq!(
                run(&mut vm, &read(2), &mut env, compiled).unwrap_err().kind,
                error
            );
            assert!(vm.active_associated_initializers.is_empty());
            assert_eq!(env[&caller], Value::Bool(true));
            vm.associated_initializers.insert(key(4), &good);
            assert!(matches!(
                run(&mut vm, &read(2), &mut env, compiled).unwrap(),
                Flow::Value(Value::Bool(true))
            ));
            assert!(vm.active_associated_initializers.is_empty());
        }
    }
}
#[test]
fn associated_chains_use_the_explicit_stack_and_clean_up_budget_failure() {
    let package = Package::default();
    let mut chain: Vec<_> = (3..67).map(read).collect();
    chain.push(expr(K::Literal(Literal::Bool(true))));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (i, initializer) in chain.iter().enumerate() {
        vm.associated_initializers
            .insert(key(i as u32 + 2), initializer);
    }
    vm.budget.depth = 8;
    let mut env = Environment::default();
    assert!(matches!(
        run(&mut vm, &read(2), &mut env, true).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    assert!(vm.active_associated_initializers.is_empty());
    vm.budget.steps = 100;
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.active_associated_initializers.is_empty());
    vm.budget.steps = Budget::default().steps;
    assert!(matches!(
        run(&mut vm, &read(2), &mut env, true).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
}

#[test]
fn associated_code_reuse_does_not_cache_selection_results_or_execution() {
    let package = Package::default();
    let yes = expr(K::Literal(Literal::Bool(true)));
    // Deliberately use identical source coordinates and result types: those are
    // not code identity, nor is the associated member's declaration alone.
    let no = expr(K::Literal(Literal::Bool(false)));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut env = Environment::default();
    for (initializer, expected, compilations) in [
        (&yes, true, 1),
        (&yes, true, 1),
        (&no, false, 2),
        (&yes, true, 2),
    ] {
        vm.associated_initializers.insert(key(2), initializer);
        let before = vm.remaining_steps();
        assert!(matches!(run(&mut vm, &read(2), &mut env, true).unwrap(),
            Flow::Value(Value::Bool(value)) if value == expected));
        assert_eq!(vm.bytecode_compilations(), compilations);
        assert!(vm.remaining_steps() < before);
        assert!(vm.active_associated_initializers.is_empty());
    }
    // A warmed body cannot bypass the current cycle guard or missing evidence.
    vm.active_associated_initializers.insert(key(2));
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
    vm.active_associated_initializers.clear();
    vm.associated_initializers.clear();
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::UnmaterializedDispatch
    );
}

#[test]
fn borrowed_code_cache_does_not_poison_retries_after_compilation_budget_failure() {
    let package = Package::default();
    let body = expr(K::Literal(Literal::Bool(true)));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.budget.steps = 0;
    assert_eq!(
        vm.borrowed_invocation_code(&body).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    vm.budget.steps = Budget::default().steps;
    let program = vm.borrowed_invocation_code(&body).unwrap().unwrap();
    let count = vm.bytecode_compilations();
    let reused = vm.borrowed_invocation_code(&body).unwrap().unwrap();
    assert!(Arc::ptr_eq(&program, &reused));
    assert_eq!(vm.bytecode_compilations(), count);
    vm.bytecode_enabled = false;
    assert!(vm.borrowed_invocation_code(&body).unwrap().is_none());
}
