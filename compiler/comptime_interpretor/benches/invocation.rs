//! Invocation-only benchmark: no parser, elaborator, LLVM, or package setup in
//! the timed region. Run several times in release mode; compare medians.
use comptime_interpretor::{Budget, Interpreter, Value};
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

fn main() {
    let iterations: u64 = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(20_000);
    for reads in [0, 256] {
        for count in [0, 1, 8, 64] {
            let boolean = Term::Primitive(Primitive::Bool);
            let expression = |kind| Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: boolean.clone(),
                kind,
            };
            let parameters: Vec<_> = (0..count)
                .map(|index| LocalId { owner: 1, index })
                .collect();
            let body = if count == 0 {
                expression(K::Literal(nir::program::Literal::Bool(true)))
            } else {
                expression(K::Block {
                    statements: parameters
                        .iter()
                        .cycle()
                        .take(reads)
                        .map(|id| expression(K::Local(*id)))
                        .collect(),
                    result: Some(Box::new(expression(K::Local(parameters[0])))),
                })
            };
            let id = DeclarationId {
                package: 1,
                local: 1,
            };
            let package = Package {
                functions: vec![Function {
                    declaration: id,
                    name: "bench".into(),
                    phase: FunctionPhase::Comptime,
                    inline: false,
                    pure: true,
                    is_unsafe: false,
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    entry_snapshots: vec![],
                    type_parameters: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                    signature: Term::Function {
                        parameters: parameters
                            .iter()
                            .map(|_| Parameter {
                                sort: Sort::Runtime,
                                ty: boolean.clone(),
                            })
                            .collect(),
                        result: Box::new(boolean),
                    },
                    parameters,
                    defaults: vec![],
                    body,
                }],
                ..Default::default()
            };
            let mut vm = Interpreter::for_comptime(
                &package,
                Budget {
                    steps: u64::MAX,
                    depth: 128,
                },
            )
            .unwrap();
            for _ in 0..100 {
                assert_eq!(
                    vm.invoke(id, vec![Value::Bool(true); count as usize])
                        .unwrap(),
                    Value::Bool(true)
                );
            }
            let fuel = vm.remaining_steps();
            let allocations = ALLOCATIONS.load(Ordering::Relaxed);
            let start = Instant::now();
            for _ in 0..iterations {
                assert_eq!(
                    std::hint::black_box(
                        vm.invoke(id, vec![Value::Bool(true); count as usize])
                            .unwrap()
                    ),
                    Value::Bool(true)
                );
            }
            let elapsed = start.elapsed();
            let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
            println!(
                "reads={reads} locals={count} ns/invoke={} allocations/invoke={:.2} steps/invoke={}",
                elapsed.as_nanos() / iterations as u128,
                allocations as f64 / iterations as f64,
                (fuel - vm.remaining_steps()) / iterations
            );
        }
    }
}
