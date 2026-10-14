//! Interface behavior promises survive implementation, generic calls and IR.
use comptime_interpretor::Value;
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{compile, compile_with_protocols};

#[test]
fn pure_trait_contract_is_inherited_by_unmarked_implementations() {
    let package = compile("trait Flip {pure fn flip(self)->Self;} impl Flip for bool {fn flip(self)->Self {not self}} pure fn generic(value:T)->T where T :- Flip {value.flip()} fn run()->bool {generic(false)}");
    assert!(package.traits[0].methods[0].pure);
    assert!(package.traits[0].methods[0].to_string().contains("(pure true)"));
    assert!(package.functions.iter().find(|function| function.name == "flip").unwrap().pure);
    let graph = luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    assert!(graph.interfaces.iter().flat_map(|interface| &interface.methods)
        .any(|method| method.name == "flip" && method.pure));
    assert!(graph.functions.iter().any(|instance| instance.function.name == "flip" && instance.function.pure));
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn implementations_cannot_silently_weaken_a_pure_trait_promise() {
    for body in ["self.*", "self.* = true;true"] {
        let error = compile_with_protocols(&format!("trait Access {{pure fn access(self)->bool;}} impl Access for *bool {{fn access(self)->bool {{{body}}}}}"), &Default::default()).unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
}

#[test]
fn pure_default_methods_keep_the_contract_when_selected() {
    let package = compile("trait Negate {pure fn negate(self)->Self;pure fn twice(self)->Self {self.negate().negate()}} impl Negate for bool {fn negate(self)->Self {not self}} pure fn generic(value:T)->T where T :- Negate {value.twice()} fn run()->bool {generic(true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn pure_parent_methods_keep_their_origin_through_child_bounds() {
    let package = compile("trait Parent {pure fn keep(self)->Self;} trait Child :- Parent {} impl Parent for bool {fn keep(self)->Self {self}} impl Child for bool {} pure fn generic(value:T)->T where T :- Child {value.keep()} fn run()->bool {generic(true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn an_unmarked_interface_does_not_supply_symbolic_purity() {
    let source = "trait Flip {fn flip(self)->Self;} impl Flip for bool {pure fn flip(self)->Self {not self}} pure fn generic(value:T)->T where T :- Flip {value.flip()}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn a_pure_interface_does_not_grant_termination_to_defaults() {
    let source = "trait Flag {pure fn flag(self)->bool;} impl Flag for bool {fn flag(self)->bool {self.as(Flag).flag()}} fn take(value:bool,.flag:bool=value.as(Flag).flag())->bool {flag}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn pure_method_local_type_binders_survive_generic_dispatch() {
    let package = compile("trait Keep {pure fn keep(value:U)->U where U;} impl Keep for bool {fn keep(value:U)->U where U {value}} pure fn generic(value:U)->U where T :- Keep,U {T.keep(value)} fn run()->bool {generic<bool,bool>(true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn default_overrides_inherit_purity_and_comptime_keeps_its_phase() {
    let invalid = "trait Choose {pure fn choose(p:*bool)->bool {true}} impl Choose for bool {fn choose(p:*bool)->bool {p.* = true;true}}";
    assert!(compile_with_protocols(invalid, &Default::default()).unwrap_err().contains("P4 evidence"));
    let package = compile("trait Flip {pure comptime fn flip(value:bool)->bool;} impl Flip for bool {comptime fn flip(value:bool)->bool {not value}} comptime fn run()->bool {bool.as(Flip).flip(false)}");
    let implementation = package.functions.iter().find(|function| function.name == "flip").unwrap();
    assert!(implementation.pure);
    assert_eq!(implementation.phase, nir::program::FunctionPhase::Comptime);
    assert_eq!(luna::invocation::invoke(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn bodyless_non_trait_promises_still_require_a_contract() {
    let error = compile_with_protocols("pure fn foreign_read()->bool;", &Default::default()).unwrap_err();
    assert!(error.contains("bodyless function requires a trait contract"), "{error}");
}

#[test]
fn concrete_unmarked_methods_can_prove_behavior_from_their_bodies() {
    let package = compile("trait Flip {fn flip(self)->Self;} impl Flip for bool {fn flip(self)->Self {not self}} pure fn invert(value:bool)->bool {value.as(Flip).flip()} fn take(value:bool,.flipped:bool=value.as(Flip).flip())->bool {flipped} fn run()->bool {invert(false) and take(false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn concrete_default_method_chains_can_supply_finite_defaults() {
    let package = compile("trait Flip {fn flip(self)->Self;fn twice(self)->Self {self.flip().flip()}} impl Flip for bool {fn flip(self)->Self {not self}} fn take(value:bool,.same:bool=value.as(Flip).twice())->bool {same} fn run()->bool {take(true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn concrete_method_binders_are_substituted_for_behavior_analysis() {
    let package = compile("trait Keep {fn keep(value:U)->U where U;} impl Keep for bool {fn keep(value:U)->U where U {value}} fn take(value:bool,.same:bool=bool.as(Keep).keep(value))->bool {same} fn run()->bool {take(true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn selected_implementation_effects_and_cycles_still_reject_defaults() {
    for source in [
        "trait Flag {fn flag(self)->bool;} impl Flag for bool {fn flag(self)->bool {self.as(Flag).flag()}} fn take(value:bool,.flag:bool=value.as(Flag).flag())->bool {flag}",
        "trait Flag {fn flag(self)->bool;} impl Flag for bool {fn flag(self)->bool {while true {} true}} fn take(value:bool,.flag:bool=value.as(Flag).flag())->bool {flag}",
        "trait Read {fn read(self)->bool;} impl Read for *bool {fn read(self)->bool {self.*}} fn take(value:*bool,.flag:bool=value.as(Read).read())->bool {flag}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{error}");
    }
    let source = "trait Read {fn read(self)->bool;} impl Read for *bool {fn read(self)->bool {self.*}} pure fn read(value:*bool)->bool {value.as(Read).read()}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn constrained_blanket_implementation_and_default_override_select_correctly() {
    let package = compile("trait Flag {fn flag(self)->bool {false}} impl Flag for T where T, requires T == bool {fn flag(self)->bool {self}} fn take(value:bool,.flag:bool=value.as(Flag).flag())->bool {flag} fn run()->bool {take(true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn body_evidence_does_not_erase_the_interface_call_precondition() {
    let prefix = "typealias Byte=.u8'builtin;trait Next {fn next(value:Byte)->Byte where requires value < 255;} impl Next for Byte {fn next(value:Byte)->Byte {value + 1}}";
    let package = compile(&format!("{prefix} fn take(value:Byte,.next:Byte=if value < 255 {{Byte.as(Next).next(value)}} else {{value}})->Byte {{next}} fn run()->Byte {{take(40)}}"));
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::UnsignedInteger(41));
    let error = compile_with_protocols(&format!("{prefix} fn take(.next:Byte=Byte.as(Next).next(255))->Byte {{next}} fn run()->Byte {{take()}}"), &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
}
