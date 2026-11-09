//! Cold source validation must not rebuild declaration domains per closure use.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn cold_sysroot_phase_inspection_leaves_fuel_for_real_invocation() {
    let package = sysroot::compile("comptime fn answer() -> bool { true }").unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "answer")
        .unwrap()
        .declaration;
    let budget = Budget::default();
    let mut fuel = budget.steps as usize;
    nir::phases::Inspection::new(&package)
        .check_package(&mut fuel)
        .unwrap();
    let phase_cost = budget.steps - fuel as u64;
    let mut vm = Interpreter::for_comptime(&package, budget).unwrap();
    let construction_cost = budget.steps - vm.remaining_steps();
    assert!(construction_cost > phase_cost);
    assert_eq!(vm.invoke(entry, vec![]).unwrap(), Value::Bool(true));
    assert!(vm.remaining_steps() > 0);
    eprintln!(
        "cold sysroot: phase={phase_cost}, construction={construction_cost}, remaining after invocation={}",
        vm.remaining_steps()
    );
}
