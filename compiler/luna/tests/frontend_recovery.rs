//! Source mutations must be rejected or compiled, never panic the compiler.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn binary_and_type_proposition_mutations_recover_to_executable_sources() {
    let seeds = [
        ("comptime fn run()->bool {(20 - 3) * 2 + 1 == 35}", true),
        (
            "fn run()->bool {let flag=false;(do {let flag=true;flag}) and not flag}",
            false,
        ),
        (
            "typealias Checked=refines {not false and (Itself == bool or Itself == .u8'builtin)} bool;fn run()->Checked {true}",
            false,
        ),
    ];
    for (seed, comptime) in seeds {
        let execute = || {
            let package = support::compile_with_protocols(seed, &Default::default()).unwrap();
            let result = if comptime {
                luna::invocation::invoke(&package, "run", 100_000)
            } else {
                luna::execution::interpret(&package, "run", 100_000)
            };
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
        };
        execute();
        let mut rejected = 0;
        for (offset, character) in seed.char_indices() {
            for replacement in ["", "(", ")", "and", "or", "==", "-"] {
                let mut source = seed.to_owned();
                source.replace_range(offset..offset + character.len_utf8(), replacement);
                let result = std::panic::catch_unwind(|| {
                    support::compile_with_protocols(&source, &Default::default())
                });
                let result = result.unwrap_or_else(|_| panic!("compiler panicked: {source}"));
                rejected += usize::from(result.is_err());
            }
            // Restore and execute the real source after each group of malformed
            // inputs, including edits that failed after successful parsing.
            execute();
        }
        assert!(rejected > 0, "mutation seed did not exercise diagnostics");
    }
}

#[test]
fn declaration_pattern_and_staging_recovery_preserves_compiler_invariants() {
    let seeds = [
        "fn identity(value:T)->T where T {value} fn run()->bool {identity(true)}",
        "trait Read {fn read(self)->bool;} struct Flag {value:bool} impl Read for Flag {fn read(self)->bool {self.value}} fn run()->bool {Flag {value:true}.as(Read).read()}",
        "enum Choice {Some(bool),None} fn run(value:Choice)->bool {value match {.Some(flag)=>flag,.None=>false}}",
        "comptime fn code()-><|- bool> {'(true)} fn run()->bool {~<code()>}",
        "fn run(value:bool)->bool {const p:<bool -| found:bool> = '<found>;let ~[found=result]<p> = value;result}",
    ];
    for seed in seeds {
        support::compile_with_protocols(seed, &Default::default()).unwrap();
        for (offset, character) in seed.char_indices() {
            // Deleting and replacing each scalar tests adjacent delimiters and
            // tokens without ever splitting a UTF-8 encoding.
            for replacement in ["", ")", "}", ",", ":"] {
                let mut source = seed.to_owned();
                source.replace_range(offset..offset + character.len_utf8(), replacement);
                let result = std::panic::catch_unwind(|| {
                    support::compile_with_protocols(&source, &Default::default())
                });
                assert!(result.is_ok(), "compiler panicked for source: {source}");
            }
        }
    }
}

#[test]
fn dependent_declarations_and_protocol_recovery_never_panic() {
    let seeds = [
        "typealias Type=.Type'builtin;pure comptime fn Token<T,flag:bool>->Type {T} fn accept(value:Token<bool,true>)->Token<bool,true> {value}",
        "typealias Type=.Type'builtin;trait View where T {assoc Output:Type;fn view(self,value:T)->Output;} struct Flag {} impl View<bool> for Flag {assoc Output:Type=bool;fn view(self,value:bool)->bool {value}}",
        "fn select(value:bool,.fallback:bool=value)->bool {fallback} fn run()->bool {select(true)}",
        "fn unpack(value:Tuple<bool,Tuple<bool,bool>>)->bool {let (a,(b,c))=value;a and b and not c}",
        "fn constrained(value:T)->T where T,requires T == bool {value} fn run()->bool {constrained(false)}",
        "effect read()->bool;fn ask()->#read bool {read()#} fn run()->bool {ask()# {read()=>true}}",
        "fn run()->.str'builtin {\"中文🙂\"}",
    ];
    for seed in seeds {
        support::compile_with_tuple_protocols(seed, &Default::default()).unwrap();
        for (offset, character) in seed.char_indices() {
            let truncated = &seed[..offset];
            let result = std::panic::catch_unwind(|| {
                support::compile_with_tuple_protocols(truncated, &Default::default())
            });
            assert!(result.is_ok(), "compiler panicked at EOF: {truncated}");
            for replacement in ["", ")", "}", ",", ":"] {
                let mut source = seed.to_owned();
                source.replace_range(offset..offset + character.len_utf8(), replacement);
                let result = std::panic::catch_unwind(|| {
                    support::compile_with_tuple_protocols(&source, &Default::default())
                });
                assert!(result.is_ok(), "compiler panicked for source: {source}");
            }
        }
    }
}
