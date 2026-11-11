//! Invocation-local binding handoff for completed code fragments. Local names
//! are resolved against the exact Program owned by the frame before execution;
//! register positions never become cross-program binding identities.
use super::*;

/// Requested new bindings, disjoint from the caller environment. This is not
/// domain admission: the checked code still supplies each binding's value type.
pub(super) struct Exports {
    bindings: Vec<LocalId>,
    source: SourceRange,
}

pub(super) struct BoundExports {
    bindings: Vec<(LocalId, usize)>,
    source: SourceRange,
}

impl Exports {
    pub(super) fn new(
        vm: &mut Interpreter<'_>,
        bindings: Vec<LocalId>,
        environment: &Environment,
        source: SourceRange,
    ) -> Result<Self, Error> {
        let mut seen = BTreeSet::new();
        for binding in &bindings {
            vm.tick(Some(source))?;
            if environment.contains_key(binding) || !seen.insert(*binding) {
                return Err(vm.failure(ErrorKind::Type, Some(source)));
            }
        }
        Ok(Self { bindings, source })
    }
}

impl fmt::Display for Exports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(requested-exports (source {} {}) (bindings",
            self.source.start, self.source.end
        )?;
        for binding in &self.bindings {
            write!(f, " {binding}")?;
        }
        f.write_str("))")
    }
}

impl fmt::Display for BoundExports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(bound-exports (source {} {}) (bindings",
            self.source.start, self.source.end
        )?;
        for (binding, slot) in &self.bindings {
            write!(f, " ({binding} %{slot})")?;
        }
        f.write_str("))")
    }
}

impl Interpreter<'_> {
    pub(super) fn start_frame_with_exports(
        &mut self,
        code: &Arc<Program>,
        environment: &mut Environment,
        requested: Exports,
    ) -> Result<Frame, Error> {
        let source = requested.source;
        let mut bindings = Vec::with_capacity(requested.bindings.len());
        for binding in requested.bindings {
            self.tick(Some(source))?;
            // Recheck collisions at the actual transfer boundary. A request
            // may have been prepared before other input fragments completed.
            if environment.contains_key(&binding) {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            let index = code
                .locals()
                .binary_search_by_key(&binding, |(id, _)| *id)
                .map_err(|_| self.failure(ErrorKind::MissingBinding, Some(source)))?;
            bindings.push((binding, code.locals()[index].1));
        }
        // start_frame charges allocation work before taking the environment.
        // Failure anywhere above leaves the caller's entire environment intact.
        let mut frame = self.start_frame(code, environment)?;
        frame.exports = Some(BoundExports { bindings, source });
        Ok(frame)
    }
}

impl Frame {
    pub(super) fn complete_environment(
        mut self,
        mut flow: Result<Flow, Error>,
    ) -> (Result<Flow, Error>, Environment) {
        if let Some(exports) = self.exports.take() {
            if matches!(&flow, Ok(Flow::Value(_))) {
                let invalid = if !self.terminal || self.waiting.is_some() || !self.scopes.is_empty()
                {
                    Some(ErrorKind::InvalidControlFlow)
                } else if exports
                    .bindings
                    .iter()
                    .any(|(binding, _)| self.environment.contains_key(binding))
                {
                    Some(ErrorKind::Type)
                } else if exports
                    .bindings
                    .iter()
                    .any(|(_, slot)| self.slots.get(*slot).and_then(Option::as_ref).is_none())
                {
                    Some(ErrorKind::MissingBinding)
                } else {
                    None
                };
                if let Some(kind) = invalid {
                    flow = Err(Error {
                        kind,
                        source: Some(exports.source),
                    });
                } else {
                    // Validate the whole handoff before making any new binding
                    // visible. Values move out of slots, preserving storage and
                    // value-reference identity; no source Copy/Clone is invoked.
                    for (binding, slot) in exports.bindings {
                        let value = self.slots[slot].take().expect("validated export");
                        self.environment.insert(binding, value);
                    }
                }
            }
        }
        // Errors and non-local control still return original inputs with their
        // actual mutations. They never publish partially initialized exports.
        (flow, self.restore_environment())
    }
}
