//! Generated bodies and the recursive instance reservations that name them.
use super::*;
use std::{collections::HashMap, sync::Arc};

#[derive(PartialEq, Eq, Hash)]
pub(super) struct InstanceKey {
    pub owner: OwnerId,
    pub declaration_inputs: Vec<Term>,
    pub ordinary_inputs: Vec<Option<Term>>,
    pub signature: Term,
}

#[derive(Clone, Copy)]
pub(super) struct Checkpoint {
    reservations: usize,
    bodies: usize,
}

#[derive(Default)]
pub(super) struct GeneratedFunctions {
    next: u32,
    instances: HashMap<Arc<InstanceKey>, usize>,
    reservations: Vec<(Arc<InstanceKey>, nir::function_parameters::Declaration)>,
    bodies: Vec<n::Function>,
    positions: BTreeMap<DeclarationId, usize>,
}

impl GeneratedFunctions {
    pub(super) fn fresh_identity(&mut self, span: Span) -> Result<DeclarationId, Error> {
        let local = self.next;
        self.next = local
            .checked_add(1)
            .ok_or_else(|| error(span, "generated function identity exhausted"))?;
        Ok(DeclarationId {
            package: u64::MAX - 1,
            local,
        })
    }

    pub(super) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            reservations: self.reservations.len(),
            bodies: self.bodies.len(),
        }
    }

    pub(super) fn instance(&self, key: &InstanceKey) -> Option<DeclarationId> {
        let &slot = self.instances.get(key)?;
        Some(self.reservations[slot].1.declaration)
    }

    pub(super) fn reserve(
        &mut self,
        key: InstanceKey,
        inputs: nir::function_parameters::Declaration,
    ) {
        let key = Arc::new(key);
        let previous = self.instances.insert(key.clone(), self.reservations.len());
        debug_assert!(previous.is_none());
        self.reservations.push((key, inputs));
    }

    pub(super) fn parameter_declarations(
        &self,
    ) -> impl Iterator<Item = &nir::function_parameters::Declaration> {
        self.reservations.iter().map(|(_, inputs)| inputs)
    }

    pub(super) fn publish(&mut self, function: n::Function) {
        let previous = self
            .positions
            .insert(function.declaration, self.bodies.len());
        debug_assert!(previous.is_none());
        self.bodies.push(function);
    }

    pub(super) fn body(&self, id: DeclarationId) -> Option<&n::Function> {
        self.positions.get(&id).map(|slot| &self.bodies[*slot])
    }

    // Nested successes belong to their enclosing attempt. Identities already
    // issued may occur in diagnostics or retained work and are never recycled.
    pub(super) fn rollback(&mut self, checkpoint: Checkpoint) {
        for function in self.bodies.drain(checkpoint.bodies..) {
            self.positions.remove(&function.declaration);
        }
        for (key, _) in self.reservations.drain(checkpoint.reservations..) {
            self.instances.remove(&key);
        }
    }

    pub(super) fn into_bodies(self) -> Vec<n::Function> {
        self.bodies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(owner: u32) -> InstanceKey {
        InstanceKey {
            owner: OwnerId::new(hir::hir_id::LocalDefId::new(owner)),
            declaration_inputs: vec![],
            ordinary_inputs: vec![],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(p(P::Bool)),
            },
        }
    }

    #[test]
    fn enclosing_failure_retires_nested_reservations_without_reusing_identities() {
        let mut generated = GeneratedFunctions::default();
        let reserve = |generated: &mut GeneratedFunctions, owner| {
            let id = generated.fresh_identity(Span::default()).unwrap();
            generated.reserve(
                key(owner),
                nir::function_parameters::Declaration {
                    declaration: id,
                    parameters: vec![],
                    domains: vec![],
                },
            );
            id
        };
        let committed = reserve(&mut generated, 1);
        let outer = generated.checkpoint();
        let recursive = reserve(&mut generated, 2);
        let inner = generated.checkpoint();
        let failed = reserve(&mut generated, 3);
        generated.rollback(inner);
        assert_eq!(generated.instance(&key(2)), Some(recursive));
        assert_eq!(generated.instance(&key(3)), None);
        let retried = reserve(&mut generated, 3);
        assert_ne!(failed, retried);
        generated.rollback(outer);
        assert_eq!(generated.instance(&key(1)), Some(committed));
        assert_eq!(generated.instance(&key(2)), None);
        assert_eq!(generated.instance(&key(3)), None);
        let retained: Vec<_> = generated
            .parameter_declarations()
            .map(|d| d.declaration)
            .collect();
        assert_eq!(retained, [committed]);
    }
}
