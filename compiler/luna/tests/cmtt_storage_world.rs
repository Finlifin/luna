//! Contextual-code storage and entry snapshots use the actual declaration world.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

#[test]
fn entry_snapshot_retains_list_domains_and_isolates_later_receiver_updates() {
    let source = "
        struct State { flag: bool, values: meta.List<bool> }
        impl State {
            comptime fn build(self) -> <|- bool>
            where ensures self.flag and not self'old.flag
            {
                self.values.append(false);
                self.flag = true;
                '(true)
            }
        }
        comptime fn outer() -> <|- bool> {
            let state = State { flag: false, values: [true] };
            const code = state.build();
            let changed = state.values.length() == 2;
            '(changed and ~<code>)
        }
        fn run() -> bool { ~<outer()> }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let failure =
        sysroot::compile(&source.replace("not self'old.flag", "self'old.flag")).unwrap_err();
    assert!(failure.contains("postcondition rejected"), "{failure}");
}

#[test]
fn nominal_object_entries_use_the_generator_world_when_snapshotting_a_receiver() {
    let source = "
        struct Payload { flag: bool }
        struct State { flag: bool, values: meta.Object }
        impl State {
            comptime fn build(self) -> <|- bool>
            where ensures self.flag and not self'old.flag
            {
                self.values = self.values.append(Payload { flag: false });
                self.flag = true;
                '(true)
            }
        }
        comptime fn outer() -> <|- bool> {
            let state = State { flag: false, values: { item: Payload { flag: true } } };
            const code = state.build();
            let changed = state.values.length() == 2;
            '(changed and ~<code>)
        }
        fn run() -> bool { ~<outer()> }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let failure =
        sysroot::compile(&source.replace("self.flag = true", "self.flag = false")).unwrap_err();
    assert!(failure.contains("postcondition rejected"), "{failure}");
}
