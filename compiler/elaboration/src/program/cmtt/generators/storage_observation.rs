//! Invocation-local storage work reserves the caller's allowance before world
//! preparation. Neither failure nor unwinding can refund unaccounted work.
use super::*;
use comptime_interpretor::{Budget, Interpreter};

#[derive(Debug)]
enum Failure {
    Preparation(comptime_interpretor::Error),
    Operation(comptime_interpretor::Error),
}

fn observe<T>(
    package: &n::Package,
    steps: &mut u64,
    operation: impl FnOnce(&mut Interpreter<'_>) -> Result<T, comptime_interpretor::Error>,
) -> Result<T, Failure> {
    let mut budget = Budget {
        steps: std::mem::take(steps),
        depth: 128,
    };
    let mut vm = Interpreter::for_comptime_in_budget(package, &[], &mut budget)
        .map_err(Failure::Preparation)?;
    let result = operation(&mut vm);
    *steps = vm.remaining_steps();
    result.map_err(Failure::Operation)
}

pub(super) fn with_storage_vm<T>(
    package: &n::Package,
    steps: &mut u64,
    span: Span,
    label: &str,
    action: &str,
    operation: impl FnOnce(&mut Interpreter<'_>) -> Result<T, comptime_interpretor::Error>,
) -> Result<T, Error> {
    observe(package, steps, operation).map_err(|failure| {
        let (phase, failure) = match failure {
            Failure::Preparation(failure) => ("preparation", failure),
            Failure::Operation(failure) => (action, failure),
        };
        error(span, format!("{label} {phase} failed: {failure}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use comptime_interpretor::{ErrorKind, Storage, Value};

    fn world() -> n::Package {
        n::Package {
            structs: vec![n::Struct {
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                name: "Payload".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                fields: (0..16).map(|i| (format!("field{i}"), p(P::Bool))).collect(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn failed_preparation_reserves_work_and_never_enters_storage() {
        let package = world();
        let mut steps = 1;
        let failure = observe(&package, &mut steps, |_| -> Result<(), _> {
            panic!("failed preparation must not enter the operation")
        })
        .unwrap_err();
        assert!(
            matches!(failure, Failure::Preparation(ref e) if e.kind == ErrorKind::StepsExhausted)
        );
        assert_eq!(steps, 0);
        assert!(matches!(
            observe(&package, &mut steps, |_| Ok(())),
            Err(Failure::Preparation(_))
        ));
        let storage = Storage::new(Value::Bool(true));
        steps = 100_000;
        assert_eq!(
            observe(&package, &mut steps, |vm| vm.read_owned_value(&storage)).unwrap(),
            Value::Bool(true)
        );
        assert!(steps < 100_000);
    }

    #[test]
    fn returned_operation_errors_settle_work_but_unwind_keeps_the_reservation() {
        let package = world();
        let storage = Storage::new(Value::Bool(true));
        let mut steps = 100_000;
        let mut after_read = 0;
        let failure = observe(&package, &mut steps, |vm| -> Result<(), _> {
            vm.read_owned_value(&storage)?;
            after_read = vm.remaining_steps();
            Err(comptime_interpretor::Error {
                kind: ErrorKind::Type,
                source: None,
            })
        })
        .unwrap_err();
        assert!(matches!(failure, Failure::Operation(ref e) if e.kind == ErrorKind::Type));
        assert!(after_read > 0 && after_read < 100_000);
        assert_eq!(steps, after_read);
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            observe(&package, &mut steps, |vm| -> Result<(), _> {
                vm.read_owned_value(&storage)?;
                panic!("interrupted storage observation")
            })
        }));
        assert!(interrupted.is_err());
        assert_eq!(steps, 0);
        assert_eq!(storage.read().unwrap(), Value::Bool(true));
    }

    #[test]
    fn observations_share_live_cells_but_entry_copies_own_their_snapshots() {
        let package = world();
        let nested = Storage::new(Value::Bool(true));
        let root = Storage::new(Value::Tuple(vec![Value::Storage(nested.clone())]));
        let mut steps = 100_000;
        let Value::Tuple(read) =
            observe(&package, &mut steps, |vm| vm.read_owned_value(&root)).unwrap()
        else {
            panic!("tuple")
        };
        assert_eq!(read[0], Value::Storage(nested.clone()));
        let Value::Tuple(copied) =
            observe(&package, &mut steps, |vm| vm.copy_owned_value(&root)).unwrap()
        else {
            panic!("tuple")
        };
        let Value::Storage(copied) = &copied[0] else {
            panic!("nested cell")
        };
        assert_ne!(*copied, nested);
        assert_eq!(copied.read().unwrap(), Value::Bool(true));
        let checkpoint = observe(&package, &mut steps, |vm| {
            vm.checkpoint_owned_graph(&[Value::Storage(root)])
        })
        .unwrap();
        assert!(steps < 100_000);
        // Restoration was paid for when the graph was captured; it must not
        // require a new observation VM or a new allowance.
        steps = 0;
        checkpoint.restore().unwrap();
        assert_eq!(steps, 0);
    }
}
