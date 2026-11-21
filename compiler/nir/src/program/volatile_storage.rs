use crate::newtypes::Newtype;
use crate::program::Struct;
use crate::types::ParameterSubstitution;
use crate::{DeclarationId, Term};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

/// Volatile memory / MMIO storage markers derived from checked `__flurry_volatile` attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolatileStorage {
    pub declarations: BTreeSet<DeclarationId>,
    pub fields: BTreeMap<DeclarationId, BTreeSet<usize>>,
}

pub enum VolatileNominal<'a> {
    Struct(&'a [(String, Term)]),
    Newtype(&'a Term),
}

impl VolatileStorage {
    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty() && self.fields.is_empty()
    }

    pub fn entry_count(&self) -> usize {
        self.declarations
            .len()
            .saturating_add(self.fields.len())
            .saturating_add(self.fields.values().map(BTreeSet::len).sum::<usize>())
    }

    pub fn is_declaration_volatile(&self, declaration: DeclarationId) -> bool {
        self.declarations.contains(&declaration)
    }

    pub fn is_field_volatile(&self, declaration: DeclarationId, index: usize) -> bool {
        self.is_declaration_volatile(declaration)
            || self
                .fields
                .get(&declaration)
                .is_some_and(|set| set.contains(&index))
    }

    pub fn type_is_volatile_in_nir(
        &self,
        structs: &[Struct],
        newtypes: &[Newtype],
        ty: &Term,
        fuel: &mut usize,
    ) -> bool {
        self.traverse_type(ty, fuel, |cur, pending| {
            let Term::Nominal {
                declaration,
                arguments,
            } = cur
            else {
                return true;
            };
            if let Some(def) = structs.iter().find(|s| s.declaration == *declaration) {
                return push_substituted_fields(
                    def.type_parameters.is_empty(),
                    || def.parameter_binders(),
                    arguments,
                    def.fields.iter().map(|(_, f)| f),
                    pending,
                );
            }
            if let Some(def) = newtypes.iter().find(|n| n.declaration == *declaration) {
                return push_substituted_fields(
                    def.type_parameters.is_empty(),
                    || def.parameter_binders(),
                    arguments,
                    std::iter::once(&def.representation),
                    pending,
                );
            }
            true
        })
    }

    pub fn type_is_volatile_with<'a>(
        &self,
        lookup_nominal: impl Fn(&Term) -> Option<VolatileNominal<'a>>,
        ty: &Term,
        fuel: &mut usize,
    ) -> bool {
        self.traverse_type(ty, fuel, |cur, pending| {
            match lookup_nominal(cur) {
                Some(VolatileNominal::Struct(fields)) => {
                    pending.extend(fields.iter().map(|(_, f)| f.clone()));
                }
                Some(VolatileNominal::Newtype(repr)) => {
                    pending.push(repr.clone());
                }
                None => {}
            }
            true
        })
    }

    fn traverse_type(
        &self,
        ty: &Term,
        fuel: &mut usize,
        mut expand_nominal: impl FnMut(&Term, &mut Vec<Term>) -> bool,
    ) -> bool {
        if self.is_empty() {
            return false;
        }
        let mut pending: Vec<Term> = vec![ty.clone()];
        let mut visited: HashSet<Term> = HashSet::new();
        while let Some(cur) = pending.pop() {
            let Some(next) = fuel.checked_sub(1) else {
                *fuel = 0;
                return true;
            };
            *fuel = next;
            if visited.len() >= 128 {
                return true;
            }
            if !visited.insert(cur.clone()) {
                continue;
            }
            match &cur {
                Term::Nominal { declaration, .. } => {
                    if self.is_declaration_volatile(*declaration)
                        || self
                            .fields
                            .get(declaration)
                            .is_some_and(|set| !set.is_empty())
                    {
                        return true;
                    }
                    if !expand_nominal(&cur, &mut pending) {
                        return true;
                    }
                }
                Term::Tuple(items) => pending.extend(items.iter().cloned()),
                Term::Record(fields) => pending.extend(fields.iter().map(|(_, f)| f.clone())),
                Term::Optional(inner) => pending.push((**inner).clone()),
                Term::ErrorResult { errors, result } => {
                    pending.extend(errors.iter().cloned());
                    pending.push((**result).clone());
                }
                _ => {}
            }
        }
        false
    }
}

fn push_substituted_fields<'a>(
    no_params: bool,
    binders: impl FnOnce() -> Result<Vec<crate::DeclarationParameter>, crate::TypeError>,
    arguments: &[Term],
    fields: impl Iterator<Item = &'a Term>,
    pending: &mut Vec<Term>,
) -> bool {
    if no_params || arguments.is_empty() {
        pending.extend(fields.cloned());
        return true;
    }
    let Ok(binders) = binders() else {
        return false;
    };
    let Ok(sub) = ParameterSubstitution::for_fragments(&binders, arguments) else {
        return false;
    };
    for f in fields {
        let Ok(inst) = sub.term(f) else {
            return false;
        };
        pending.push(inst);
    }
    true
}

impl fmt::Display for VolatileStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(volatile-storage (declarations")?;
        for decl in &self.declarations {
            write!(f, " {decl}")?;
        }
        f.write_str(") (fields")?;
        for (decl, indices) in &self.fields {
            write!(f, " (field-set {decl}")?;
            for index in indices {
                write!(f, " {index}")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}
