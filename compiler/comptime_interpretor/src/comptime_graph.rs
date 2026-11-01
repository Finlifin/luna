//! Validate identity-bearing capture graphs without copying their owned cells.
//! The visited identities live only for this observation: mutable cells must be
//! checked again on later calls, and shared callbacks retain their identity.
use super::*;

impl Interpreter<'_> {
    pub(super) fn check_comptime_graph(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        self.check_comptime_graph_inner(value, source, depth, &mut HashSet::new())
    }

    fn check_comptime_graph_inner(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
        seen: &mut HashSet<(u8, usize)>,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        if depth >= 128 || self.depth.saturating_add(depth) >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        match value {
            Value::Pointer(_) => return Err(self.failure(ErrorKind::PhaseMismatch, Some(source))),
            Value::Storage(storage) => {
                if !seen.insert((0, Arc::as_ptr(&storage.0) as usize)) {
                    return Ok(());
                }
                let contents = storage
                    .contents_for_copy(
                        &mut self.budget.steps,
                        self.budget.depth.saturating_sub(self.depth + depth),
                    )
                    .map_err(|kind| self.failure(kind, Some(source)))?;
                self.check_comptime_graph_inner(contents.value(), source, depth + 1, seen)?;
            }
            Value::ComptimeReference(reference) => {
                // Inspect the owner once, including sibling captures, rather
                // than treating two projected paths as identical payloads.
                self.check_comptime_graph_inner(
                    &Value::Storage(reference.storage.clone()),
                    source,
                    depth + 1,
                    seen,
                )?;
            }
            Value::Closure(closure) => {
                if !seen.insert((1, Arc::as_ptr(closure) as usize)) {
                    return Ok(());
                }
                self.check_comptime_domain(&closure.ty, source)?;
                for (_, value) in &closure.environment {
                    self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
                }
            }
            Value::Object(object) => {
                for entry in object.entries() {
                    self.check_comptime_domain(&entry.value().ty, source)?;
                    self.check_comptime_graph_inner(&entry.value().value, source, depth + 1, seen)?;
                }
            }
            Value::Record {
                declaration,
                arguments,
                fields,
            }
            | Value::Variant {
                declaration,
                arguments,
                fields,
                ..
            } => {
                // A shared cell can change after capture. Inspect the actual
                // nominal domain, including inactive enum alternatives, rather
                // than trusting the closure's original environment signature.
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let charged = arguments.iter().try_for_each(|argument| {
                    nir::type_computations::charge_term(argument, &mut fuel)
                });
                self.budget.steps -= (before - fuel) as u64;
                charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
                self.check_comptime_domain(
                    &Term::Nominal {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    },
                    source,
                )?;
                for value in fields {
                    self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
                }
            }
            Value::Tuple(values) => {
                for value in values {
                    self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
                }
            }
            Value::List { ty, values } => {
                self.check_comptime_domain(ty, source)?;
                for value in values {
                    self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
                }
            }
            Value::AnonymousRecord(fields) => {
                for (_, value) in fields {
                    self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
                }
            }
            Value::Optional { ty, value } => {
                self.check_comptime_domain(ty, source)?;
                if let Some(value) = value {
                    self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
                }
            }
            Value::Newtype { ty, value } | Value::ErrorResult { ty, value, .. } => {
                self.check_comptime_domain(ty, source)?;
                self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
            }
            // Type values describe storage; code and function identities do
            // not contain an invocation-local captured payload graph.
            _ => {}
        }
        Ok(())
    }
}
