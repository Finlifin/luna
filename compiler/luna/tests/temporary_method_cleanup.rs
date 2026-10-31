//! Static method lookup must not replace a live handler-input temporary.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn mapper_temporary_receiver_keeps_handler_input_cleanup_identity() {
    let source = "typealias Unit=.void'builtin;
        typealias Count=.u32'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for Count {}
        impl Duplicate for *T where T {}
        inline effect emit(value:bool);
        trait Stream {inline fn produce(self)->#emit Unit;}
        struct Pair {first:bool,second:bool}
        impl Stream for Pair {
            inline fn produce(self)->#emit Unit {
                emit(self.first)#;emit(self.second)#;
            }
        }
        inline fn expand(iter:I,f:%c fn(bool)->J)->#emit Unit
            where I:-Stream,J:-Stream,c:(.Type'builtin).vars.Closure {
            iter.as(Stream).produce()# {
                emit(value)=>f(value).as(Stream).produce(),
            }
        }
        fn run()->bool {
            var count:Count=0;let observed=count.ref;
            expand(Pair {first:false,second:true},
                |value| Pair {first:value,second:not value})# {
                emit(value)=>{
                    assumes observed.* < 4;
                    observed.* = observed.* + 1;
                },
            };
            count == 4
        }";
    let package = support::compile(source);
    let mut expanded = package.clone();
    elaboration::program::inlining::expand(&mut expanded, &mut 1_000_000).unwrap();
    for package in [&package, &expanded] {
        assert_eq!(
            luna::execution::interpret_with_protocols(
                package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000,
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}
