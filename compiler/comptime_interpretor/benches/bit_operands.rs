//! Fixed-arity BitVec execution in both interpreters. Setup and code warming
//! precede allocation/fuel observations; results still include complete calls.
//! Use the same profile on both sides. Dev timings are diagnostic, not release
//! performance claims. BitVec results are never invocation-result memo entries.
use comptime_interpretor::{Budget, Interpreter, Value};
use comptime_runtime::bitvec::BitVec;
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, LocalId, Package, SourceRange};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

struct CountingAllocator;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
const ID: DeclarationId = DeclarationId {
    package: 1,
    local: 1,
};
const INPUT: LocalId = LocalId { owner: 1, index: 0 };

fn expression(ty: Term, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty,
        kind,
    }
}

fn fixture(arity: usize, repeats: usize) -> Package {
    let bits = Term::Primitive(Primitive::BitVec(64));
    let result = Term::Primitive(Primitive::BitVec(if arity == 2 { 128 } else { 64 }));
    let input = || Box::new(expression(bits.clone(), K::Local(INPUT)));
    let operation = || {
        expression(
            result.clone(),
            match arity {
                1 => K::BitSlice {
                    value: input(),
                    start: 0,
                    end: 64,
                },
                2 => K::BitConcat(input(), input()),
                3 => K::BitWindow {
                    value: input(),
                    start: Box::new(expression(
                        Term::Primitive(Primitive::U64),
                        K::Literal(nir::program::Literal::UnsignedInteger(0)),
                    )),
                    end: Box::new(expression(
                        Term::Primitive(Primitive::U64),
                        K::Literal(nir::program::Literal::UnsignedInteger(64)),
                    )),
                },
                _ => unreachable!(),
            },
        )
    };
    let body = expression(
        result.clone(),
        K::Block {
            statements: (1..repeats).map(|_| operation()).collect(),
            result: Some(Box::new(operation())),
        },
    );
    Package {
        functions: vec![Function {
            declaration: ID,
            name: "bits".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            parameters: vec![INPUT],
            defaults: vec![],
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: bits,
                }],
                result: Box::new(result),
            },
            body,
        }],
        ..Default::default()
    }
}

fn main() {
    let iterations: u64 = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|value| value.parse().unwrap())
        .unwrap_or(1_000);
    assert!(iterations > 0);
    for tree in [false, true] {
        for arity in 1..=3 {
            for repeats in [1, 32] {
                let package = fixture(arity, repeats);
                let budget = Budget {
                    steps: u64::MAX,
                    depth: 128,
                };
                let mut vm = if tree {
                    Interpreter::for_comptime_tree(&package, budget)
                } else {
                    Interpreter::for_comptime(&package, budget)
                }
                .unwrap();
                let input = Value::BitVec(
                    BitVec::from_words(
                        64,
                        &[u32::MAX; 2],
                        &mut comptime_runtime::Budget::default(),
                    )
                    .unwrap(),
                );
                let width = if arity == 2 { 128 } else { 64 };
                let expected = Value::BitVec(
                    BitVec::from_words(
                        width,
                        &vec![u32::MAX; width / 32],
                        &mut comptime_runtime::Budget::default(),
                    )
                    .unwrap(),
                );
                for _ in 0..10 {
                    assert_eq!(vm.invoke(ID, vec![input.clone()]).unwrap(), expected);
                }
                let before_steps = vm.remaining_steps();
                let before_allocations = ALLOCATIONS.load(Ordering::Relaxed);
                let start = Instant::now();
                for _ in 0..iterations {
                    assert_eq!(
                        std::hint::black_box(vm.invoke(ID, vec![input.clone()]).unwrap()),
                        expected
                    );
                }
                let elapsed = start.elapsed();
                let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before_allocations;
                println!(
                    "tree={tree} arity={arity} repeats={repeats} ns/invoke={} allocations/invoke={:.2} steps/invoke={}",
                    elapsed.as_nanos() / u128::from(iterations),
                    allocations as f64 / iterations as f64,
                    (before_steps - vm.remaining_steps()) / iterations
                );
            }
        }
    }
}
