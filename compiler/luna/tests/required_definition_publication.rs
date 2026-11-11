//! A required invocation delivers its entire generated world to later consumers.
use comptime_interpretor::Value;
#[path = "support/sysroot_execution.rs"]
mod sysroot;

#[test]
fn required_builders_deliver_cross_kind_layouts_to_reflection_and_dispatch() {
    assert_eq!(
        sysroot::run(
            "
            comptime fn Build() -> Type {
                let wrapper = meta.NewtypeBuilder.new(bool, .name = .Flag).Build();
                let record = meta.StructBuilder.new(.name = .Record);
                record.add_field(.flag, wrapper);
                let choice = meta.EnumBuilder.new(.name = .Choice);
                choice.add_tuple(.some, [record.Build()]);
                choice.Build()
            }
            typealias Choice = Build();
            comptime fn inspect(subject: Type) -> bool {
                let record = subject.enum_variant_field_types()[0][0];
                let wrapper = record.struct_field_types()[0];
                wrapper.NewtypeRepresentation() == bool
            }
            trait Ready { fn ready() -> bool; }
            impl Ready for Choice {
                fn ready() -> bool { true }
            }
            fn run() -> bool {
                const reflected = inspect(Choice);
                reflected and Choice.as(Ready).ready()
            }
            "
        ),
        Value::Bool(true)
    );
}
