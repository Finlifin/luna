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
static BYTES: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn primitive(ty: Primitive) -> Term {
    Term::Primitive(ty)
}

fn package(length: usize, reads: usize, mode: &str) -> Package {
    use nir::normal_forms::{Head, Kind, Parameter as Input, Role};
    let source = SourceRange { start: 1, end: 2 };
    let list = Term::NormalForm {
        head: id(2),
        sort: Sort::Type,
        arguments: vec![primitive(Primitive::Type)],
    };
    let list_binding = LocalId { owner: 1, index: 0 };
    let index_binding = LocalId { owner: 1, index: 1 };
    let mut parameters = vec![list_binding];
    let mut types = vec![Parameter {
        sort: Sort::ComptimeValue,
        ty: list.clone(),
    }];
    let literal = Expr {
        source,
        ty: primitive(Primitive::Integer),
        kind: K::Literal(nir::program::Literal::Integer((length - 1).to_string())),
    };
    let index = match mode {
        "literal" => literal,
        "local" => {
            parameters.push(index_binding);
            types.push(Parameter {
                sort: Sort::ComptimeValue,
                ty: primitive(Primitive::Integer),
            });
            Expr {
                source,
                ty: primitive(Primitive::Integer),
                kind: K::Local(index_binding),
            }
        }
        // A block cannot use the leaf fast path. It returns the same index,
        // allowing paired measurements against general snapshot evaluation.
        "general" => Expr {
            source,
            ty: primitive(Primitive::Integer),
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(literal)),
            },
        },
        _ => unreachable!(),
    };
    let read = Expr {
        source,
        ty: primitive(Primitive::Type),
        kind: K::TypeListIndex(
            Box::new(Expr {
                source,
                ty: list,
                kind: K::Local(list_binding),
            }),
            Box::new(index),
        ),
    };
    Package {
        normal_forms: vec![Head {
            collector: None,
            role: Role::TypeList,
            declaration: id(2),
            requirements: vec![],
            parameters: vec![Input {
                declaration: id(3),
                binding: LocalId { owner: 2, index: 0 },
                kind: Kind::Type,
                ty: primitive(Primitive::Type),
            }],
        }],
        functions: vec![Function {
            declaration: id(1),
            name: "ReadType".into(),
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
            parameters,
            defaults: vec![],
            signature: Term::Function {
                parameters: types,
                result: Box::new(primitive(Primitive::Type)),
            },
            body: Expr {
                source,
                ty: primitive(Primitive::Type),
                kind: K::Block {
                    statements: vec![read.clone(); reads - 1],
                    result: Some(Box::new(read)),
                },
            },
        }],
        ..Default::default()
    }
}

fn main() {
    let iterations: u64 = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(100);
    assert!(iterations > 0);
    println!("benchmark=type-list-index iterations={iterations} warmup=5");
    for length in [16, 512, 4096] {
        for reads in [1, 64] {
            for mode in ["literal", "local", "general"] {
                let package = package(length, reads, mode);
                let mut vm = Interpreter::for_comptime(
                    &package,
                    Budget {
                        steps: u64::MAX,
                        depth: 128,
                    },
                )
                .unwrap();
                let arguments = || {
                    let mut args = vec![Value::List {
                        ty: Term::NormalForm {
                            head: id(2),
                            sort: Sort::Type,
                            arguments: vec![primitive(Primitive::Type)],
                        },
                        values: vec![Value::Type(primitive(Primitive::Bool)); length].into(),
                    }];
                    if mode == "local" {
                        args.push(Value::Integer(comptime_runtime::Integer::from_u128(
                            (length - 1) as u128,
                        )));
                    }
                    args
                };
                for _ in 0..5 {
                    assert_eq!(
                        vm.invoke(id(1), arguments()).unwrap(),
                        Value::Type(primitive(Primitive::Bool))
                    );
                }
                let steps = vm.remaining_steps();
                let allocations = ALLOCATIONS.load(Ordering::Relaxed);
                let bytes = BYTES.load(Ordering::Relaxed);
                let start = Instant::now();
                for _ in 0..iterations {
                    assert_eq!(
                        std::hint::black_box(vm.invoke(id(1), arguments()).unwrap()),
                        Value::Type(primitive(Primitive::Bool))
                    );
                }
                let elapsed = start.elapsed();
                let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
                let bytes = BYTES.load(Ordering::Relaxed) - bytes;
                println!(
                    "mode={mode} length={length} reads={reads} ns/invoke={} allocations/invoke={:.2} allocated-bytes/invoke={:.2} steps/invoke={}",
                    elapsed.as_nanos() / iterations as u128,
                    allocations as f64 / iterations as f64,
                    bytes as f64 / iterations as f64,
                    (steps - vm.remaining_steps()) / iterations
                );
            }
        }
    }
}
