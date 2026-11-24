//! Validate identity-bearing capture graphs without copying their owned cells.
//! The visited identities live only for this observation: mutable cells must be
//! checked again on later calls, and shared callbacks retain their identity.
use super::*;

impl Value {
    /// Visit immediate aggregate payloads in source order. Object entries carry
    /// their own domains. Closure environments and storage require their owner's
    /// separate authority and are never followed by this operation.
    pub fn try_for_each_data_field<'v, E>(
        &'v self,
        mut visit: impl FnMut(Option<&'v Term>, &'v Value) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::Tuple(fields) | Self::Record { fields, .. } | Self::Variant { fields, .. } => {
                fields.iter().try_for_each(|field| visit(None, field))
            }
            Self::List { values, .. } => values.iter().try_for_each(|value| visit(None, value)),
            Self::AnonymousRecord(fields) => {
                fields.iter().try_for_each(|(_, value)| visit(None, value))
            }
            Self::Object(object) => object.entries().iter().try_for_each(|entry| {
                let field = entry.value();
                visit(Some(&field.ty), &field.value)
            }),
            Self::Optional { value, .. } => value.iter().try_for_each(|value| visit(None, value)),
            Self::Newtype { value, .. } | Self::ErrorResult { value, .. } => visit(None, value),
            Self::Code(_)
            | Self::Closure(_)
            | Self::ComptimeReference(_)
            | Self::Pointer(_)
            | Self::Storage(_)
            | Self::Type(_)
            | Self::NormalForm { .. }
            | Self::Description(_)
            | Self::TypeParameter(_)
            | Self::AbstractValue { .. }
            | Self::Function(_)
            | Self::FunctionInstance { .. }
            | Self::IirCallable(_)
            | Self::RuntimeTraitMethod(_)
            | Self::Unit
            | Self::Bool(_)
            | Self::Char(_)
            | Self::Symbol(_)
            | Self::String(_)
            | Self::SignedInteger(_)
            | Self::UnsignedInteger(_)
            | Self::Integer(_)
            | Self::Float(_)
            | Self::Float16(_)
            | Self::BFloat16(_)
            | Self::Float32(_)
            | Self::Float64(_)
            | Self::BitVec(_) => Ok(()),
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn check_comptime_graph(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
    ) -> Result<(), Error> {
        self.check_comptime_graph_in(value, Some(source), depth)
    }

    pub(super) fn check_comptime_graph_in(
        &mut self,
        value: &Value,
        source: Option<SourceRange>,
        depth: usize,
    ) -> Result<(), Error> {
        // Only for visited-node deduplication; never iterated.
        let mut seen = BTreeSet::new();
        self.check_comptime_graph_with_seen_in(value, source, depth, &mut seen)
    }

    pub(super) fn check_comptime_graph_with_seen(
        &mut self,
        value: &Value,
        source: SourceRange,
        depth: usize,
        seen: &mut BTreeSet<(u8, usize)>,
    ) -> Result<(), Error> {
        self.check_comptime_graph_with_seen_in(value, Some(source), depth, seen)
    }

    pub(super) fn check_comptime_graph_with_seen_in(
        &mut self,
        value: &Value,
        source: Option<SourceRange>,
        depth: usize,
        seen: &mut BTreeSet<(u8, usize)>,
    ) -> Result<(), Error> {
        self.check_comptime_graph_inner(value, source, depth, seen)
    }

    fn check_comptime_graph_inner(
        &mut self,
        value: &Value,
        source: Option<SourceRange>,
        depth: usize,
        seen: &mut BTreeSet<(u8, usize)>,
    ) -> Result<(), Error> {
        self.tick(source)?;
        if depth >= 128 || self.depth.saturating_add(depth) >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, source));
        }
        match value {
            Value::Pointer(_) => return Err(self.failure(ErrorKind::PhaseMismatch, source)),
            Value::Storage(storage) => {
                if !seen.insert((0, Arc::as_ptr(&storage.0) as usize)) {
                    return Ok(());
                }
                let contents = storage
                    .contents_for_copy(
                        &mut self.budget.steps,
                        self.budget.depth.saturating_sub(self.depth + depth),
                    )
                    .map_err(|kind| self.failure(kind, source))?;
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
            Value::IirCallable(handle) => {
                self.iir_target(handle)?;
                self.check_comptime_domain_in(&handle.ty, source)?;
            }
            Value::Closure(closure) => {
                if !seen.insert((1, Arc::as_ptr(closure) as usize)) {
                    return Ok(());
                }
                self.check_comptime_domain_in(&closure.ty, source)?;
                for (_, value) in &closure.environment {
                    self.check_comptime_graph_inner(value, source, depth + 1, seen)?;
                }
            }
            Value::Record {
                declaration,
                arguments,
                ..
            }
            | Value::Variant {
                declaration,
                arguments,
                ..
            } => {
                // A shared cell can change after capture. Inspect the actual
                // nominal domain, including inactive enum alternatives, rather
                // than trusting the closure's original environment signature.
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let charged = arguments.iter().try_for_each(|argument| {
                    nir::copy_budget::charge_term_copy(argument, &mut fuel, 128)
                });
                self.budget.steps -= (before - fuel) as u64;
                charged.map_err(|error| {
                    self.failure(
                        match error {
                            nir::copy_budget::CopyBudgetExhausted::Steps => {
                                ErrorKind::StepsExhausted
                            }
                            nir::copy_budget::CopyBudgetExhausted::Depth => {
                                ErrorKind::DepthExhausted
                            }
                        },
                        source,
                    )
                })?;
                self.check_comptime_domain_in(
                    &Term::Nominal {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    },
                    source,
                )?;
            }
            Value::List { ty, values } => {
                self.check_comptime_domain_in(ty, source)?;
                if values
                    .summary()
                    .is_some_and(|summary| summary.scalars.is_some())
                {
                    // Uniform primitive leaves carry no mutable identities or
                    // domains. Still check the child depth in this invocation.
                    if let Some(first) = values.first() {
                        self.check_comptime_graph_inner(first, source, depth + 1, seen)?;
                    }
                    return Ok(());
                }
            }
            Value::Optional { ty, .. }
            | Value::Newtype { ty, .. }
            | Value::ErrorResult { ty, .. } => self.check_comptime_domain_in(ty, source)?,
            // Type values describe storage; code and function identities do
            // not contain an invocation-local captured payload graph.
            _ => {}
        }
        value.try_for_each_data_field(|domain, child| {
            if let Some(domain) = domain {
                self.check_comptime_domain_in(domain, source)?;
            }
            self.check_comptime_graph_inner(child, source, depth + 1, seen)
        })
    }
}
