//! Source receivers retain their storage identity across comptime calls.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

const METHODS: &str = "
    struct Cell { value: bool }
    struct Pair { left: Cell, right: Cell }
    impl Cell {
        comptime fn replace(self, value: bool) { self = Cell { value: value }; }
        comptime fn set_field(self, value: bool) { self.value = value; }
        comptime fn forward(self, value: bool) { self.replace(value); }
        comptime fn read(self) -> bool { self.value }
        comptime fn make(self) -> Cell { self.value = not self.value; Cell{value:false} }
        comptime fn collect(self, ...values: T) where T { self.replace(true); }
    }
";

fn invoke(body: &str) -> Result<Value, String> {
    let package = support::compile(&format!("{METHODS} comptime fn run()->bool {{ {body} }}"));
    luna::invocation::invoke(&package, "run", 1_000_000).map_err(|error| error.to_string())
}

#[test]
fn whole_receiver_updates_and_forwarding_reach_the_callers_slot() {
    assert_eq!(
        invoke("var cell=Cell{value:false}; cell.forward(true); cell.read()").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn let_receiver_can_read_and_update_fields_but_cannot_be_rebound() {
    assert_eq!(
        invoke("let cell=Cell{value:false}; cell.set_field(true); cell.read()").unwrap(),
        Value::Bool(true)
    );
    assert!(invoke("let cell=Cell{value:false}; cell.forward(true); cell.read()").is_err());
    assert!(invoke("let cell=Cell{value:false}; cell.collect(true); cell.read()").is_err());
    assert!(invoke("let cell=Cell{value:false}; Cell.collect(cell,true); cell.read()").is_err());
}

#[test]
fn projected_receivers_preserve_siblings_and_value_snapshots() {
    assert_eq!(invoke("let pair=Pair{left:Cell{value:false}, right:Cell{value:false}}; let old=pair.left; pair.left.forward(true); pair.left.value and not pair.right.value and not old.value").unwrap(), Value::Bool(true));
}

#[test]
fn explicit_receiver_call_and_temporary_receiver_use_the_same_transport() {
    assert_eq!(invoke("var cell=Cell{value:false}; Cell.replace(cell,true); cell.read() and not Cell{value:false}.read()").unwrap(), Value::Bool(true));
}

#[test]
fn collected_arguments_preserve_direct_and_projected_receivers() {
    assert_eq!(invoke("var cell=Cell{value:false}; cell.collect(false,true); let pair=Pair{left:Cell{value:false},right:Cell{value:false}}; pair.left.collect(true); cell.value and pair.left.value").unwrap(), Value::Bool(true));
    assert_eq!(
        invoke("var cell=Cell{value:false}; Cell.collect(cell,false,true); cell.value").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn collected_call_evaluates_a_computed_receiver_once_before_inputs() {
    assert_eq!(
        invoke("var cell=Cell{value:false}; var observed=false; cell.make().collect(do {observed=cell.value; true}); observed and cell.value").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_parameters_still_have_value_semantics() {
    let package = support::compile(&format!(
        "{METHODS}
        comptime fn replace(cell:Cell) {{ cell = Cell{{value:true}}; }}
        comptime fn run()->bool {{var cell=Cell{{value:false}};replace(cell);not cell.value}}"
    ));
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn trait_receivers_write_back_to_the_selected_subject() {
    let package = support::compile(
        "struct Cell {value:bool}
        trait Set { ^{comptime_phase:true} comptime fn set(self); }
        comptime impl Set for Cell { comptime fn set(self) {self.value=true;} }
        comptime fn run()->bool {let cell=Cell{value:false};cell.as(Set).set();cell.value}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn compound_receiver_fields_preserve_old_operand_and_rhs_writes() {
    let package = support::compile(
        "typealias Integer = .Integer'builtin;
        struct Counter {value: Integer, calls: Integer}
        struct Pair {left: Counter, right: Counter}
        impl Counter {
            comptime fn step(self) {
                self.value += do {self.value = 100; self.calls += 1; 3};
                self.value *= 2;
                self.value -= 1;
            }
            comptime fn produce(self) -> Counter {
                self.calls += 1;
                Counter {value: 2, calls: 0}
            }
            comptime fn read(self) -> Integer {self.value}
        }
        impl Pair {
            comptime fn nested(self) {
                self.left.value += do {self.right.value += 4; 2};
            }
        }
        comptime fn run() -> bool {
            let counter = Counter {value: 2, calls: 0};
            let saved = counter;
            counter.step();
            let pair = Pair {left: counter, right: saved};
            pair.nested();
            let temporary = counter.produce();
            temporary.step();
            counter.produce().step();
            counter.value == 9 and counter.calls == 3
                and saved.value == 2 and saved.calls == 0
                and pair.left.value == 11 and pair.right.value == 6
                and temporary.read() == 9
        }",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_pointer_receivers_are_rejected_before_any_invocation() {
    for source in [
        "struct Cell {value:bool} impl Cell {comptime fn get(*self)->bool {self.value.*}}",
        "trait Read {comptime fn get(*self)->bool;}",
        "struct Cell {value:bool} impl Cell {comptime fn build(*self)-><|-bool> {'(true)}}",
        "struct Cell {value:bool} impl Cell {comptime fn pattern(*self)-><bool -| value:bool> {'<value>}}",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("comptime methods use value-reference self"),
            "{error}"
        );
    }
}

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

#[test]
fn packet_and_property_inputs_preserve_receiver_storage_and_order() {
    use comptime_interpretor::Budget;
    let package = sysroot::compile(
        "
        struct Cell { value: bool }
        struct Pair { left: Cell, right: Cell }
        comptime fn packet(ready: bool) -> Tuple<bool, bool> { assumes ready; (true, true) }
        comptime fn options(...properties: T) -> T where T { properties }
        const SAMPLE = options { flag: true };
        typealias Options = SAMPLE'type;
        impl Cell {
            comptime fn replace(self, a: bool, b: bool) { self = Cell { value: a and b }; }
            comptime fn collect(self, ...items: Tuple<bool, bool>) {
                let (a, b) = items; self.replace(a, b);
            }
            comptime fn props(self, ...properties: Options) { self.value = properties.flag; }
            comptime fn mixed(self, ...items: Tuple<bool, bool>, ...properties: Options) {
                let (a, b) = items; self.value = a and b and properties.flag;
            }
            comptime fn produce(self) -> Cell { self.value = not self.value; Cell { value: false } }
        }
        comptime fn check() -> bool {
            var cell = Cell { value: false };
            cell.replace(...packet(true));
            if not cell.value { return false; };
            cell.value = false;
            Cell.replace(cell, ...packet(true));
            if not cell.value { return false; };
            cell.value = false;
            Cell.collect(cell, ...packet(true));
            if not cell.value { return false; };
            let pair = Pair { left: cell, right: cell };
            pair.left.props { flag: false };
            if pair.left.value or not pair.right.value { return false; };
            var ready = false;
            pair.left.mixed { flag: do {ready = true; true}, ...packet(ready) };
            if not pair.left.value { return false; };
            cell.value = false;
            cell.produce().replace(...packet(cell.value));
            cell.value
        }
        comptime fn exits() -> bool {
            var cell = Cell { value: false };
            cell.produce().replace(...do { return cell.value; packet(false) });
            false
        }
        comptime fn forbidden() -> bool {
            let cell = Cell { value: false };
            Cell.replace(cell, ...packet(true));
            true
        }
    ",
    )
    .unwrap();
    let check = sysroot::source_function(&package, "check");
    let forbidden = sysroot::source_function(&package, "forbidden");
    let exits = sysroot::source_function(&package, "exits");
    for tree in [false, true] {
        sysroot::with_comptime(
            &package,
            Budget {
                steps: 1_000_000,
                depth: 128,
            },
            tree,
            |vm| {
                let failure = vm.invoke(forbidden, vec![]).unwrap_err();
                assert_eq!(
                    failure.kind,
                    comptime_interpretor::ErrorKind::InvalidValueReference
                );
                assert_eq!(
                    vm.invoke(exits, vec![]).unwrap(),
                    Value::Bool(true),
                    "tree={tree}"
                );
                assert_eq!(
                    vm.invoke(check, vec![]).unwrap(),
                    Value::Bool(true),
                    "tree={tree}"
                );
            },
        );
    }
}
