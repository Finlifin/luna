//! List hashing composes element protocols without inspecting representations.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};

fn compile(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} use meta.{{List, ComptimeEq, ComptimeHash}}; use meta.builtin.{{Integer, u64}}; {body}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

fn list_digest(values: &[u64]) -> u64 {
    std::iter::once(76u8)
        .chain(values.iter().flat_map(|value| value.to_le_bytes()))
        .fold(14695981039346656037, |state, byte| {
            (state ^ u64::from(byte)).wrapping_mul(1099511628211)
        })
}

#[test]
fn list_hashes_encode_complete_element_digests_in_order_without_target_data() {
    let package = compile(
        "comptime fn empty()->u64 {let values:List<u64> = []; values.hash()}
         comptime fn forward()->u64 {let values:List<u64> = [1,256,18446744073709551615]; values.hash()}
         comptime fn reverse()->u64 {let values:List<u64> = [18446744073709551615,256,1]; values.hash()}
         comptime fn nested()->u64 {let values:List<List<u64>> = [[],[1,256,18446744073709551615]]; values.hash()}
         fn required()->bool {inline if nested()==nested() {true} else {false}}",
    ).unwrap();
    assert!(package.target.is_none());
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let forward = list_digest(&[1, 256, u64::MAX]);
    for (name, expected) in [
        ("empty", list_digest(&[])),
        ("forward", forward),
        ("reverse", list_digest(&[u64::MAX, 256, 1])),
        ("nested", list_digest(&[list_digest(&[]), forward])),
    ] {
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::UnsignedInteger(expected.into()),
            "{name}"
        );
    }
}

#[test]
fn custom_hashes_preserve_snapshots_and_propagate_failures() {
    let package = compile(
        "struct Key {value:Integer}
        comptime impl ComptimeEq for Key {
            comptime fn eq(self,other:Self)->bool {self.value.rem_euclid(10)==other.value.rem_euclid(10)}
        }
        comptime impl ComptimeHash for Key {
            comptime fn hash(self)->u64 {
                let previous=self.value;
                self.value=99;
                let observed=1.div_trunc(if previous==99 {0} else {1});
                if previous.rem_euclid(10)==2 and observed==1 {2} else {0}
            }
        }
        comptime fn run()->bool {
            let a:List<Key> = [Key {value:12},Key {value:22}];
            let b:List<Key> = [Key {value:22},Key {value:12}];
            let words:List<u64> = [2,2];
            a.eq(b) and a.hash()==b.hash() and a.hash()==words.hash()
                and a[0].value==12 and a[1].value==22
        }
        comptime fn fail()->u64 {let values:List<Key> = [Key {value:99}];values.hash()}",
    ).unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        vm.invoke(support::id(&package, "fail"), vec![])
            .unwrap_err()
            .kind,
        ErrorKind::DivisionByZero
    );
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn equality_alone_does_not_supply_list_hashing() {
    let failure=compile(
        "struct Key {value:bool}
        comptime impl ComptimeEq for Key {comptime fn eq(self,other:Self)->bool {self.value==other.value}}
        comptime fn bad(values:List<Key>)->u64 {values.as(ComptimeHash).hash()}",
    ).err().expect("missing element hash must reject the call");
    assert!(
        failure.contains("trait") || failure.contains("method"),
        "{failure}"
    );
}

#[test]
fn list_key_protocol_checks_equality_after_element_hash_collisions() {
    use comptime_executor::{Context, KeyProtocol};
    let package = compile(
        "use meta.builtin.u128;
         comptime fn low()->List<u128> {[1]}
         comptime fn high()->List<u128> {[18446744073709551616]}",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let low = vm.invoke(support::id(&package, "low"), vec![]).unwrap();
    let high = vm.invoke(support::id(&package, "high"), vec![]).unwrap();
    let Value::List { ty, .. } = &low else {
        panic!("expected list value")
    };
    let mut protocol = prepare_protocol(&package, ty);
    assert!(protocol.hash(&low, &mut Context::new(1)).is_err());
    let mut context = Context::new(5_000_000);
    assert_eq!(
        protocol.hash(&low, &mut context).unwrap(),
        protocol.hash(&high, &mut context).unwrap()
    );
    assert!(!protocol.equivalent(&low, &high, &mut context).unwrap());
    assert!(protocol.equivalent(&high, &high, &mut context).unwrap());
}

fn prepare_protocol(
    package: &nir::program::Package,
    ty: &nir::Term,
) -> luna::comptime_keys::Protocol {
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    luna::comptime_keys::Protocol::prepare(package, &items, ty, Default::default()).unwrap()
}

#[test]
fn key_methods_can_call_user_helpers_with_ordinary_type_inputs() {
    use comptime_executor::{Context, KeyProtocol};
    let package = compile(
        "use meta.builtin.Type;
        comptime fn passthrough(comptime T:Type,value:T)->T {value}
        struct Key {value:bool}
        comptime impl ComptimeEq for Key {
            comptime fn eq(self,other:Self)->bool {
                passthrough(bool,self.value)==passthrough(bool,other.value)
            }
        }
        comptime impl ComptimeHash for Key {
            comptime fn hash(self)->u64 {if passthrough(bool,self.value) {1} else {0}}
        }
        comptime fn make(value:bool)->Key {Key {value:value}}",
    )
    .unwrap();
    let make = support::id(&package, "make");
    let signature = &package
        .functions
        .iter()
        .find(|f| f.declaration == make)
        .unwrap()
        .signature;
    let nir::Term::Function { result, .. } = signature.callable_signature() else {
        panic!("function signature")
    };
    let mut protocol = prepare_protocol(&package, result);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let yes = vm.invoke(make, vec![Value::Bool(true)]).unwrap();
    let no = vm.invoke(make, vec![Value::Bool(false)]).unwrap();
    let mut context = Context::new(1_000_000);
    let first = protocol.hash(&yes, &mut context).unwrap();
    assert_eq!(first, protocol.hash(&yes, &mut context).unwrap());
    assert!(protocol.equivalent(&yes, &yes, &mut context).unwrap());
    assert!(!protocol.equivalent(&yes, &no, &mut context).unwrap());
}

#[test]
fn key_helpers_resolve_traits_after_binding_ordinary_type_inputs() {
    use comptime_executor::{Context, KeyProtocol};
    let package = compile(
        "use meta.builtin.Type;
        comptime fn equal(comptime T:Type,left:T,right:T)->bool
        where requires T:-ComptimeEq
        {left.as(ComptimeEq).eq(right)}
        comptime fn digest(comptime T:Type,value:T)->u64
        where requires T:-ComptimeHash
        {value.as(ComptimeHash).hash()}
        struct Key {value:bool}
        comptime impl ComptimeEq for Key {
            comptime fn eq(self,other:Self)->bool {
                equal(bool,self.value,other.value)
            }
        }
        comptime impl ComptimeHash for Key {
            comptime fn hash(self)->u64 {
                let flag = digest(bool,self.value);
                digest(u64,flag)
            }
        }
        comptime fn make(value:bool)->Key {Key {value:value}}",
    )
    .unwrap();
    let make = support::id(&package, "make");
    let signature = &package
        .functions
        .iter()
        .find(|f| f.declaration == make)
        .unwrap()
        .signature;
    let nir::Term::Function { result, .. } = signature.callable_signature() else {
        panic!("function signature")
    };
    let mut protocol = prepare_protocol(&package, result);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let yes = vm.invoke(make, vec![Value::Bool(true)]).unwrap();
    let no = vm.invoke(make, vec![Value::Bool(false)]).unwrap();
    assert!(protocol.hash(&yes, &mut Context::new(1)).is_err());
    let mut context = Context::new(1_000_000);
    let first = protocol.hash(&yes, &mut context).unwrap();
    assert_eq!(first, protocol.hash(&yes, &mut context).unwrap());
    assert_ne!(first, protocol.hash(&no, &mut context).unwrap());
    assert!(protocol.equivalent(&yes, &yes, &mut context).unwrap());
    assert!(!protocol.equivalent(&yes, &no, &mut context).unwrap());
}

#[test]
fn key_helpers_resolve_associated_values_after_binding_type_inputs() {
    use comptime_executor::{Context, KeyProtocol};
    let package = compile(
        "use meta.builtin.Type;
        pure comptime trait Tag {assoc code:u64;}
        comptime impl Tag for bool {assoc code:u64 = 17;}
        comptime fn tag_code(value:u64)->u64 {if value == 28 {29} else {17}}
        comptime impl Tag for u64 {assoc code:u64 = tag_code(28);}
        comptime fn code(comptime T:Type)->u64
        where requires T:-Tag
        {T.as(Tag).code}
        struct Key {value:bool}
        comptime impl ComptimeEq for Key {
            comptime fn eq(self,other:Self)->bool {
                self.value == other.value and code(bool) == 17 and code(u64) == 29
            }
        }
        comptime impl ComptimeHash for Key {
            comptime fn hash(self)->u64 {
                if self.value {code(bool)} else {code(u64)}
            }
        }
        comptime fn make(value:bool)->Key {Key {value:value}}",
    )
    .unwrap();
    let make = support::id(&package, "make");
    let signature = &package
        .functions
        .iter()
        .find(|f| f.declaration == make)
        .unwrap()
        .signature;
    let nir::Term::Function { result, .. } = signature.callable_signature() else {
        panic!("function signature")
    };
    let mut protocol = prepare_protocol(&package, result);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let yes = vm.invoke(make, vec![Value::Bool(true)]).unwrap();
    let no = vm.invoke(make, vec![Value::Bool(false)]).unwrap();
    assert!(protocol.hash(&yes, &mut Context::new(1)).is_err());
    let mut context = Context::new(1_000_000);
    let first = protocol.hash(&yes, &mut context).unwrap();
    assert_eq!(first, protocol.hash(&yes, &mut context).unwrap());
    assert_ne!(first, protocol.hash(&no, &mut context).unwrap());
    assert!(protocol.equivalent(&yes, &yes, &mut context).unwrap());
    assert!(!protocol.equivalent(&yes, &no, &mut context).unwrap());
}
