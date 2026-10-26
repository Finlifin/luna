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

fn base_package(element: Primitive, reads: usize, mode: &str) -> Package {
    use nir::normal_forms::{Head, Kind, Parameter as Input, Role};
    let source = SourceRange { start: 1, end: 2 };
    let list = Term::NormalForm {
        head: id(2),
        sort: Sort::Type,
        arguments: vec![primitive(element)],
    };
    let list_binding = LocalId { owner: 1, index: 0 };
    let parameters = vec![list_binding];
    let types = vec![Parameter {
        sort: Sort::ComptimeValue,
        ty: list.clone(),
    }];
    let local = Expr {
        source,
        ty: list.clone(),
        kind: K::Local(list_binding),
    };
    let values = match mode {
        "local" => local,
        "general" => Expr {
            source,
            ty: list,
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(local)),
            },
        },
        _ => unreachable!(),
    };
    let read = Expr {
        source,
        ty: primitive(Primitive::Integer),
        kind: K::ListLength(Box::new(values)),
    };
    Package {
        normal_forms: vec![Head {
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
            name: "length".into(),
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
                result: Box::new(primitive(Primitive::Integer)),
            },
            body: Expr {
                source,
                ty: primitive(Primitive::Integer),
                kind: K::Block {
                    statements: vec![read.clone(); reads - 1],
                    result: Some(Box::new(read)),
                },
            },
        }],
        ..Default::default()
    }
}

fn package(element: Primitive, count: usize, general: bool) -> (Package, Term, Term) {
    let mut package = base_package(element, 1, "local");
    let constructor = &mut package.functions[0];
    let Term::Function { parameters, .. } = &constructor.signature else {
        unreachable!()
    };
    let list = parameters[0].ty.clone();
    let pointer = Term::Pointer(Box::new(list.clone()));
    let source = constructor.body.source;
    let binding = constructor.parameters[0];
    let node = |ty, kind| Expr { source, ty, kind };
    constructor.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: list.clone(),
        }],
        result: Box::new(pointer.clone()),
    };
    constructor.body = node(
        pointer.clone(),
        K::AddressOf(Box::new(node(list.clone(), K::Local(binding)))),
    );
    let mut writer = constructor.clone();
    writer.declaration = id(4);
    writer.name = "append".into();
    writer.pure = false;
    writer.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: pointer.clone(),
        }],
        result: Box::new(primitive(Primitive::Void)),
    };
    let item = if element == Primitive::Type {
        K::TypeValue(primitive(Primitive::Bool))
    } else {
        K::Literal(nir::program::Literal::Bool(true))
    };
    let suffix = node(
        list.clone(),
        K::TypeList(vec![node(primitive(element), item)]),
    );
    let suffix = if general {
        node(
            list.clone(),
            K::Block {
                statements: vec![],
                result: Some(Box::new(suffix)),
            },
        )
    } else {
        suffix
    };
    let local = node(pointer.clone(), K::Local(binding));
    let append = node(
        primitive(Primitive::Void),
        K::WritePointer {
            pointer: Box::new(local.clone()),
            value: Box::new(node(
                list.clone(),
                K::ListConcat(
                    Box::new(node(list.clone(), K::ReadPointer(Box::new(local)))),
                    Box::new(suffix),
                ),
            )),
        },
    );
    writer.body = node(
        primitive(Primitive::Void),
        K::Block {
            statements: vec![append; count],
            result: None,
        },
    );
    package.functions.push(writer);
    (package, list, pointer)
}

fn main() {
    let iterations: u64 = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(20);
    assert!(iterations > 0);
    println!("benchmark=list-append iterations={iterations} warmup=5");
    for element in [Primitive::Bool, Primitive::Type] {
        for count in [16, 256] {
            for general in [false, true] {
                let (package, list, pointer_ty) = package(element, count, general);
                let mut vm = Interpreter::for_comptime(
                    &package,
                    Budget {
                        steps: u64::MAX,
                        depth: 128,
                    },
                )
                .unwrap();
                let invoke = |vm: &mut Interpreter<'_>| {
                    let original = if element == Primitive::Type {
                        Value::TypeList(vec![])
                    } else {
                        Value::List {
                            ty: list.clone(),
                            values: vec![],
                        }
                    };
                    let owner = comptime_interpretor::Storage::new(original);
                    let pointer = vm
                        .invoke_with_typed_values(
                            id(1),
                            vec![(list.clone(), Value::Storage(owner.clone()))],
                        )
                        .unwrap();
                    assert_eq!(
                        vm.invoke_with_typed_values(id(4), vec![(pointer_ty.clone(), pointer)])
                            .unwrap(),
                        Value::Unit
                    );
                    match owner.read().unwrap() {
                        Value::TypeList(values) => assert_eq!(values.len(), count),
                        Value::List { values, .. } => assert_eq!(values.len(), count),
                        other => panic!("invalid result {other:?}"),
                    }
                };
                for _ in 0..5 {
                    invoke(&mut vm);
                }
                let steps = vm.remaining_steps();
                let allocations = ALLOCATIONS.load(Ordering::Relaxed);
                let bytes = BYTES.load(Ordering::Relaxed);
                let start = Instant::now();
                for _ in 0..iterations {
                    invoke(&mut vm);
                }
                let elapsed = start.elapsed();
                let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
                let bytes = BYTES.load(Ordering::Relaxed) - bytes;
                println!(
                    "element={element:?} general={general} count={count} ns/invoke={} allocations/invoke={:.2} allocated-bytes/invoke={:.2} steps/invoke={}",
                    elapsed.as_nanos() / iterations as u128,
                    allocations as f64 / iterations as f64,
                    bytes as f64 / iterations as f64,
                    (steps - vm.remaining_steps()) / iterations
                );
            }
        }
    }
}
