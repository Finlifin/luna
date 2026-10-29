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
        .unwrap_or(1000);
    for width in [1, 64, 1024] {
        let boolean = Term::Primitive(Primitive::Bool);
        // Structural tuples here exercise VM representation, not source Tuple identity.
        let ty = Term::Tuple(vec![Term::Tuple(vec![boolean.clone(); 4]); width]);
        let payload = Value::Tuple(vec![Value::Tuple(vec![Value::Bool(true); 4]); width]);
        let id = DeclarationId {
            package: 1,
            local: 1,
        };
        let mut package = Package {
            functions: vec![Function {
                declaration: id,
                name: "observe".into(),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                parameters: vec![LocalId { owner: 1, index: 0 }],
                defaults: vec![],
                signature: Term::Function {
                    parameters: vec![Parameter {
                        sort: Sort::Runtime,
                        ty: ty.clone(),
                    }],
                    result: Box::new(boolean.clone()),
                },
                body: Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: boolean,
                    kind: K::Literal(nir::program::Literal::Bool(true)),
                },
            }],
            ..Default::default()
        };
        let mut caller = package.functions[0].clone();
        caller.declaration.local = 2;
        caller.name = "run".into();
        let source = caller.body.source;
        let node = |ty, kind| Expr { source, ty, kind };
        let call = node(
            Term::Primitive(Primitive::Bool),
            K::Call {
                callee: Box::new(node(package.functions[0].callable_type(), K::Function(id))),
                arguments: vec![nir::program::CallArgument {
                    parameter: 0,
                    value: node(ty, K::Local(caller.parameters[0])),
                    is_default: false,
                    passing: nir::program::ArgumentPassing::ComptimeReference,
                }],
                parameter_bindings: package.functions[0].parameters.clone(),
            },
        );
        caller.body = node(
            Term::Primitive(Primitive::Bool),
            K::Block {
                statements: vec![call; 16],
                result: Some(Box::new(caller.body.clone())),
            },
        );
        let entry = caller.declaration;
        package.functions.push(caller);
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: u64::MAX,
                depth: 128,
            },
        )
        .unwrap();
        for _ in 0..5 {
            assert_eq!(
                vm.invoke(entry, vec![payload.clone()]).unwrap(),
                Value::Bool(true)
            );
        }
        let fuel = vm.remaining_steps();
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        let start = Instant::now();
        for _ in 0..iterations {
            assert_eq!(
                std::hint::black_box(vm.invoke(entry, vec![payload.clone()]).unwrap()),
                Value::Bool(true)
            );
        }
        let elapsed = start.elapsed();
        let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
        println!(
            "width={width} ns/invoke={} allocations/invoke={:.2} steps/invoke={}",
            elapsed.as_nanos() / iterations as u128,
            allocations as f64 / iterations as f64,
            (fuel - vm.remaining_steps()) / iterations
        );
    }
}
