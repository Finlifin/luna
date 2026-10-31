//! Measure checked struct publication, excluding parsing and source elaboration.
//! The rebuild control is a fresh publisher over the growing checked package;
//! it is not a reconstruction of an older compiler revision.
use comptime_interpretor::{ReflectedField, StructBuildRequest};
use elaboration::definitions::CheckedStructPublisher;
use nir::{
    DeclarationId, Primitive, Term,
    program::{Package, SourceRange, Struct},
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const ALLOWANCE: u64 = u64::MAX / 8;

fn id(local: usize) -> DeclarationId {
    DeclarationId {
        package: 73,
        local: u32::try_from(local).expect("declaration index overflow"),
    }
}
fn nominal(local: usize) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn baseline(count: usize) -> Package {
    Package {
        structs: (0..count)
            .map(|index| Struct {
                declaration: id(index),
                name: format!("Source{index}"),
                type_parameters: vec![],
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                fields: (0..4)
                    .map(|field| (format!("field{field}"), Term::Primitive(Primitive::Bool)))
                    .collect(),
            })
            .collect(),
        ..Default::default()
    }
}
fn requests(base: usize, count: usize) -> Vec<StructBuildRequest> {
    (0..count)
        .map(|index| StructBuildRequest {
            name: format!("Generated{index}"),
            fields: vec![
                ReflectedField {
                    name: "value".into(),
                    ty: Term::Primitive(Primitive::Bool),
                },
                ReflectedField {
                    name: "optional".into(),
                    ty: Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
                },
                ReflectedField {
                    name: "payload".into(),
                    ty: nominal(0),
                },
                ReflectedField {
                    name: "previous".into(),
                    ty: Term::Pointer(Box::new(nominal(if index == 0 {
                        0
                    } else {
                        base + index - 1
                    }))),
                },
            ],
            source: SourceRange { start: 0, end: 1 },
        })
        .collect()
}

struct Measurement {
    elapsed: Duration,
    steps: u64,
    definitions: Vec<Struct>,
}

fn reused(world: &Package, requests: &[StructBuildRequest]) -> Measurement {
    let mut remaining = ALLOWANCE;
    let start = Instant::now();
    let publisher = CheckedStructPublisher::new(world);
    for (index, request) in requests.iter().enumerate() {
        let prepared = publisher
            .prepare(id(world.structs.len() + index), request, &mut remaining)
            .unwrap();
        (prepared.commit)().unwrap();
    }
    let definitions = publisher
        .into_definitions()
        .into_iter()
        .flat_map(|batch| batch.to_vec())
        .collect::<Vec<_>>();
    black_box(&definitions);
    Measurement {
        elapsed: start.elapsed(),
        steps: ALLOWANCE - remaining,
        definitions,
    }
}

fn rebuilt(baseline: &Package, requests: &[StructBuildRequest]) -> Measurement {
    // Initial fixture cloning is outside the measured region. Each subsequent
    // publisher still performs its own checked-world initialization inside it.
    let mut world = baseline.clone();
    let mut remaining = ALLOWANCE;
    let start = Instant::now();
    for request in requests {
        let publisher = CheckedStructPublisher::new(&world);
        {
            let prepared = publisher
                .prepare(id(world.structs.len()), request, &mut remaining)
                .unwrap();
            (prepared.commit)().unwrap();
        }
        let batches = publisher.into_definitions();
        for batch in batches {
            world.structs.extend(batch.iter().cloned());
        }
    }
    let definitions = world.structs.split_off(baseline.structs.len());
    black_box(&definitions);
    Measurement {
        elapsed: start.elapsed(),
        steps: ALLOWANCE - remaining,
        definitions,
    }
}

fn validate(left: &Measurement, right: &Measurement, requests: &[StructBuildRequest], base: usize) {
    assert_eq!(
        left.definitions, right.definitions,
        "strategies must publish identical definitions"
    );
    assert_eq!(left.definitions.len(), requests.len());
    for (index, (definition, request)) in left.definitions.iter().zip(requests).enumerate() {
        assert_eq!(definition.declaration, id(base + index));
        assert_eq!(definition.name, request.name);
        assert_eq!(definition.fields.len(), request.fields.len());
        for ((name, ty), field) in definition.fields.iter().zip(&request.fields) {
            assert_eq!(name, &field.name);
            assert_eq!(ty, &field.ty);
        }
    }
}

fn report(strategy: &str, baseline: usize, count: usize, measurements: &mut [(Duration, u64)]) {
    measurements.sort_by_key(|(duration, _)| *duration);
    let median = measurements[measurements.len() / 2];
    let steps = median.1;
    assert!(
        measurements.iter().all(|(_, work)| *work == steps),
        "metered work must be repeatable"
    );
    println!(
        "{strategy},{baseline},{count},4,{},{:.3},{:.3},{:.3},{steps}",
        measurements.len(),
        median.0.as_secs_f64() * 1e6,
        measurements[0].0.as_secs_f64() * 1e6,
        measurements.last().unwrap().0.as_secs_f64() * 1e6
    );
}

fn main() {
    let mut samples = 5usize;
    let mut base = 64usize;
    let mut counts = vec![64usize, 256, 1024];
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--samples" => {
                samples = args
                    .next()
                    .expect("--samples requires a number")
                    .parse()
                    .unwrap()
            }
            "--baseline" => {
                base = args
                    .next()
                    .expect("--baseline requires a number")
                    .parse()
                    .unwrap()
            }
            "--counts" => {
                counts = args
                    .next()
                    .expect("--counts requires comma-separated numbers")
                    .split(',')
                    .map(|n| n.parse().unwrap())
                    .collect()
            }
            "--help" => {
                println!(
                    "builder-publication-bench [--samples 5] [--baseline 64] [--counts 64,256,1024]"
                );
                return;
            }
            _ => panic!("unknown argument: {argument}"),
        }
    }
    assert!(samples > 0 && base > 0 && counts.iter().all(|n| *n > 0));
    let world = baseline(base);
    println!("strategy,baseline,generated,fields,samples,median_us,min_us,max_us,steps");
    for count in counts {
        id(base.checked_add(count).expect("declaration count overflow"));
        let requests = requests(base, count);
        // Warm both strategies equally; alternate their measured execution order.
        validate(
            &reused(&world, &requests),
            &rebuilt(&world, &requests),
            &requests,
            base,
        );
        let mut reuse_times = Vec::with_capacity(samples);
        let mut rebuild_times = Vec::with_capacity(samples);
        for sample in 0..samples {
            let (reuse, rebuild) = if sample % 2 == 0 {
                (reused(&world, &requests), rebuilt(&world, &requests))
            } else {
                let rebuild = rebuilt(&world, &requests);
                (reused(&world, &requests), rebuild)
            };
            validate(&reuse, &rebuild, &requests, base);
            reuse_times.push((reuse.elapsed, reuse.steps));
            rebuild_times.push((rebuild.elapsed, rebuild.steps));
        }
        report("reuse", base, count, &mut reuse_times);
        report("rebuild", base, count, &mut rebuild_times);
    }
}
