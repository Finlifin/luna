//! Releasing leaf descriptions must not allocate temporary traversal storage.
use nir::{Primitive, Term};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    // Per-thread observations exclude allocations by the test runner.
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CountingAllocator;
fn record() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn disposing_many_leaf_types_does_not_allocate() {
    let terms = vec![Term::Primitive(Primitive::Bool); 4096];
    ALLOCATIONS.with(|count| count.set(Some(0)));
    for term in terms {
        term.drop_iteratively();
    }
    let count = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(count, 0);
}

#[test]
fn scalar_payloads_and_empty_containers_need_no_disposal_stack() {
    let terms = [
        Term::Unit,
        Term::Bool(true),
        Term::String("owned payload".into()),
        Term::Symbol("owned symbol".into()),
        Term::Tuple(vec![]),
        Term::Record(vec![]),
        Term::TypeListValue(vec![]),
    ];
    ALLOCATIONS.with(|count| count.set(Some(0)));
    for term in terms {
        term.drop_iteratively();
    }
    let count = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(count, 0);
}
