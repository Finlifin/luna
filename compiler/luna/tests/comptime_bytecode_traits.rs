//! Compiled trait calls retain their public interface around the chosen body.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};

#[test]
fn interface_defaults_use_declaration_bindings_and_method_values_compile() {
    let package = support::compile(
        "trait Choose {
        comptime fn choose(self,value:bool,.first:bool=not value,.last:bool=first)->bool;
        }
        struct Host {}
        impl Choose for Host {comptime fn choose(self,input:bool,a:bool,b:bool)->bool {b}}
        comptime fn direct()->bool {Host {}.as(Choose).choose(true)}
        comptime fn overridden()->bool {Host {}.as(Choose).choose(true,.first=true)}
        comptime fn stored()->bool {let f=Host.as(Choose).choose;f(Host {},true,false,true)}
        trait Echo where T {comptime fn echo(value:T)->T;}
        impl Echo<T> for Host where T {comptime fn echo(input:T)->T {input}}
        comptime fn specialized()->bool {Host.as(Echo<bool>).echo(true)}",
    );
    for compiled in [false, true] {
        let mut vm = if compiled {
            Interpreter::for_comptime(&package, Budget::default()).unwrap()
        } else {
            Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
        };
        for (name, value) in [
            ("direct", false),
            ("overridden", true),
            ("stored", true),
            ("specialized", true),
        ] {
            let id = support::id(&package, name);
            let before = vm.bytecode_invocations();
            assert_eq!(
                vm.invoke(id, vec![])
                    .unwrap_or_else(|e| panic!("{name}: {e:?}")),
                Value::Bool(value),
                "{name}"
            );
            assert_eq!(
                vm.bytecode_invocations() - before,
                if compiled { 2 } else { 0 },
                "{name}"
            );
        }
    }
    let package = std::sync::Arc::new(package);
    let mut session = comptime_interpretor::CodeSession::new(package.clone());
    for _ in 0..2 {
        let mut vm =
            Interpreter::for_comptime_in_session(&package, &mut session, &mut Budget::default())
                .unwrap();
        for (name, expected) in [
            ("direct", false),
            ("overridden", true),
            ("stored", true),
            ("specialized", true),
        ] {
            let value = vm
                .with_code_session(&mut session, |vm| {
                    vm.invoke(support::id(&package, name), vec![])
                })
                .unwrap();
            assert_eq!(value, Value::Bool(expected), "session call {name}");
        }
    }
}

#[test]
fn interface_preconditions_and_postconditions_survive_weaker_implementations() {
    let mut package = support::compile(
        "trait Service {
        comptime fn accept(value:bool)->result:bool where requires value, ensures result;
        }
        impl Service for bool {comptime fn accept(input:bool)->bool {true}}
        comptime fn run(value:bool)->bool {assumes value;bool.as(Service).accept(value)}",
    );
    // Remove only the caller proof after elaboration to test the retained
    // interface boundary independently of source proof admission.
    let run = package
        .functions
        .iter_mut()
        .find(|f| f.name == "run")
        .unwrap();
    let nir::program::ExprKind::Block { statements, .. } = &mut run.body.kind else {
        panic!("run block")
    };
    statements.clear();
    let id = support::id(&package, "run");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for value in [false, true, false, true] {
        let before = vm.bytecode_invocations();
        let result = vm.invoke(id, vec![Value::Bool(value)]);
        if value {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(
            vm.bytecode_invocations() - before,
            if value { 2 } else { 1 }
        );
    }
    let mut forged = package.clone();
    let body = forged
        .functions
        .iter_mut()
        .find(|f| f.name == "accept")
        .unwrap();
    body.body.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let mut vm = Interpreter::for_comptime(&forged, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id, vec![Value::Bool(true)]).unwrap_err().kind,
        ErrorKind::ProgramContractRejected
    );
    assert_eq!(vm.bytecode_invocations(), 2);
}

#[test]
fn nested_interface_exits_keep_independent_entry_snapshots() {
    let package = support::compile(
        "trait Echo {
        comptime fn echo(value:bool)->result:bool where ensures result == value'old;
        }
        impl Echo for bool {comptime fn echo(input:bool)->bool {input}}
        trait Wrap {
        comptime fn wrap(value:bool)->result:bool where ensures result == value'old;
        }
        impl Wrap for bool {comptime fn wrap(input:bool)->bool {bool.as(Echo).echo(input)}}
        comptime fn run(value:bool)->bool {bool.as(Wrap).wrap(value)}",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for value in [false, true, false] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![Value::Bool(value)])
                .unwrap(),
            Value::Bool(value)
        );
        assert_eq!(vm.bytecode_invocations() - before, 3);
    }
}

#[test]
fn dynamic_trait_instances_keep_interface_defaults_and_contracts_in_bytecode() {
    let package = support::compile(
        "trait Choose {
            comptime fn choose(.value:bool=flag)->result:bool
            where flag:bool, ensures result == value;
        }
        struct Host {}
        impl Choose for Host {
            comptime fn choose(input:bool)->bool where flag:bool {input}
        }
        comptime fn run(flag:bool)->bool {Host.as(Choose).choose<flag>()}
        comptime fn overridden(flag:bool)->bool {Host.as(Choose).choose<flag>(.value=flag)}",
    );
    for name in ["run", "overridden"] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for flag in [true, false, true] {
            let before = vm.bytecode_invocations();
            assert_eq!(
                vm.invoke(support::id(&package, name), vec![Value::Bool(flag)])
                    .unwrap(),
                Value::Bool(flag)
            );
            assert_eq!(vm.bytecode_invocations() - before, 2, "{name}");
        }
    }
    let mut forged = package.clone();
    forged
        .functions
        .iter_mut()
        .find(|f| f.name == "choose")
        .unwrap()
        .body
        .kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(true));
    let mut vm = Interpreter::for_comptime(&forged, Budget::default()).unwrap();
    for flag in [false, true, false, true] {
        let before = vm.bytecode_invocations();
        let result = vm.invoke(support::id(&forged, "run"), vec![Value::Bool(flag)]);
        if flag {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
        }
        assert_eq!(vm.bytecode_invocations() - before, 2);
    }
}

#[test]
fn dynamic_ordinary_instances_freeze_inputs_and_specialize_defaults_in_bytecode() {
    let package = support::compile(
        "comptime fn choose(.value:bool=flag)->bool where flag:bool {value}
        comptime fn select()->bool where flag:bool {flag}
        comptime fn run()->bool {
            var flag=true;
            let first=select<flag>;
            flag=false;
            let second=select<flag>;
            first() and not second() and not choose<flag>()
        }
        comptime fn early()->bool {choose<do {return true;}>()}",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (name, count) in [("run", 4), ("early", 1), ("run", 4)] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_invocations() - before, count, "{name}");
    }
}
