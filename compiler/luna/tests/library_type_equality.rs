//! Real meta protocols delegate Type equality to proofs, including in containers.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

const SOURCE: &str = "
    pure comptime fn Identity<T> -> Type { T }
    struct First {}
    struct Second {}
    newtype Wrapped = bool;
    typealias Flag = bool;
    effect read() -> bool;
    effect write() -> bool;
    typealias ReadWrite = #[read, write] bool;
    typealias WriteRead = #[write, read] bool;

    comptime fn compare(left: Type, right: Type) -> bool { left.eq(right) }
    comptime fn different(left: Type, right: Type) -> bool { left.ne(right) }

    comptime fn identities() -> bool {
        compare(Flag, bool) and different(bool, u8)
            and compare(First, First) and different(First, Second)
            and different(Wrapped, bool)
            and compare(Identity<bool>, Identity<bool>)
            and compare(ReadWrite, WriteRead)
    }
    comptime fn containers() -> bool {
        let left: List<Type> = [bool, Identity<u8>, ReadWrite];
        let right: List<Type> = [Flag, Identity<u8>, WriteRead];
        let empty: List<Type> = [];
        let nested_left: List<List<Type>> = [empty, left];
        let nested_right: List<List<Type>> = [empty, right];
        let present: ?Type = Identity<bool>;
        let absent: ?Type = null;
        left.eq(right) and not left.ne(right) and empty.eq(empty)
            and nested_left.eq(nested_right)
            and present.eq(present) and absent.ne(present)
    }
    comptime fn short_circuit() -> bool {
        let left: List<Type> = [bool, Identity<bool>];
        let right: List<Type> = [u8, bool];
        let shorter: List<Type> = [Identity<bool>];
        left.ne(right) and shorter.ne(left)
    }
    comptime fn unknown_eq() -> bool { compare(Identity<bool>, bool) }
    comptime fn unknown_ne() -> bool { different(Identity<bool>, bool) }
    comptime fn unknown_list() -> bool {
        let left: List<Type> = [Identity<bool>];
        let right: List<Type> = [bool];
        left.eq(right)
    }
";

#[test]
fn public_type_equality_preserves_identity_and_composes_with_containers() {
    let package = sysroot::compile(SOURCE).unwrap();
    for entry in ["identities", "containers", "short_circuit"] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 2_000_000).unwrap(),
            Value::Bool(true),
            "{entry}"
        );
    }
}

#[test]
fn opaque_equality_unknown_is_not_false_or_a_cached_answer() {
    let package = std::sync::Arc::new(sysroot::compile(SOURCE).unwrap());
    for entry in ["unknown_eq", "unknown_ne", "unknown_list"] {
        let mut call =
            luna::invocation::PreparedInvocation::new(package.clone(), entry, None).unwrap();
        assert!(call.invoke(1).is_err());
        for _ in 0..2 {
            let error = call.invoke(2_000_000).unwrap_err();
            assert!(error.contains("TypeContractUnresolved"), "{entry}: {error}");
            assert!(call.last_stats().bytecode_invocations > 0);
        }
    }
    let mut recovered =
        luna::invocation::PreparedInvocation::new(package, "identities", None).unwrap();
    assert!(recovered.invoke(1).is_err());
    for _ in 0..2 {
        assert_eq!(recovered.invoke(2_000_000).unwrap(), Value::Bool(true));
    }
}
