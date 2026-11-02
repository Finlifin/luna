//! Domain substitution must depend on the referenced inputs, not unrelated
//! declarations sharing the same source parameter names.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn dependent_domains_remain_distinct_in_a_large_generic_declaration_world() {
    let mut source = String::new();
    for index in 0..256 {
        source.push_str(&format!(
            "fn identity_{index}(value:T)->T\nwhere T\n{{ value }}\n"
        ));
    }
    source.push_str(
        r#"
trait Root where T, mode:T, .copy:T=mode {
    comptime fn selected(self)->bool;
}
trait Leaf :- Root<T,mode> where T, mode:T {
    comptime fn inherited(self)->bool { self.as(Root<T,mode>).selected() }
}
impl Root<bool,true> for bool {
    comptime fn selected(self)->bool { true }
}
impl Root<bool,false> for bool {
    comptime fn selected(self)->bool { false }
}
impl Leaf<bool,true> for bool {}
impl Leaf<bool,false> for bool {}
comptime fn selected()->bool {
    true.as(Leaf<bool,true>).inherited()
        and not true.as(Leaf<bool,false>).inherited()
}
fn run()->bool {
    const result = selected();
    identity_0<bool>(identity_255<bool>(result))
}
"#,
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    // A populated declaration world must not hide the selected Type domain,
    // nor let another owner's equally spelled parameter authorize this value.
    let invalid = source.replace("Leaf<bool,false>", "Leaf<bool,'x'>");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("type mismatch"), "{failure}");
}
