//! Invocation-local binding handoff for completed code fragments. Frames share their lexical
//! export layout with the code and publish values only on normal completion.
use super::*;

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
                    .bindings()
                    .iter()
                    .any(|(binding, _)| self.environment.contains_key(binding))
                {
                    Some(ErrorKind::Type)
                } else if exports
                    .bindings()
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
                        source: Some(exports.source()),
                    });
                } else {
                    // Validate the whole handoff before making any new binding
                    // visible. Values move out of slots, preserving storage and
                    // value-reference identity; no source Copy/Clone is invoked.
                    for &(binding, slot) in exports.bindings() {
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
