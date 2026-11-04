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
            sort: Sort::ComptimeValue,
        } = value
        else {
            return Ok(false);
        };
        let Some(expected) = self.0.get(declaration) else {
            return Ok(false);
        };
        nir::type_computations::charge_term(expected, fuel)?;
        nir::type_computations::charge_term(domain, fuel)?;
        Ok(expected == domain)
    }
}

impl Builder<'_> {
    pub(super) fn normalized_interface_arguments(
        &mut self,
        declaration: Id,
        subject: &Term,
        arguments: &[Term],
    ) -> Result<Vec<Term>, String> {
        self.tick(0)?;
        let contract = self
            .package
            .traits
            .iter()
            .find(|contract| contract.declaration == declaration)
            .ok_or("missing dispatch interface input contract")?;
        self.fuel = self
            .fuel
            .checked_sub(arguments.len().saturating_add(1))
            .ok_or("interface input admission budget exhausted")?;
        let mut inputs = Vec::with_capacity(arguments.len().saturating_add(1));
        inputs.push(self.normalized_type(subject)?);
        for argument in arguments {
            inputs.push(if argument.sort() == Sort::Type {
                self.normalized_type(argument)?
            } else {
                nir::type_computations::charge_term(argument, &mut self.fuel)
                    .map_err(|e| e.to_string())?;
                argument.clone()
            });
        }
        let substitution = contract
            .input_substitution(&inputs, &[], &mut self.fuel)
            .map_err(|e| e.to_string())?;
        for (index, argument) in inputs.iter().skip(1).enumerate() {
            let domain = contract
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let domain = substitution
                .instantiate_bounded(domain, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            let domain = self.normalized_type(&domain)?;
            if domain == Term::Primitive(Primitive::Type) {
                // input_substitution checked the entire telescope's sorts and
                // scopes; normalized_type handled this Type argument above.
                continue;
            }
            let scope = &self.value_scope;
            self.world
                .with_admission_queries(|queries| {
                    nir::value_admission::validate(
                        argument,
                        &domain,
                        &[],
                        &mut self.fuel,
                        &mut nir::value_admission::DomainQueries {
                            layout: queries.layout,
                            declared: &mut |value, domain, fuel| scope.admits(value, domain, fuel),
                            collection: queries.collection,
                            callable: queries.callable,
                            target: queries.target,
                        },
                    )
                })
                .map_err(|e| format!("interface value input admission failed: {e}"))?;
        }
        Ok(inputs.into_iter().skip(1).collect())
    }
}
