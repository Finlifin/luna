//! Compound bit updates use checked local places, not synthesized pointers.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn bit_compound_receivers_keep_old_bits_rhs_changes_and_snapshots() {
    let source = concat!(
        include_str!("../../../library/builtin/primitives.fl"),
        "
        typealias Bits4=BitVec<4>;
        trait Operation where Rhs {
            assoc Output:Type;
            pure comptime fn add(self,rhs:Rhs)->Output;
        }
        impl Operation<Bits4> for Bits4 {
            assoc Output:Type=Bits4;
            pure comptime fn add(self,rhs:Bits4)->Bits4 {self or rhs}
        }
        struct Cell {bits:u8,other:bool}
        struct Pair {cell:Cell,sibling:bool}
        impl Cell {
            comptime fn edit(self) {
                self.bits'bitvec[4..8] += do {self.bits=5;self.other=true;1};
            }
        }
        comptime fn run()->bool {
            let pair=Pair {cell:Cell {bits:160,other:false},sibling:true};
            let saved=pair;
            pair.cell.edit();
            pair.cell.bits == 181 and pair.cell.other and pair.sibling
                and saved.cell.bits == 160 and not saved.cell.other
        }"
    );
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[elaboration::arithmetic::Operator::Add as usize] = Some("input.Operation");
    let package =
        support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    let dump = package.to_string();
    assert!(dump.contains("(bit-update "), "{dump}");
    for operation in ["(address-of ", "(read-pointer ", "(bit-write-pointer "] {
        assert!(!dump.contains(operation), "{dump}");
    }
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
