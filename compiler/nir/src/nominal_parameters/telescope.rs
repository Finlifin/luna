//! The declaration-side index used by both structural substitution and
//! receiving-world admission. Only the latter consults value-domain evidence.
use super::*;
use crate::type_computations::Error as QueryError;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound::{Excluded, Unbounded};

pub(super) struct Binders {
    pub(super) ordered: Vec<DeclarationParameter>,
    pub(super) positions: BTreeMap<DeclarationId, (usize, Sort)>,
}

impl Binders {
    pub(super) fn new<T: Borrow<Term>>(
        ids: &[DeclarationId],
        domains: &[T],
    ) -> Result<Self, TypeError> {
        if !domains.is_empty() && domains.len() != ids.len() {
            return Err(TypeError::ArgumentCount {
                expected: ids.len(),
                actual: domains.len(),
            });
        }
        let mut ordered = Vec::with_capacity(ids.len());
        let mut positions = BTreeMap::new();
        for (slot, &id) in ids.iter().enumerate() {
            let domain = domains
                .get(slot)
                .map(Borrow::borrow)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let binder = DeclarationParameter::comptime(id, domain);
            if positions.insert(id, (slot, binder.sort)).is_some() {
                return Err(TypeError::DuplicateDeclarationParameter(id));
            }
            if domain.sort() != Sort::Type {
                return Err(TypeError::WrongSort {
                    expected: Sort::Type,
                    actual: domain.sort(),
                });
            }
            domain.check_scope(&[])?;
            ordered.push(binder);
        }
        Ok(Self { ordered, positions })
    }

    pub(super) fn bounded_substitution<D: Borrow<Term>, V: Borrow<Term>>(
        ids: &[DeclarationId],
        domains: &[D],
        arguments: &[V],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Self, QueryError> {
        use crate::type_computations::{Error, charge_term};
        // Scope checks borrow the caller's frames and look up referenced slots.
        // Retaining an owned copy is charged by into_owned.
        *fuel = fuel.checked_sub(ids.len()).ok_or(Error::Budget)?;
        for domain in domains {
            charge_term(domain.borrow(), fuel)?;
        }
        for argument in arguments {
            charge_term(argument.borrow(), fuel)?;
        }
        let parameters = Binders::new(ids, domains).map_err(Error::Scope)?;
        if arguments.len() != ids.len() {
            return Err(Error::Scope(TypeError::ArgumentCount {
                expected: ids.len(),
                actual: arguments.len(),
            }));
        }
        for (parameter, domain) in parameters.ordered.iter().zip(domains) {
            if parameter.sort != Sort::Type {
                parameters.charge_expansion(domain.borrow(), arguments, fuel)?;
            }
        }
        parameters
            .check_substitution(domains, arguments, frames)
            .map_err(Error::Scope)?;
        Ok(parameters)
    }

    pub(super) fn check_substitution<D: Borrow<Term>, V: Borrow<Term>>(
        &self,
        domains: &[D],
        arguments: &[V],
        frames: &[Vec<Sort>],
    ) -> Result<(), TypeError> {
        if self.ordered.len() != arguments.len() {
            return Err(TypeError::ArgumentCount {
                expected: self.ordered.len(),
                actual: arguments.len(),
            });
        }
        for argument in arguments {
            argument.borrow().check_scope(frames)?;
        }
        for (index, (parameter, argument)) in self.ordered.iter().zip(arguments).enumerate() {
            let expected = if parameter.sort == Sort::Type {
                Sort::Type
            } else {
                let domain = self.instantiate(domains[index].borrow(), arguments, frames)?;
                Sort::of_comptime_domain(&domain)
            };
            if expected != argument.borrow().sort() {
                return Err(TypeError::WrongSort {
                    expected,
                    actual: argument.borrow().sort(),
                });
            }
        }
        Ok(())
    }

    // Callers have checked arity, argument scope and the original binder
    // identities. Replacements are caller terms and are never substituted again.
    pub(super) fn instantiate<T: Borrow<Term>>(
        &self,
        term: &Term,
        arguments: &[T],
        frames: &[Vec<Sort>],
    ) -> Result<Term, TypeError> {
        term.instantiate_domain_checked_indices(&self.positions, arguments, frames)
    }

    pub(super) fn replacement<'a, T: Borrow<Term>>(
        &self,
        id: DeclarationId,
        arguments: &'a [T],
    ) -> Option<&'a Term> {
        self.positions
            .get(&id)
            .map(|&(index, _)| arguments[index].borrow())
    }

    pub(super) fn charge_expansion<T: Borrow<Term>>(
        &self,
        term: &Term,
        arguments: &[T],
        fuel: &mut usize,
    ) -> Result<(), QueryError> {
        crate::type_computations::visit_expansion(
            term,
            fuel,
            128,
            &|id| self.replacement(id, arguments),
            &mut |_, _| {},
        )
        .map_err(Into::into)
    }

    pub(super) fn instantiate_bounded<T: Borrow<Term>>(
        &self,
        term: &Term,
        arguments: &[T],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Term, QueryError> {
        self.charge_expansion(term, arguments, fuel)?;
        self.instantiate(term, arguments, frames)
            .map_err(QueryError::Scope)
    }

    pub(super) fn instantiate_expression<T: Borrow<Term>>(
        &self,
        template: &crate::program::Expr,
        arguments: &[T],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<crate::program::Expr, ExpressionInstantiationError> {
        crate::copy_budget::charge_projected(
            crate::copy_budget::Node::Expr(template),
            fuel,
            &|id| self.replacement(id, arguments),
        )
        .map_err(ExpressionInstantiationError::Copy)?;
        let mut value = template.clone();
        value
            .rewrite_types(&mut |ty| self.instantiate(ty, arguments, frames))
            .map_err(ExpressionInstantiationError::Scope)?;
        Ok(value)
    }

    /// Admit each value only after all local binders mentioned in its original
    /// domain have passed. External rigids belong to the caller's world.
    pub(super) fn admit<D: Borrow<Term>, T: Borrow<Term>>(
        &self,
        domains: &[D],
        arguments: &[T],
        frames: &[Vec<Sort>],
        first: usize,
        fuel: &mut usize,
        queries: &mut crate::value_admission::DomainQueries<'_, '_>,
    ) -> Result<(), DeclarationAdmissionFailure> {
        let count = self.ordered.len();
        *fuel = fuel.checked_sub(count).ok_or(AdmissionError::Budget)?;
        let mut dependents = vec![Vec::new(); count];
        let mut pending = vec![0usize; count];
        let mut ready = BTreeSet::new();
        for (slot, missing) in pending.iter_mut().enumerate() {
            if let Some(domain) = domains.get(slot) {
                let mut steps = *fuel as u64;
                let inputs = crate::copy_budget::term_inputs(domain.borrow(), &mut steps);
                *fuel = steps as usize;
                let inputs = inputs.map_err(|err| match err {
                    crate::copy_budget::InputObservationError::Limit(_) => AdmissionError::Budget,
                    crate::copy_budget::InputObservationError::Scope(e) => {
                        AdmissionError::Invalid(e.to_string())
                    }
                })?;
                for id in inputs.declarations.keys() {
                    if let Some(&(source, _)) = self.positions.get(id) {
                        dependents[source].push(slot);
                        *missing += 1;
                    }
                }
            }
            if *missing == 0 {
                ready.insert(slot);
            }
        }
        let mut cursor = first.checked_sub(1);
        let mut admitted = 0;
        while !ready.is_empty() {
            // Continue the declaration-order sweep. An earlier slot woken by
            // this sweep runs after the remaining ready slots to its right.
            let slot = cursor
                .and_then(|last| ready.range((Excluded(last), Unbounded)).next().copied())
                .unwrap_or_else(|| *ready.first().unwrap());
            ready.remove(&slot);
            cursor = Some(slot);
            let template = domains
                .get(slot)
                .map(Borrow::borrow)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let at = |stage, reason| DeclarationAdmissionFailure {
                slot: Some((slot, stage)),
                reason,
            };
            let domain = self
                .instantiate_bounded(template, arguments, frames, fuel)
                .map_err(|error| at(InputAdmissionStage::Domain, error.into()))?;
            let expected = Sort::of_comptime_domain(&domain);
            let argument = arguments[slot].borrow();
            if argument.sort() != expected {
                return Err(at(
                    InputAdmissionStage::Value,
                    AdmissionError::Invalid(
                        TypeError::WrongSort {
                            expected,
                            actual: argument.sort(),
                        }
                        .to_string(),
                    ),
                ));
            }
            if expected != Sort::Type {
                crate::value_admission::validate(argument, &domain, frames, fuel, queries)
                    .map_err(|error| at(InputAdmissionStage::Value, error))?;
            }
            admitted += 1;
            *fuel = fuel
                .checked_sub(dependents[slot].len())
                .ok_or(AdmissionError::Budget)?;
            for &consumer in &dependents[slot] {
                pending[consumer] -= 1;
                if pending[consumer] == 0 {
                    ready.insert(consumer);
                }
            }
        }
        if admitted != count {
            return Err(
                AdmissionError::from("declaration input domains have a cyclic dependency").into(),
            );
        }
        Ok(())
    }
}
