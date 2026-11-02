//! Qualified values compose shipped ordering without inspecting their layout.
use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use luna::{
    cli::{self, Command},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-qualified-ordering-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("check command")
    };
    let result = frontend::compile(&options);
    std::fs::remove_file(&path).unwrap();
    match result {
        Ok(Artifact::Nir(package)) => Ok(package),
        Ok(_) => panic!("NIR artifact"),
        Err(error) => Err(error.to_string()),
    }
}

#[test]
fn qualified_ordering_uses_public_protocols_in_both_engines() {
    let package = compile(
        r#"
        struct Key { rank: u8, ignored: bool }
        comptime impl ComptimeEq for Key {
            comptime fn eq(self, other: Self) -> bool { self.rank == other.rank }
        }
        comptime impl ComptimeOrd for Key {
            comptime fn cmp(self, other: Self) -> Order {
                assumes self.rank != 99 and other.rank != 99;
                let left = self.rank;
                let right = other.rank;
                self.rank = 0;
                other.rank = 0;
                left.cmp(right)
            }
        }
        comptime fn optional() -> bool {
            let absent: ?u8 = null;
            let small: ?u8 = 1;
            let large: ?u8 = 2;
            let outer_absent: ??u8 = null;
            let inner_absent: ?u8 = null;
            let present_absent: ??u8 = inner_absent;
            let nested_small: ??u8 = small;
            absent.lt(small) and small.gt(absent) and absent.eq(absent)
                and absent.le(absent) and absent.ge(absent) and small.lt(large)
                and (small.cmp(small) matches .equal)
                and outer_absent.lt(present_absent) and present_absent.lt(nested_small)
        }
        comptime fn errors() -> bool {
            let first: !u8 u8 = 1;
            let second: !u8 u8 = 2;
            let failure: !u8 u8 = error 1;
            let later: !u8 u8 = error 2;
            first.lt(second) and second.lt(failure) and failure.lt(later)
                and later.gt(first) and failure.ge(failure) and first.le(first)
                and (first.cmp(first) matches .equal)
                and (failure.cmp(failure) matches .equal)
        }
        comptime fn mixed() -> bool {
            let none: ?u8 = null;
            let some: ?u8 = 7;
            let empty_success: !u8 ?u8 = none;
            let present_success: !u8 ?u8 = some;
            let failure: !u8 ?u8 = error 0;
            let values: List<!u8 ?u8> = [failure, present_success, empty_success];
            let sorted = values.sort_by(|left, right| left.lt(right));
            sorted.eq([empty_success, present_success, failure])
                and values[0].eq(failure)
        }
        comptime fn custom() -> bool {
            let left: ?Key = Key {rank: 2, ignored: false};
            let right: ?Key = Key {rank: 1, ignored: true};
            let same: ?Key = Key {rank: 2, ignored: true};
            let failure: !Key Key = error Key {rank: 2, ignored: false};
            let other: !Key Key = error Key {rank: 2, ignored: true};
            left.gt(right) and left.eq(same) and (left.cmp(same) matches .equal)
                and (left match {value? => value.rank == 2, null => false})
                and (right match {value? => value.rank == 1, null => false})
                and (failure.cmp(other) matches .equal)
        }
        comptime fn skipped() -> bool {
            let absent: ?Key = null;
            let present: ?Key = Key {rank: 99, ignored: false};
            let success: !Key Key = Key {rank: 99, ignored: false};
            let failure: !Key Key = error Key {rank: 99, ignored: true};
            absent.lt(present) and present.gt(absent)
                and success.lt(failure) and failure.gt(success)
        }
        comptime fn fail_optional() -> Order {
            let value: ?Key = Key {rank: 99, ignored: false};
            value.cmp(value)
        }
        comptime fn fail_success() -> Order {
            let value: !Key Key = Key {rank: 99, ignored: false};
            value.cmp(value)
        }
        comptime fn fail_error() -> Order {
            let value: !Key Key = error Key {rank: 99, ignored: false};
            value.cmp(value)
        }
    "#,
    )
    .unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for compiled in [true, false] {
        let budget = Budget {
            steps: 5_000_000,
            ..Default::default()
        };
        let mut vm = if compiled {
            Interpreter::for_comptime(&package, budget)
        } else {
            Interpreter::for_comptime_tree(&package, budget)
        }
        .unwrap();
        for name in ["fail_optional", "fail_success", "fail_error"] {
            assert_eq!(
                vm.invoke(id(name), vec![]).unwrap_err().kind,
                ErrorKind::ProgramContractRejected,
                "{name}"
            );
        }
        for name in ["optional", "errors", "mixed", "custom", "skipped"] {
            let before = vm.events().len();
            assert_eq!(
                vm.invoke(id(name), vec![]).unwrap(),
                Value::Bool(true),
                "{name}"
            );
            if name == "skipped" {
                assert!(
                    !vm.events()[before..]
                        .iter()
                        .any(|event| { matches!(event, Event::CheckPredicate { .. }) })
                );
            }
        }
        assert_eq!(vm.bytecode_invocations() > 0, compiled);
    }
}

#[test]
fn missing_payload_order_is_rejected_even_when_that_branch_is_absent() {
    for body in [
        "let value: ?Unordered = null; value.as(ComptimeOrd).cmp(value)",
        "let value: !Unordered u8 = 1; value.as(ComptimeOrd).cmp(value)",
        "let value: !u8 Unordered = error 1; value.as(ComptimeOrd).cmp(value)",
    ] {
        let source = format!(
            r#"
            struct Unordered {{}}
            comptime impl ComptimeEq for Unordered {{
                comptime fn eq(self, other: Self) -> bool {{ true }}
            }}
            comptime fn invalid() -> Order {{ {body} }}
        "#
        );
        let error = compile(&source).unwrap_err();
        assert!(!error.contains("parsing failed"), "{error}");
        assert!(error.contains("no-applicable-candidate"), "{error}");
    }
}
