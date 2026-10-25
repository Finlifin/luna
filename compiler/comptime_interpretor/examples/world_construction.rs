//! Constructor-only benchmark; package allocation and destruction are not timed.
//! Compare release-mode medians on the same host; do not use timing as a test gate.
use comptime_interpretor::{Budget, Interpreter};
use nir::program::{
    Expr, ExprKind, Function, FunctionPhase, Literal, Package, RuntimeFunctionIdentity, SourceRange,
};
use nir::{DeclarationId, Primitive, Term};
use std::{hint::black_box, time::Instant};

fn main() {
    for count in [1000u32, 4000, 16000] {
        let id = |local| DeclarationId { package: 1, local };
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        };
        let function = Function {
            declaration: id(0),
            name: "entry".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            signature: signature.clone(),
            parameters: vec![],
            defaults: vec![],
            body: Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: boolean,
                kind: ExprKind::Literal(Literal::Bool(true)),
            },
        };
        let package = Package {
            functions: (0..count)
                .map(|i| Function {
                    declaration: id(i),
                    ..function.clone()
                })
                .collect(),
            runtime_function_identities: (0..count)
                .map(|i| RuntimeFunctionIdentity {
                    declaration: id(count + i),
                    type_parameters: vec![],
                    signature: signature.clone(),
                })
                .collect(),
            ..Default::default()
        };
        let budget = Budget {
            steps: 10_000_000,
            depth: 64,
        };
        drop(Interpreter::for_comptime(&package, budget).unwrap());
        let mut times = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            let vm = Interpreter::for_comptime(black_box(&package), budget).unwrap();
            times.push(start.elapsed());
            black_box(vm);
        }
        times.sort();
        println!(
            "world_entries={} median_us={}",
            count * 2,
            times[3].as_micros()
        );
    }
}
