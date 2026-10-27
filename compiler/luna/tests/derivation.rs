#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn explicit_strategy_methods_execute_for_independent_subjects() {
    let package = support::compile(
        "trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {fn flag(self)->bool {true}} struct First {} struct Second {} derive Flag.use(enabled) for First; derive Flag.use(enabled) for Second; fn run()->bool {First{}.as(Flag).flag() and Second{}.as(Flag).flag()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(package.implementations.len(), 2);
}

#[test]
fn strategy_applicability_checks_the_complete_interface_instance() {
    let source = "typealias u32 = .u32'builtin; trait Flag where T {fn flag(self)->bool;} derive fn enabled for Flag<bool> {fn flag(self)->bool {true}} struct Record {} derive Flag<u32>.use(enabled) for Record;";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("derivation strategy does not match the requested interface instance"),
        "{error}"
    );
}

#[test]
fn generated_implementations_still_check_members_and_overlap() {
    for (source, expected) in [
        (
            "typealias u32=.u32'builtin; trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {fn flag(self)->u32 {1}} struct Record {} derive Flag.use(enabled) for Record;",
            "does not match its trait signature",
        ),
        (
            "trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {} struct Record {} derive Flag.use(enabled) for Record;",
            "missing required trait method",
        ),
        (
            "trait Flag {fn flag(self)->bool;} derive fn enabled for Flag {fn flag(self)->bool {true}} struct Record {} derive Flag.use(enabled) for Record; derive Flag.use(enabled) for Record;",
            "implementation overlap",
        ),
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains(expected),
            "ordinary impl checking was not reached: {error}"
        );
    }
}

#[test]
fn non_strategy_selections_and_generation_statements_are_not_silently_accepted() {
    for source in [
        "trait Flag {} fn enabled() {} struct Record {} derive Flag.use(enabled) for Record;",
        "trait Flag {} derive fn enabled for Flag {var value = true;} struct Record {} derive Flag.use(enabled) for Record;",
        "trait Flag {} derive fn enabled(flag:bool) for Flag {} struct Record {} derive Flag.use(enabled) for Record;",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("strategy") || error.contains("derivation expansion"),
            "{error}"
        );
    }
}

#[test]
fn matching_parameterized_strategy_executes_without_unfolding_trait_identity() {
    let package = support::compile(
        "trait Flag where T {fn flag(self)->bool;} derive fn enabled for Flag<bool> {fn flag(self)->bool {true}} struct Record {} derive Flag<bool>.use(enabled) for Record; fn run()->bool {Record{}.as(Flag<bool>).flag()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_derivation_preserves_membership_and_function_phases() {
    let source = format!(
        "mod attrs {{{}}} use attrs.comptime_phase as staged; typealias Type=.Type'builtin; trait Flag {{^staged comptime fn flag()->bool;}} derive fn enabled for Flag {{comptime fn flag()->bool {{true}}}} struct Record {{}} comptime derive Flag.use(enabled) for Record; comptime fn run()->bool {{Record.as(Flag).flag()}}",
        include_str!("../../../library/attrs/lib.fl")
    );
    let package = support::compile(&source);
    assert!(package.implementations[0].phases.comptime);
    assert!(!package.implementations[0].phases.runtime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_generic_methods_rebind_their_own_type_parameters() {
    let package = support::compile(
        "trait Echo {fn echo(value:T)->T where T;} derive fn echoing for Echo {fn echo(value:T)->T where T {value}} struct Subject {} derive Echo.use(echoing) for Subject; fn run()->bool {Subject.as(Echo).echo<bool>(true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn strategy_matching_preserves_normal_form_heads() {
    let source = "pure comptime fn Identity<T> -> .Type'builtin {T} trait Flag where T {} derive fn enabled for Flag<Identity<bool>> {} struct Subject {} derive Flag<bool>.use(enabled) for Subject;";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("derivation strategy does not match the requested interface instance"),
        "{error}"
    );
}

#[test]
fn block_local_requests_do_not_leave_retired_item_references() {
    let source = "trait Flag {} derive fn enabled for Flag {} struct Record {} fn run()->bool {derive Flag.use(enabled) for Record; true}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("block-local derive requests"), "{error}");
}

#[test]
fn generated_associated_types_and_values_feed_method_signatures_and_bodies() {
    let package = support::compile(
        "typealias Type=.Type'builtin; trait Source {assoc Item:Type;assoc initial:Item;fn make()->Item;} derive fn booleans for Source {fn make()->Item {initial} assoc initial:Item=Self == First;assoc Item:Type=bool;} struct First {} struct Second {} derive Source.use(booleans) for First; derive Source.use(booleans) for Second; fn run()->bool {First.as(Source).make() and not Second.as(Source).make()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(package.implementations.len(), 2);
}

#[test]
fn generated_associated_values_follow_comptime_membership() {
    let source = format!(
        "mod attrs {{{}}} use attrs.comptime_phase as staged; typealias Type=.Type'builtin; trait Source {{assoc Item:Type=bool; ^staged assoc initial:bool; ^staged comptime fn make()->Item;}} derive fn booleans for Source {{assoc initial:bool=true; comptime fn make()->bool {{initial}}}} struct Subject {{}} comptime derive Source.use(booleans) for Subject; comptime fn run()->bool {{Subject.as(Source).make()}}",
        include_str!("../../../library/attrs/lib.fl")
    );
    let package = support::compile(&source);
    assert!(package.implementations[0].phases.comptime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_associated_contract_failures_use_ordinary_checks() {
    for source in [
        "typealias Type=.Type'builtin;trait Source {assoc Item:Type;} derive fn invalid for Source {assoc Item:bool=true;} struct Subject {} derive Source.use(invalid) for Subject;",
        "typealias Type=.Type'builtin;trait Source {assoc Item:Type;assoc initial:Item;} derive fn invalid for Source {assoc Item:Type=bool;} struct Subject {} derive Source.use(invalid) for Subject;",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("associated") && !error.contains("not implemented"),
            "{error}"
        );
    }
}

#[test]
fn generated_associated_self_types_remain_distinct_between_subjects() {
    let package = support::compile(
        "typealias Type=.Type'builtin; trait IdentityView {assoc Item:Type;fn echo(value:Item)->Item;} derive fn identity_view for IdentityView {assoc Item:Type=Self;fn echo(value:Item)->Item {value}} struct First {} struct Second {} derive IdentityView.use(identity_view) for First; derive IdentityView.use(identity_view) for Second; fn first(value:First)->bool {true} fn second(value:Second)->bool {true} fn run()->bool {first(First.as(IdentityView).echo(First{})) and second(Second.as(IdentityView).echo(Second{}))}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_prefix_attributes_evaluate_in_the_impl_self_context() {
    let source = format!(
        "mod attrs {{{}}} use attrs.comptime_phase as staged; typealias Type=.Type'builtin; comptime fn metadata(T:Type)->.Object'builtin {{let value={{comptime_phase:T == Subject}};value}} trait Flag {{^staged comptime fn flag()->bool;}} derive fn enabled for Flag {{^metadata(Self) comptime fn flag()->bool {{true}}}} struct Subject {{}} comptime derive Flag.use(enabled) for Subject; comptime fn run()->bool {{Subject.as(Flag).flag()}}",
        include_str!("../../../library/attrs/lib.fl")
    );
    let package = support::compile(&source);
    assert!(package.implementations[0].phases.comptime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_prefix_attributes_preserve_override_order() {
    let source = "const staged={comptime_phase:true}; const ordinary={comptime_phase:false}; trait Flag {comptime fn flag()->bool;} derive fn enabled for Flag {^ordinary ^staged comptime fn flag()->bool {true}} struct Subject {} derive Flag.use(enabled) for Subject; comptime fn run()->bool {Subject.as(Flag).flag()}";
    let package = support::compile(source);
    assert!(package.implementations[0].phases.runtime);
    assert!(!package.implementations[0].phases.comptime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_attributes_still_require_an_implemented_consumer() {
    let source = format!(
        "mod attrs {{{}}} trait Flag {{fn flag()->bool;}} derive fn enabled for Flag {{^attrs.abi(.c) fn flag()->bool {{true}}}} struct Subject {{}} derive Flag.use(enabled) for Subject;",
        include_str!("../../../library/attrs/lib.fl")
    );
    let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        error.contains("definition attribute or modifier `abi`"),
        "{error}"
    );
}

#[test]
fn generic_requests_rebind_subjects_and_generated_self_independently() {
    let package = support::compile(
        "typealias u32=.u32'builtin; trait Echo {fn echo(value:Self)->Self;} derive fn identity for Echo {fn echo(value:Self)->Self {value}} struct First where T {} struct Second where T {} derive Echo.use(identity) for First<T> where T; derive Echo.use(identity) for Second<T> where T; fn first(value:First<bool>)->bool {true} fn second(value:Second<u32>)->bool {true} fn run()->bool {first(First<bool>.as(Echo).echo(First<bool>{})) and second(Second<u32>.as(Echo).echo(Second<u32>{}))}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_request_requirements_control_implementation_availability() {
    for clauses in ["where T:-Bound", "where T, requires T:-Bound"] {
        let source = format!(
            "trait Bound {{}} struct Allowed {{}} impl Bound for Allowed {{}} struct Denied {{}} trait Flag {{fn flag()->bool;}} derive fn enabled for Flag {{fn flag()->bool {{true}}}} struct Subject where T {{}} derive Flag.use(enabled) for Subject<T> {clauses}; fn run()->bool {{Subject<Allowed>.as(Flag).flag()}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let rejected = source.replace("Subject<Allowed>.as", "Subject<Denied>.as");
        let error = support::compile_with_protocols(&rejected, &Default::default()).unwrap_err();
        assert!(!error.contains("not implemented"), "{error}");
        assert!(
            error.contains("implementation") || error.contains("obligation"),
            "{error}"
        );
    }
}

#[test]
fn value_parameterized_requests_preserve_subject_identity() {
    let source = "trait Echo {fn echo(value:Self)->Self;} derive fn identity for Echo {fn echo(value:Self)->Self {value}} struct Subject where enabled:bool {} derive Echo.use(identity) for Subject<enabled> where enabled:bool; fn accept(value:Subject<true>)->bool {true} fn run()->bool {accept(Subject<true>.as(Echo).echo(Subject<true>{}))}";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let rejected = source.replace(
        "accept(Subject<true>.as(Echo).echo(Subject<true>{}))",
        "accept(Subject<false>.as(Echo).echo(Subject<false>{}))",
    );
    let error = support::compile_with_protocols(&rejected, &Default::default()).unwrap_err();
    assert!(!error.contains("not implemented"), "{error}");
}

#[test]
fn generic_requests_preserve_comptime_implementation_phase() {
    let source = "trait Flag {^{comptime_phase:true} comptime fn flag()->bool;} derive fn enabled for Flag {comptime fn flag()->bool {true}} struct Subject where T {} comptime derive Flag.use(enabled) for Subject<T> where T; comptime fn run()->bool {Subject<bool>.as(Flag).flag()}";
    let package = support::compile(source);
    assert!(package.implementations[0].phases.comptime);
    assert!(!package.implementations[0].phases.runtime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_member_errors_preserve_strategy_and_all_use_sites() {
    let source = "trait Flag {fn flag()->bool;} derive fn invalid for Flag {fn flag()->bool {()}} struct First {} struct Second {} derive Flag.use(invalid) for First; derive Flag.use(invalid) for Second;";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("derivation strategy `invalid` defined here"),
        "{error}"
    );
    assert!(
        error.contains("generated member `flag` originates here"),
        "{error}"
    );
    assert_eq!(
        error
            .matches("derive use site of strategy `invalid`")
            .count(),
        2,
        "{error}"
    );
    for request in [
        "derive Flag.use(invalid) for First",
        "derive Flag.use(invalid) for Second",
    ] {
        let start = source.find(request).unwrap();
        let end = start + request.len();
        assert!(error.lines().any(|line| {
            let Some(note) = line.trim().strip_prefix("note: ") else { return false };
            let Some((range, message)) = note.split_once(": ") else { return false };
            if !message.starts_with("derive use site") { return false; }
            let Some((lo, hi)) = range.split_once("..") else { return false };
            matches!((lo.parse::<usize>(), hi.parse::<usize>()), (Ok(lo), Ok(hi)) if lo <= start && end <= hi)
        }), "{error}");
    }
}

#[test]
fn expansion_failures_report_sources_but_unrelated_errors_do_not() {
    let invalid = "trait Flag {} derive fn invalid(value:bool) for Flag {} struct Subject {} derive Flag.use(invalid) for Subject;";
    let error = support::compile_with_protocols(invalid, &Default::default()).unwrap_err();
    assert!(
        error.contains("derivation strategy input count mismatch"),
        "{error}"
    );
    assert!(
        error.contains("derivation strategy `invalid` defined here"),
        "{error}"
    );
    assert!(
        error.contains("derive use site of strategy `invalid`"),
        "{error}"
    );

    let unrelated = "trait Flag {} derive fn valid for Flag {} struct Subject {} derive Flag.use(valid) for Subject; fn bad()->bool {()}";
    let error = support::compile_with_protocols(unrelated, &Default::default()).unwrap_err();
    assert!(!error.contains("derivation strategy"), "{error}");
    assert!(!error.contains("derive use site"), "{error}");
}

#[test]
fn generated_methods_read_each_subjects_receiver_fields() {
    let package = support::compile(
        "trait Read {fn read(self)->bool;} derive fn reading for Read {fn read(self)->bool {self.value}} struct First {value:bool} struct Second {value:bool} derive Read.use(reading) for First; derive Read.use(reading) for Second; fn run()->bool {First{value:true}.as(Read).read() and not Second{value:false}.as(Read).read()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_borrowed_receivers_write_the_original_subject() {
    let package = support::compile(
        "trait Update {fn set(*self,value:bool);} derive fn updating for Update {fn set(*self,value:bool) {self.*.value = value;}} struct Subject {value:bool} derive Update.use(updating) for Subject; fn run()->bool {let subject=Subject{value:false}; Subject.as(Update).set(subject.ref,true);subject.value}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_comptime_methods_can_return_their_receiver() {
    let package = support::compile(
        "trait Keep {^{comptime_phase:true} comptime fn keep(self)->Self;} derive fn keeping for Keep {comptime fn keep(self)->Self {self}} struct Subject {value:bool} comptime derive Keep.use(keeping) for Subject; comptime fn run()->bool {Subject{value:true}.as(Keep).keep().value}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_self_without_a_receiver_uses_the_ordinary_diagnostic() {
    let source = "trait Flag {fn flag()->bool;} derive fn invalid for Flag {fn flag()->bool {self}} struct Subject {} derive Flag.use(invalid) for Subject;";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("self requires a method receiver"), "{error}");
    assert!(
        error.contains("generated member `flag` originates here"),
        "{error}"
    );
}

#[test]
fn strategy_inputs_are_independent_comptime_values_for_runtime_methods() {
    let package = compile_with_strategy_bool_splice(
        "trait Flag {fn flag()->bool;} comptime fn invert(value:bool)->bool {not value} derive fn configured(value:bool) for Flag {fn flag()->bool {value}} struct First {} struct Second {} derive Flag.use(configured(invert(false))) for First; derive Flag.use(configured(false)) for Second; fn run()->bool {First.as(Flag).flag() and not Second.as(Flag).flag()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn strategy_input_types_contextualize_literals_and_feed_associated_members() {
    let package = support::compile(
        "typealias u32=.u32'builtin;trait Code {assoc code:u32;fn get()->u32;} derive fn configured(input:u32) for Code {assoc code:u32=input;fn get()->u32 {code}} struct Subject {} derive Code.use(configured(7)) for Subject; fn run()->bool {Subject.as(Code).get() == 7}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unused_strategy_inputs_are_still_checked_and_invoked_at_comptime() {
    for argument in ["()", "runtime_value()"] {
        let source = format!(
            "trait Flag {{fn flag()->bool;}} fn runtime_value()->bool {{true}} derive fn configured(ignored:bool) for Flag {{fn flag()->bool {{true}}}} struct Subject {{}} derive Flag.use(configured({argument})) for Subject;"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(!error.contains("not implemented"), "{error}");
        assert!(
            error.contains("type mismatch") || error.contains("runtime function"),
            "{error}"
        );
    }
}

#[test]
fn strategy_input_parameters_can_depend_on_preceding_type_inputs() {
    let package = compile_with_strategy_bool_splice(
        "typealias Type=.Type'builtin;trait Source {assoc Item:Type;fn make()->Item;} derive fn configured(T:Type,value:T) for Source {assoc Item:Type=T;fn make()->Item {value}} struct Subject {} derive Source.use(configured(bool,true)) for Subject; fn run()->bool {Subject.as(Source).make()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn strategy_inputs_are_available_to_prefix_attributes_and_comptime_methods() {
    let source = "trait Flag {^{comptime_phase:true} comptime fn flag()->bool;} derive fn configured(staged:bool,value:bool) for Flag {^{comptime_phase:staged} comptime fn flag()->bool {value}} struct Subject {} comptime derive Flag.use(configured(true,true)) for Subject; comptime fn run()->bool {Subject.as(Flag).flag()}";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn strategy_inputs_participate_in_complete_interface_matching() {
    let source = "trait Flag where enabled:bool {fn flag()->bool;} derive fn configured(enabled:bool) for Flag<enabled> {fn flag()->bool {enabled}} struct Subject {} derive Flag<true>.use(configured(true)) for Subject; fn run()->bool {Subject.as(Flag<true>).flag()}";
    let package = compile_with_strategy_bool_splice(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace(".use(configured(true))", ".use(configured(false))");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        error.contains("does not match the requested interface instance"),
        "{error}"
    );
}

#[test]
fn strategy_input_call_shapes_are_not_silently_discarded() {
    for source in [
        "trait Flag {} derive fn configured(value:bool) for Flag {} struct Subject {} derive Flag.use(configured(true,false)) for Subject;",
        "trait Flag {} derive fn configured(value:bool) for Flag {} struct Subject {} derive Flag.use(configured(.value=true)) for Subject;",
        "trait Flag {} derive fn configured(lambda value:fn()->bool) for Flag {} struct Subject {} derive Flag.use(configured(||true)) for Subject;",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("strategy input") || error.contains("strategy parameter"),
            "{error}"
        );
        assert!(!error.contains("parsing failed"), "{error}");
    }
}

fn compile_with_strategy_bool_splice(source: &str) -> nir::program::Package {
    let source = format!(
        "trait Splice {{assoc RuntimeTarget:.Type'builtin;fn splice(comptime data:Self)->RuntimeTarget;}} impl Splice for bool {{assoc RuntimeTarget:.Type'builtin=bool;fn splice(comptime data:bool)->bool {{data}}}} {source}"
    );
    support::compile_with_bindings(&source, &Default::default(), Some("Splice")).unwrap()
}

#[test]
fn runtime_strategy_input_use_still_requires_splice() {
    let source = "trait Flag {fn flag()->bool;} derive fn configured(value:bool) for Flag {fn flag()->bool {value}} struct Subject {} derive Flag.use(configured(true)) for Subject;";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("bound Splice protocol"), "{error}");
}

#[test]
fn many_strategy_inputs_share_one_large_completed_input_group() {
    let mut parameters = vec!["payload:.str'builtin".to_owned()];
    let mut arguments = vec![format!("\"{}\"", "x".repeat(16384))];
    for index in 0..64 {
        parameters.push(format!("flag_{index}:bool"));
        arguments.push(if index == 63 { "false" } else { "true" }.into());
    }
    let source = format!(
        "trait Flag {{fn flag()->bool;}} derive fn configured({}) for Flag {{fn flag()->bool {{flag_0 and not flag_63}}}} struct Subject {{}} derive Flag.use(configured({})) for Subject; fn run()->bool {{Subject.as(Flag).flag()}}",
        parameters.join(","),
        arguments.join(",")
    );
    let package = compile_with_strategy_bool_splice(&source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn strategy_local_values_compute_in_order_and_are_captured_by_methods() {
    let source = "trait Flag {fn flag()->bool;} derive fn configured(input:bool) for Flag {let inverted=not input; const result:bool=not inverted; fn flag()->bool {result}} struct First {} struct Second {} derive Flag.use(configured(true)) for First; derive Flag.use(configured(false)) for Second; fn run()->bool {First.as(Flag).flag() and not Second.as(Flag).flag()}";
    let package = compile_with_strategy_bool_splice(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn zero_input_strategy_locals_supply_associated_types_and_attributes() {
    let source = "typealias Type=.Type'builtin; trait Flag {assoc Item:Type;^{comptime_phase:true} comptime fn flag()->Item;} derive fn configured for Flag {const T:Type=bool; let staged=true; let result=not false; assoc Item:Type=T; ^{comptime_phase:staged} comptime fn flag()->Item {result}} struct Subject {} comptime derive Flag.use(configured) for Subject; comptime fn run()->bool {Subject.as(Flag).flag()}";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unused_strategy_locals_are_checked_without_accepting_mutation() {
    for (body, expected) in [
        ("let unused:bool=();", "type mismatch"),
        ("let unused=runtime_value();", "runtime function"),
        (
            "var state=true;",
            "mutable or destructuring strategy bindings",
        ),
        (
            "let state=true;state=false;",
            "strategy statement execution",
        ),
    ] {
        let source = format!(
            "trait Flag {{fn flag()->bool;}} fn runtime_value()->bool {{true}} derive fn configured for Flag {{{body} fn flag()->bool {{true}}}} struct Subject {{}} derive Flag.use(configured) for Subject;"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn strategy_temporary_references_do_not_escape_into_generated_members() {
    let package = compile_with_strategy_bool_splice(
        "trait Flag {fn flag()->bool;} derive fn configured for Flag {let storage=true; let pointer=storage.ref; let result=pointer.*; fn flag()->bool {result}} struct Subject {} derive Flag.use(configured) for Subject; fn run()->bool {Subject.as(Flag).flag()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_local_annotations_keep_transitive_type_dependencies() {
    let package = compile_with_strategy_bool_splice(
        "typealias Type=.Type'builtin; trait Flag {fn flag()->bool;} derive fn configured for Flag {const T:Type=bool; let result:T=true; fn flag()->bool {result}} struct Subject {} derive Flag.use(configured) for Subject; fn run()->bool {Subject.as(Flag).flag()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
