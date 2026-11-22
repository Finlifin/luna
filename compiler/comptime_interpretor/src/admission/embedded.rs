//! Formation can re-enter the VM. Keep only the evidence it will consume
//! after a receiver borrow ends, in the same order as direct admission.
use super::*;

enum Check<'a> {
    Visit,
    Domain(&'a Term, usize),
    Payload(&'a Value, usize),
}

fn walk(
    value: &Value,
    depth: usize,
    check: &mut impl FnMut(Check<'_>) -> Result<(), Error>,
) -> Result<(), Error> {
    if depth >= 128 {
        return Err(Error {
            kind: ErrorKind::DepthExhausted,
            source: None,
        });
    }
    check(Check::Visit)?;
    match value {
        Value::Description(_) | Value::Type(_) | Value::List { .. } => {
            check(Check::Payload(value, depth))?;
        }
        Value::Optional { value, .. } => {
            if let Some(value) = value {
                walk(value, depth + 1, check)?;
            }
        }
        Value::ErrorResult { value, .. } | Value::Newtype { value, .. } => {
            walk(value, depth + 1, check)?;
        }
        Value::Tuple(fields) | Value::Record { fields, .. } | Value::Variant { fields, .. } => {
            for field in fields {
                walk(field, depth + 1, check)?;
            }
        }
        Value::AnonymousRecord(fields) => {
            for (_, field) in fields {
                walk(field, depth + 1, check)?;
            }
        }
        Value::Object(object) => {
            for entry in object.entries() {
                check(Check::Domain(&entry.value().ty, depth + 1))?;
                walk(&entry.value().value, depth + 1, check)?;
            }
        }
        _ => {}
    }
    Ok(())
}

enum Retained {
    Domain(Term),
    Payload(Value),
}

#[derive(Default)]
struct EmbeddedChecks {
    pending: Vec<(u64, usize, Retained)>,
    visits: u64,
}

impl EmbeddedChecks {
    /// Representation validation has already paid for and bounded the tree.
    /// A subtree with no formation work contributes only a visit count.
    fn capture(value: &Value) -> Result<Self, Error> {
        let mut checks = Self::default();
        walk(value, 0, &mut |check| {
            let (depth, retained) = match check {
                Check::Visit => {
                    checks.visits += 1;
                    return Ok(());
                }
                Check::Domain(ty, depth) => (depth, Retained::Domain(ty.clone())),
                Check::Payload(value, depth) => (depth, Retained::Payload(value.clone())),
            };
            checks
                .pending
                .push((std::mem::take(&mut checks.visits), depth, retained));
            Ok(())
        })?;
        Ok(checks)
    }

    fn discharge(self, vm: &mut Interpreter<'_>) -> Result<(), Error> {
        for (visits, depth, retained) in self.pending {
            vm.charge_embedded_visits(visits)?;
            match retained {
                Retained::Domain(ty) => vm.admit_type(&ty, depth)?,
                Retained::Payload(value) => vm.admit_value_types(&value, depth)?,
            }
        }
        vm.charge_embedded_visits(self.visits)
    }
}

impl Interpreter<'_> {
    pub(crate) fn admit_embedded_type_descriptions(
        &mut self,
        value: &Value,
        depth: usize,
    ) -> Result<(), Error> {
        walk(value, depth, &mut |check| match check {
            Check::Visit => self.tick(None),
            Check::Domain(ty, depth) => self.admit_type(ty, depth),
            Check::Payload(value, depth) => self.admit_value_types(value, depth),
        })
    }

    fn charge_embedded_visits(&mut self, count: u64) -> Result<(), Error> {
        if count == 0 {
            return Ok(());
        }
        if self.depth >= self.budget.depth {
            return self.tick(None);
        }
        if self.budget.steps < count {
            self.budget.steps = 0;
            return Err(self.failure(ErrorKind::StepsExhausted, None));
        }
        self.budget.steps -= count;
        Ok(())
    }

    /// The caller has already observed the parameter domain, including every
    /// associated type. Shape inspection only collects formation obligations.
    pub(crate) fn admit_reference_input(
        &mut self,
        reference: &ValueReference,
        domain: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        let (obligations, embedded) = {
            let guard = reference
                .borrow(&mut self.budget.steps)
                .map_err(|kind| self.failure(kind, Some(source)))?;
            let value = guard.value();
            self.check_borrowed_representation(value, source, 0)?;
            let obligations = self.value_obligations(value, domain, Some(source))?;
            (obligations, EmbeddedChecks::capture(value)?)
        };
        self.discharge_value_obligations(obligations, Some(source))?;
        embedded.discharge(self)
    }
}
