//! Compare declaration preparation, fresh invocation and resident execution in
//! the same binary. Includes VM destruction; excludes fixture construction.
use comptime_interpretor::{Budget, CodeSession, Interpreter, Value};
use nir::program::*;
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

struct CountingAllocator;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn budget() -> Budget {
    Budget {
        steps: 100_000_000,
        depth: 64,
    }
}

fn world(count: u32, reads: usize) -> Arc<Package> {
    let mut package = Package::default();
    let boolean = Term::Primitive(Primitive::Bool);
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: boolean.clone(),
        }],
        result: Box::new(boolean.clone()),
    };
    for index in 0..count {
        let binding = LocalId {
            owner: index + 1,
            index: 0,
        };
        let local = Expr {
            ty: boolean.clone(),
            kind: ExprKind::Local(binding),
            source: SourceRange { start: 1, end: 2 },
        };
        let function = id(index * 4 + 1);
        let interface = id(index * 4 + 2);
        let member = id(index * 4 + 3);
        let implementation = id(index * 4 + 4);
        package.functions.push(Function {
            declaration: function,
            name: format!("identity{index}"),
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
            signature: signature.clone(),
            parameters: vec![binding],
            defaults: vec![],
            body: Expr {
                ty: boolean.clone(),
                source: local.source,
                kind: ExprKind::Block {
                    statements: vec![local.clone(); reads],
                    result: Some(Box::new(local)),
                },
            },
        });
        package.traits.push(Trait {
            comptime_only: true,
            declaration: interface,
            name: format!("Interface{index}"),
            self_parameter: interface,
            type_parameters: vec![],
            parameter_types: vec![],
            parents: vec![],
            requirements: vec![],
            capabilities: vec![],
            associated: vec![],
            methods: vec![MethodContract {
                phase: TraitPhase::Comptime,
                is_quote: false,
                pure: true,
                is_unsafe: false,
                type_arguments: vec![],
                type_parameters: vec![],
                capabilities: vec![],
                program: Default::default(),
                parameter_bindings: vec![binding],
                by_value_receiver: false,
                by_reference_receiver: false,
                declaration: member,
                name: "identity".into(),
                signature: signature.clone().with_comptime_phase(true).into(),
                requirements: vec![],
                default: None,
            }],
        });
        package.implementations.push(Implementation {
            phases: ImplementationPhases {
                runtime: false,
                comptime: true,
            },
            declaration: implementation,
            interface: Some(interface),
            interface_arguments: vec![],
            subject: boolean.clone(),
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            associated: vec![],
            associated_initializers: vec![],
            methods: vec![("identity".into(), function)],
        });
        package.dispatch_instances.push(DispatchInstance {
            interface,
            interface_arguments: vec![],
            subject: boolean.clone(),
            implementation,
            arguments: vec![],
            associated: vec![],
            associated_initializers: vec![],
            methods: vec![MethodInstance {
                member,
                body: function,
                arguments: vec![],
                method_arguments: vec![],
            }],
        });
    }
    Arc::new(package)
}

fn main() {
    let iterations: u64 = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|value| value.parse().expect("integer iteration count"))
        .unwrap_or(100);
    assert!(iterations > 0);
    println!(
        "benchmark=source-session iterations={iterations} rounds=3 result-cache=disabled includes-vm-drop=true"
    );
    for count in [1, 32, 256] {
        for reads in [0, 64] {
            let source = world(count, reads);
            let mut session = CodeSession::new(source.clone());
            let mut warm =
                Interpreter::for_comptime_in_session(&source, &mut session, &mut budget()).unwrap();
            warm.with_code_session(&mut session, |vm| {
                assert_eq!(
                    vm.invoke(id(1), vec![Value::Bool(true)]).unwrap(),
                    Value::Bool(true)
                )
            });
            drop(warm);
            let mut resident = Interpreter::for_comptime(&source, budget()).unwrap();
            assert_eq!(
                resident.invoke(id(1), vec![Value::Bool(true)]).unwrap(),
                Value::Bool(true)
            );
            for round in 0..3 {
                let mut modes = [
                    "cold-prepare",
                    "warm-prepare",
                    "cold-call",
                    "warm-call",
                    "resident-call",
                ];
                if round % 2 == 1 {
                    modes.reverse();
                }
                for mode in modes {
                    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
                    let mut steps = 0;
                    let mut compilations = 0;
                    let start = Instant::now();
                    for index in 0..iterations {
                        let input = index % 2 == 0;
                        if mode == "resident-call" {
                            let before = resident.remaining_steps();
                            let compiled = resident.bytecode_compilations();
                            assert_eq!(
                                black_box(
                                    resident.invoke(id(1), vec![Value::Bool(input)]).unwrap()
                                ),
                                Value::Bool(input)
                            );
                            steps += before - resident.remaining_steps();
                            compilations += resident.bytecode_compilations() - compiled;
                        } else {
                            let mut allowance = budget();
                            let mut vm = if mode.starts_with("warm") {
                                Interpreter::for_comptime_in_session(
                                    &source,
                                    &mut session,
                                    &mut allowance,
                                )
                                .unwrap()
                            } else {
                                Interpreter::for_comptime(&source, allowance).unwrap()
                            };
                            if mode.ends_with("call") {
                                let value = if mode == "warm-call" {
                                    vm.with_code_session(&mut session, |vm| {
                                        vm.invoke(id(1), vec![Value::Bool(input)])
                                    })
                                } else {
                                    vm.invoke(id(1), vec![Value::Bool(input)])
                                }
                                .unwrap();
                                assert_eq!(black_box(value), Value::Bool(input));
                            }
                            compilations += vm.bytecode_compilations();
                            steps += budget().steps - vm.remaining_steps();
                            black_box(vm);
                        }
                    }
                    let elapsed = start.elapsed().as_nanos();
                    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
                    assert_eq!(
                        compilations,
                        if mode == "cold-call" {
                            iterations as usize
                        } else {
                            0
                        }
                    );
                    println!(
                        "world={count} reads={reads} round={round} mode={mode} ns/op={} allocations/op={:.2} steps/op={} compilations={compilations}",
                        elapsed / u128::from(iterations),
                        allocations as f64 / iterations as f64,
                        steps / iterations
                    );
                }
            }
        }
    }
}
