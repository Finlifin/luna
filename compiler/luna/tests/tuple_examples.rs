//! Keep standalone tuple examples on the same explicit constructor contract as source tests.
use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn migrated_tuple_examples_compile_with_their_source_protocols() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../development/examples");
    for name in [
        "anonymous-record-patterns.fl",
        "associated-boolean-contracts.fl",
        "associated-callable-inputs.fl",
        "capability-dependent-bounds.fl",
        "capability-indexed-inputs.fl",
        "comptime-type-inputs.fl",
        "computed-float-defaults.fl",
        "contextual-calls.fl",
        "copied-disjunctive-contracts.fl",
        "default-dispatch.fl",
        "default-suppliers.fl",
        "deferred-float-computations.fl",
        "deferred-integer-comparisons.fl",
        "dependent-boolean-operators.fl",
        "dependent-comptime-inputs.fl",
        "dependent-default-inputs.fl",
        "dependent-packet-inputs.fl",
        "dependent-scalar-closures.fl",
        "dependent-type-values.fl",
        "dependent-value-inputs.fl",
        "elaboration.fl",
        "floating-comptime-inputs.fl",
        "generic-method-values.fl",
        "generic-methods.fl",
        "generic-trait-bounds.fl",
        "generic-trait-contracts.fl",
        "generic-trait-dispatch.fl",
        "generic-trait-method-values.fl",
        "inferred-dependent-defaults.fl",
        "inherited-members.fl",
        "mixed-collections.fl",
        "mixed-dependent-generics.fl",
        "mixed-property-inputs.fl",
        "multiple-collectors.fl",
        "ordinary-type-capabilities.fl",
        "ordinary-type-predicates.fl",
        "pure-copy.fl",
        "pure-transfers.fl",
        "scoped-generic-inputs.fl",
        "splice-higher-order.fl",
        "staged-arithmetic.fl",
        "staged-associated-callable.fl",
        "staged-bitvec.fl",
        "staged-capture-copy.fl",
        "staged-closures.fl",
        "staged-computed-spreads.fl",
        "staged-destructuring.fl",
        "staged-fields.fl",
        "staged-invocation.fl",
        "staged-literal-spreads.fl",
        "staged-parameters.fl",
        "staged-patterns.fl",
        "staged-record-inputs.fl",
        "staged-trait-dispatch.fl",
        "staged-type-closure-inputs.fl",
        "staged-value-closure-inputs.fl",
        "trait-methods.fl",
        "trait-self.fl",
        "unsigned-division.fl",
        "unsigned-remainder.fl",
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
        command
            .arg(root.join(name))
            .args([
                "--meta-list-type",
                "input::TupleElements",
                "--tuple-type",
                "input::Tuple",
            ])
            .stdin(Stdio::null());
        match name {
            "pure-copy.fl" => {
                command.args([
                    "--interpret",
                    "run",
                    "--copy-protocol",
                    "Copy",
                    "--drop-protocol",
                    "Drop",
                ]);
            }
            "splice-higher-order.fl" => {
                command.args(["--interpret", "run", "--splice-protocol", "Splice"]);
            }
            _ => {
                command.args(["--emit", "nir"]);
            }
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        assert!(!output.trim().is_empty(), "{name}: missing compiler result");
    }
}
