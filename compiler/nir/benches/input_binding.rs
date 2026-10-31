//! Isolated telescope binding: incremental input moves versus repeated prefix
//! reconstruction using the same public API. No parser, VM or result cache.
use nir::{FunctionInputBinding, FunctionInputBuilder, Parameter, Primitive, Sort, Term};
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

fn bind(signature: &Term, inputs: &[Term], incremental: bool) -> FunctionInputBinding {
    if incremental {
        let mut builder = FunctionInputBuilder::new(signature).unwrap();
        for input in inputs {
            assert_eq!(
                builder.next_type().unwrap(),
                &Term::Primitive(Primitive::Type)
            );
            builder.push(Some(input.clone())).unwrap();
        }
        builder.finish().unwrap()
    } else {
        let mut prefix = vec![None; inputs.len()];
        let mut result = None;
        for (index, input) in inputs.iter().enumerate() {
            prefix[index] = Some(input.clone());
            result = Some(signature.prepare_function_inputs(&prefix).unwrap());
        }
        result.unwrap()
    }
}
fn main() {
    let iterations: u64 = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(20);
    assert!(iterations > 0);
    println!("benchmark=input-binding iterations={iterations} warmup=3 payload-width=64");
    for count in [1, 16, 64] {
        let signature = Term::Function {
            parameters: vec![
                Parameter {
                    sort: Sort::Type,
                    ty: Term::Primitive(Primitive::Type)
                };
                count
            ],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let inputs = vec![Term::Tuple(vec![Term::Primitive(Primitive::Bool); 64]); count];
        assert_eq!(
            bind(&signature, &inputs, true),
            bind(&signature, &inputs, false)
        );
        for incremental in [false, true] {
            for _ in 0..3 {
                std::hint::black_box(bind(&signature, &inputs, incremental));
            }
            let allocations = ALLOCATIONS.load(Ordering::Relaxed);
            let bytes = BYTES.load(Ordering::Relaxed);
            let start = Instant::now();
            for _ in 0..iterations {
                std::hint::black_box(bind(&signature, &inputs, incremental));
            }
            let elapsed = start.elapsed();
            let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations;
            let bytes = BYTES.load(Ordering::Relaxed) - bytes;
            println!(
                "count={count} incremental={incremental} ns/bind={} allocations/bind={:.2} allocated-bytes/bind={:.2}",
                elapsed.as_nanos() / iterations as u128,
                allocations as f64 / iterations as f64,
                bytes as f64 / iterations as f64
            );
        }
    }
}
