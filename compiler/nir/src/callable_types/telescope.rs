//! Form a callable frame from declaration-scoped domains. This is preparation,
//! not callable admission or a public supply interface. No source names survive
//! here: the caller must preserve its public interface separately.
use super::*;
use crate::DeclarationId;
use std::{collections::BTreeMap, fmt};

/// Each successful push adds one binder after abstracting its domain against
/// the completed prefix. Failed inputs publish no binder or parameter. The
/// caller retains responsibility for source scope and declaration formation.
#[derive(Default)]
pub struct DeclarationTelescope {
    indices: BTreeMap<DeclarationId, (usize, Sort)>,
    parameters: Vec<Parameter>,
}

impl DeclarationTelescope {
    pub fn push(
        &mut self,
        declaration: DeclarationId,
        sort: Sort,
        domain: &Term,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if self.indices.contains_key(&declaration) {
            return Err(Error::Scope(TypeError::DuplicateDeclarationParameter(
                declaration,
            )));
        }
        check_type(domain, fuel)?;
        let ty = domain
            .abstract_parameter_indices(&self.indices)
            .map_err(Error::Scope)?;
        self.indices
            .insert(declaration, (self.parameters.len(), sort));
        self.parameters.push(Parameter { sort, ty });
        Ok(())
    }

    /// Introduce the result into the same frame, moving completed parameters
    /// without copying earlier domains or rebuilding their declaration index.
    /// External frames remain external; the consumer checks the final scope.
    pub fn finish(self, result: &Term, fuel: &mut usize) -> Result<Term, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        check_type(result, fuel)?;
        let result = result
            .abstract_parameter_indices(&self.indices)
            .map_err(Error::Scope)?;
        Ok(Term::Function {
            parameters: self.parameters,
            result: Box::new(result),
        })
    }
}

impl fmt::Display for DeclarationTelescope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(declaration-telescope (parameters")?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (declarations")?;
        for (declaration, (index, sort)) in &self.indices {
            write!(f, " (binding {declaration} (slot {index}) (sort {sort}))")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 8, local }
    }
    fn bound(depth: u32, index: usize, sort: Sort) -> Term {
        Term::Bound { depth, index, sort }
    }
    fn rigid(local: u32, sort: Sort) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort,
        }
    }
    fn ty() -> Term {
        Term::Primitive(Primitive::Type)
    }

    #[test]
    fn mixed_declarations_and_nested_binders_preserve_outer_scope() {
        let mut telescope = DeclarationTelescope::default();
        let mut fuel = 10_000;
        telescope.push(id(1), Sort::Type, &ty(), &mut fuel).unwrap();
        telescope
            .push(id(2), Sort::ComptimeValue, &rigid(1, Sort::Type), &mut fuel)
            .unwrap();
        let indexed = Term::NormalForm {
            head: id(9),
            sort: Sort::Type,
            arguments: vec![rigid(1, Sort::Type), rigid(2, Sort::ComptimeValue)],
        };
        let nested = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: ty(),
            }],
            result: Box::new(Term::Record(vec![
                ("local".into(), bound(0, 0, Sort::Type)),
                ("outer".into(), bound(1, 0, Sort::Type)),
                ("declarations".into(), indexed),
            ])),
        };
        telescope
            .push(id(3), Sort::Runtime, &nested, &mut fuel)
            .unwrap();
        let signature = telescope.finish(&rigid(1, Sort::Type), &mut fuel).unwrap();
        signature.check_scope(&[vec![Sort::Type]]).unwrap();
        let Term::Function { parameters, result } = signature else {
            panic!()
        };
        assert_eq!(*result, bound(0, 0, Sort::Type));
        assert_eq!(parameters[1].ty, bound(0, 0, Sort::Type));
        let Term::Function { result, .. } = &parameters[2].ty else {
            panic!()
        };
        assert_eq!(
            **result,
            Term::Record(vec![
                ("local".into(), bound(0, 0, Sort::Type)),
                ("outer".into(), bound(2, 0, Sort::Type)),
                (
                    "declarations".into(),
                    Term::NormalForm {
                        head: id(9),
                        sort: Sort::Type,
                        arguments: vec![bound(1, 0, Sort::Type), bound(1, 1, Sort::ComptimeValue)],
                    }
                ),
            ])
        );
    }

    #[test]
    fn failed_push_preserves_prefix_and_retry_binds_the_same_slot() {
        let mut telescope = DeclarationTelescope::default();
        telescope.push(id(1), Sort::Type, &ty(), &mut 100).unwrap();
        let before = telescope.to_string();
        assert!(before.starts_with("(declaration-telescope (parameters"));
        let deep = (0..129).fold(ty(), |value, _| Term::Optional(Box::new(value)));
        for (declaration, sort, domain, mut fuel) in [
            (id(2), Sort::Runtime, ty(), 0),
            (id(2), Sort::Runtime, deep, 10_000),
            (id(1), Sort::Type, ty(), 100),
            (id(2), Sort::Runtime, Term::Bool(true), 100),
            (
                id(2),
                Sort::Runtime,
                Term::NormalForm {
                    head: id(9),
                    sort: Sort::Type,
                    arguments: vec![rigid(1, Sort::ComptimeValue)],
                },
                100,
            ),
        ] {
            assert!(
                telescope
                    .push(declaration, sort, &domain, &mut fuel)
                    .is_err()
            );
            assert_eq!(telescope.to_string(), before);
        }
        telescope
            .push(id(2), Sort::Runtime, &rigid(1, Sort::Type), &mut 100)
            .unwrap();
        let signature = telescope.finish(&rigid(1, Sort::Type), &mut 100).unwrap();
        assert_eq!(
            signature,
            Term::Function {
                parameters: vec![
                    Parameter {
                        sort: Sort::Type,
                        ty: ty()
                    },
                    Parameter {
                        sort: Sort::Runtime,
                        ty: bound(0, 0, Sort::Type),
                    }
                ],
                result: Box::new(bound(0, 0, Sort::Type)),
            }
        );
    }

    #[test]
    fn wide_telescope_uses_a_shared_prefix_index_and_linear_allowance() {
        let mut telescope = DeclarationTelescope::default();
        let mut fuel = 4_100;
        telescope.push(id(0), Sort::Type, &ty(), &mut fuel).unwrap();
        for slot in 1..2_000 {
            telescope
                .push(id(slot), Sort::Runtime, &rigid(0, Sort::Type), &mut fuel)
                .unwrap();
        }
        let signature = telescope.finish(&rigid(0, Sort::Type), &mut fuel).unwrap();
        signature.check_scope(&[]).unwrap();
        assert!(fuel > 0);
    }
}
