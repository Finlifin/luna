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
