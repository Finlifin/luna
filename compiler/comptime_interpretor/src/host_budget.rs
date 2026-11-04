//! Reserve execution allowance before host entry; settle only a returned call.
//! An unwind leaves no reusable allowance for work whose cost is unknown.
use super::*;

fn reserved<T>(steps: &mut u64, host: impl FnOnce(&mut u64) -> T) -> (T, bool) {
    let granted = std::mem::take(steps);
    let mut remaining = granted;
    let result = host(&mut remaining);
    *steps = granted.min(remaining);
    (result, remaining > granted)
}

/// Builder and proof services cannot replenish their caller's allowance.
pub(super) fn clamped<T>(steps: &mut u64, host: impl FnOnce(&mut u64) -> T) -> T {
    reserved(steps, host).0
}

/// Selection protocols reject a replenishing response before admitting it.
pub(super) fn checked<T>(
    steps: &mut u64,
    source: Option<SourceRange>,
    host: impl FnOnce(&mut u64) -> Result<T, Error>,
) -> Result<T, Error> {
    let (result, replenished) = reserved(steps, host);
    if replenished {
        *steps = 0;
        Err(Error {
            kind: ErrorKind::StepsExhausted,
            source,
        })
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn failure() -> Error {
        Error {
            kind: ErrorKind::UnmaterializedDispatch,
            source: None,
        }
    }

    #[test]
    fn returned_errors_and_values_settle_spending_without_refunding() {
        for strict in [false, true] {
            for fails in [false, true] {
                let mut steps = 10;
                let host = |remaining: &mut u64| {
                    *remaining -= 4;
                    if fails { Err(failure()) } else { Ok(42) }
                };
                let result = if strict {
                    checked(&mut steps, None, host)
                } else {
                    clamped(&mut steps, host)
                };
                assert_eq!(steps, 6);
                if fails {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::UnmaterializedDispatch);
                } else {
                    assert_eq!(result.unwrap(), 42);
                }
            }
        }
    }

    #[test]
    fn refill_policy_is_preserved_on_both_success_and_error() {
        let source = Some(SourceRange { start: 4, end: 8 });
        for strict in [false, true] {
            for fails in [false, true] {
                let mut steps = 10;
                let host = |remaining: &mut u64| {
                    *remaining = u64::MAX;
                    if fails { Err(failure()) } else { Ok(42) }
                };
                let result = if strict {
                    checked(&mut steps, source, host)
                } else {
                    clamped(&mut steps, host)
                };
                assert_eq!(steps, if strict { 0 } else { 10 });
                if strict {
                    let error = result.unwrap_err();
                    assert_eq!(error.kind, ErrorKind::StepsExhausted);
                    assert_eq!(error.source, source);
                } else if fails {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::UnmaterializedDispatch);
                } else {
                    assert_eq!(result.unwrap(), 42);
                }
            }
        }
    }

    #[test]
    fn unwinding_cannot_leave_spent_or_fabricated_allowance() {
        for strict in [false, true] {
            for remaining in [3, u64::MAX] {
                let mut steps = 10;
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let host = |fuel: &mut u64| -> Result<(), Error> {
                        *fuel = remaining;
                        panic!("host interrupted before accounting");
                    };
                    if strict {
                        checked(&mut steps, None, host)
                    } else {
                        clamped(&mut steps, host)
                    }
                }));
                assert!(result.is_err());
                assert_eq!(steps, 0);
            }
        }
    }

    #[test]
    fn type_solver_unwind_requires_an_explicit_new_allowance() {
        let package = Package::default();
        let predicate = nir::program::TypePredicate::Equal(
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Bool),
        );
        let calls = std::cell::Cell::new(0);
        let solver = |_: &nir::program::TypePredicate, fuel: &mut u64| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                *fuel = u64::MAX;
                panic!("solver interrupted");
            }
            *fuel -= 1;
            ContractOutcome::Satisfied
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_budgeted_type_solver(&solver);
        assert!(catch_unwind(AssertUnwindSafe(|| vm.solve_type_predicate(&predicate))).is_err());
        assert_eq!(vm.remaining_steps(), 0);
        assert_eq!(
            vm.solve_type_predicate(&predicate),
            ContractOutcome::Unknown
        );
        assert_eq!(calls.get(), 1);
        vm.budget.steps = 10;
        assert_eq!(
            vm.solve_type_predicate(&predicate),
            ContractOutcome::Satisfied
        );
        assert_eq!(vm.remaining_steps(), 9);
        assert_eq!(calls.get(), 2);
    }
}
