//! Interface inputs use declaration domains; symbolic leaves require the
//! invoking scope's separate authority. Package metadata is not that authority.
use super::*;
use nir::value_admission::Error;
use nir::{Primitive, Sort};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct ValueScope(BTreeMap<Id, Term>);

impl ValueScope {
    pub(super) fn new(parameters: &[(Id, Term)], fuel: &mut usize) -> Result<Self, Error> {
        *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
        let mut domains = BTreeMap::new();
        for (id, domain) in parameters {
            nir::type_computations::charge_term(domain, fuel)?;
            domain.check_scope(&[]).map_err(|e| e.to_string())?;
            if domain.sort() != Sort::Type || *domain == Term::Primitive(Primitive::Type) {
                return Err("dispatch value scope requires an ordinary value domain".into());
            }
            if let Some(previous) = domains.get(id) {
                nir::type_computations::charge_term(previous, fuel)?;
                if previous != domain {
                    return Err("dispatch value scope contains conflicting domains".into());
                }
            }
            domains.insert(*id, domain.clone());
        }
        Ok(Self(domains))
    }

    fn admits(&self, value: &Term, domain: &Term, fuel: &mut usize) -> Result<bool, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        let Term::Rigid {
            declaration,
            sort: sort @ (Sort::ComptimeValue | Sort::Runtime),
        } = value
        else {
            return Ok(false);
        };
        let Some(expected) = self.0.get(declaration) else {
            return Ok(false);
        };
        nir::type_computations::charge_term(expected, fuel)?;
        nir::type_computations::charge_term(domain, fuel)?;
        Ok(expected == domain && Sort::of_comptime_domain(expected) == *sort)
    }
}

impl Builder<'_> {
    pub(super) fn normalized_interface_arguments(
        &mut self,
        declaration: Id,
        subject: &Term,
        arguments: &[Term],
    ) -> Result<Vec<Term>, String> {
        query_budget::run(&mut self.fuel, self.budget, |budget| {
            self.world.normalized_interface_arguments(
                declaration,
                subject,
                arguments,
                budget,
                &mut |value, domain, fuel| {
                    if let Term::Runtime(id) = value {
                        nir::type_computations::charge_term(domain, fuel)?;
                        return Ok(self
                            .package
                            .runtime_domains
                            .get(id)
                            .is_some_and(|declared| declared == domain));
                    }
                    self.value_scope.admits(value, domain, fuel)
                },
            )
        })
        .map_err(String::from)
    }
}
