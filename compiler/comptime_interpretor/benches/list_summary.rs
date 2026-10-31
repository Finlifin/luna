//! Isolate closed representation scanning from invocation and type admission.
//! Cold queries invalidate metadata without changing contents; warm queries
//! reuse it. Both must report exactly the same logical work.
use comptime_interpretor::Value;
use nir::{DeclarationId, Primitive, Sort, Term};
use std::{hint::black_box, time::Instant};

fn main() {
    let iterations: usize = std::env::var("LUNA_BENCH_ITERATIONS")
        .ok()
        .map(|value| value.parse().expect("positive iteration count"))
        .unwrap_or(100);
    assert!(iterations > 0);
    println!("benchmark=list-summary iterations={iterations} rounds=5 mode=representation-only");
    for element in [Primitive::Bool, Primitive::Type] {
        for count in [16, 256, 4096] {
            let item = if element == Primitive::Type {
                Value::Type(Term::Primitive(Primitive::Bool))
            } else {
                Value::Bool(true)
            };
            // This benchmark checks representation work, not head registration
            // or nominal admission; it deliberately constructs no interpreter.
            let mut value = Value::List {
                ty: Term::NormalForm {
                    head: DeclarationId {
                        package: 1,
                        local: 1,
                    },
                    sort: Sort::Type,
                    arguments: vec![Term::Primitive(element)],
                },
                values: vec![item; count].into(),
            };
            let expected = value.immutable_copy_work().unwrap();
            let mut samples = [Vec::new(), Vec::new()];
            for round in 0..5 {
                // Alternate order to reduce systematic warm-first bias.
                for cold in if round % 2 == 0 {
                    [true, false]
                } else {
                    [false, true]
                } {
                    let start = Instant::now();
                    for _ in 0..iterations {
                        if cold {
                            let Value::List { values, .. } = &mut value else {
                                unreachable!()
                            };
                            black_box(values.as_mut_slice());
                        }
                        assert_eq!(black_box(&value).immutable_copy_work(), Some(expected));
                    }
                    samples[usize::from(cold)]
                        .push(start.elapsed().as_nanos() / iterations as u128);
                }
            }
            for (cold, samples) in samples.iter_mut().enumerate() {
                samples.sort_unstable();
                println!(
                    "element={element:?} count={count} cold={} work/query={expected} median-ns/query={} samples-ns={samples:?}",
                    cold != 0,
                    samples[2]
                );
            }
        }
    }
}
