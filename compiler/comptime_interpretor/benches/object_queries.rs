//! Invocation-only benchmark: no parser, elaborator, LLVM, or package setup in
//! the timed region. Run several times in release mode; compare medians.
use comptime_interpretor::{Budget, Interpreter, ObjectValue, Value};
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, LocalId, ObjectQueryKind, Package, SourceRange,
};
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

fn package(reads: usize, mode: &str, query: ObjectQueryKind) -> Package {
    let source = SourceRange { start: 1, end: 2 };
    let binding = LocalId { owner: 1, index: 0 };
    let object_type = primitive(Primitive::Object);
    let result = if query == ObjectQueryKind::Length {
        primitive(Primitive::Integer)
    } else {
        Term::Optional(Box::new(primitive(Primitive::Bool)))
    };
    let local = Expr {
        source,
        ty: object_type.clone(),
        kind: K::Local(binding),
    };
    // A transparent block forces ordinary packet evaluation. It adds block-evaluation ticks
    // per read, so this is a path comparison, not a historical-version baseline.
    let subject = if mode == "borrowed" {
        local
    } else {
        Expr {
            source,
            ty: object_type.clone(),
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(local)),
            },
        }
    };
    let mut fields = vec![subject];
    if query != ObjectQueryKind::Length {
        fields.push(Expr {
            source,
            ty: primitive(Primitive::Integer),
            kind: K::Literal(nir::program::Literal::Integer("0".into())),
        });
    }
    let read = Expr {
        source,
        ty: result.clone(),
        kind: K::ObjectQuery {
            query,
            subject: Box::new(Expr {
                source,
                ty: Term::Tuple(fields.iter().map(|field| field.ty.clone()).collect()),
                kind: K::Tuple(fields),
            }),
        },
    };
    Package {
        functions: vec![Function {
            declaration: id(1),
            name: "observe".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            parameters: vec![binding],
            defaults: vec![],
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: object_type,
                }],
                result: Box::new(result.clone()),
            },
            body: Expr {
                source,
                ty: result,
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
    use comptime_runtime::object::{Entry, Object};
    let iterations: u64 = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|value| value.parse().unwrap())
        .unwrap_or(100);
    assert!(iterations > 0);
    for width in [1, 64, 1024] {
        for query in [ObjectQueryKind::Length, ObjectQueryKind::Value] {
            for mode in ["borrowed", "general"] {
                let reads = 32;
                let package = package(reads, mode, query);
                let mut vm = Interpreter::for_comptime(
                    &package,
                    Budget {
                        steps: u64::MAX,
                        depth: 128,
                    },
                )
                .unwrap();
                let payload = Value::Object(Object::from_entries(
                    (0..width)
                        .map(|index| Entry::Property {
                            key: format!("field_{index}").into(),
                            value: ObjectValue {
                                ty: primitive(Primitive::Bool),
                                value: Value::Bool(true),
                            },
                        })
                        .collect(),
                ));
                let expected = if query == ObjectQueryKind::Length {
                    Value::Integer(comptime_runtime::Integer::from_u128(width as u128))
                } else {
                    Value::Optional {
                        ty: Term::Optional(Box::new(primitive(Primitive::Bool))),
                        value: Some(Box::new(Value::Bool(true))),
                    }
                };
                for _ in 0..5 {
                    assert_eq!(vm.invoke(id(1), vec![payload.clone()]).unwrap(), expected);
                }
                let steps = vm.remaining_steps();
                let allocations = ALLOCATIONS.load(Ordering::Relaxed);
                let bytes = BYTES.load(Ordering::Relaxed);
                let start = Instant::now();
                for _ in 0..iterations {
                    assert_eq!(
                        std::hint::black_box(vm.invoke(id(1), vec![payload.clone()]).unwrap()),
                        expected
                    );
                }
                let elapsed = start.elapsed();
                let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
                let bytes = BYTES.load(Ordering::Relaxed) - bytes;
                println!(
                    "query={query:?} mode={mode} width={width} reads={reads} ns/invoke={} allocations/invoke={:.2} allocated-bytes/invoke={:.2} steps/invoke={}",
                    elapsed.as_nanos() / iterations as u128,
                    allocations as f64 / iterations as f64,
                    bytes as f64 / iterations as f64,
                    (steps - vm.remaining_steps()) / iterations
                );
            }
        }
    }
}
