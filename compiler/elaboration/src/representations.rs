//! Finite storage checks over completed declaration descriptions. Normal-form
//! heads stay opaque except for the configured language Tuple constructor.
use nir::{DeclarationId, Sort, Term, program as n};
use std::collections::{BTreeMap, HashSet};

/// The borrowed world cannot change while successful roots are memoized.
/// Failed roots leave no active traversal behind and can be checked again.
pub struct Checker<'a> {
    tuple: Option<DeclarationId>,
    structs: &'a BTreeMap<DeclarationId, n::Struct>,
    enums: &'a BTreeMap<DeclarationId, n::Enum>,
    newtypes: &'a BTreeMap<DeclarationId, nir::newtypes::Newtype>,
    done: HashSet<Term>,
}
impl<'a> Checker<'a> {
    pub fn new(
        tuple: Option<DeclarationId>,
        structs: &'a BTreeMap<DeclarationId, n::Struct>,
        enums: &'a BTreeMap<DeclarationId, n::Enum>,
        newtypes: &'a BTreeMap<DeclarationId, nir::newtypes::Newtype>,
    ) -> Self {
        Self {
            tuple,
            structs,
            enums,
            newtypes,
            done: HashSet::new(),
        }
    }

    pub fn check(&mut self, ty: &Term, fuel: &mut usize) -> Result<(), &'static str> {
        self.visit(ty, &mut HashSet::new(), fuel, 0)
    }

    fn visit(
        &mut self,
        ty: &Term,
        active: &mut HashSet<Term>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<(), &'static str> {
        const BUDGET: &str = "struct representation expansion budget exhausted";
        *fuel = fuel.checked_sub(1).ok_or(BUDGET)?;
        if depth >= 128 {
            return Err(BUDGET);
        }
        // Account for hashing/cloning a growing generic application before it
        // reaches the active/done tables. This is the caller's shared budget.
        nir::type_computations::charge_term(ty, fuel).map_err(|_| BUDGET)?;
        match ty {
            Term::Nominal {
                declaration,
                arguments,
            } if !self.done.contains(ty) => {
                if !active.insert(ty.clone()) {
                    return Err("recursive by-value struct representation or enum payload");
                }
                if let Some(def) = self.structs.get(declaration) {
                    let parameters = def
                        .parameter_binders()
                        .map_err(|_| "invalid struct representation parameters")?;
                    for (_, field) in &def.fields {
                        nir::copy_budget::charge_type_substitution(
                            field,
                            &parameters,
                            arguments,
                            fuel,
                        )
                        .map_err(|_| BUDGET)?;
                    }
                    let fields = def
                        .instantiate_fields(arguments)
                        .map_err(|_| "invalid struct type arguments in representation")?;
                    for (_, field) in fields {
                        self.visit(&field, active, fuel, depth + 1)?;
                    }
                } else if let Some(def) = self.enums.get(declaration) {
                    let parameters = def
                        .parameter_binders()
                        .map_err(|_| "invalid enum representation parameters")?;
                    for field in def.variants.iter().flat_map(|v| &v.fields) {
                        nir::copy_budget::charge_type_substitution(
                            field,
                            &parameters,
                            arguments,
                            fuel,
                        )
                        .map_err(|_| BUDGET)?;
                    }
                    let variants = def
                        .instantiate_variants(arguments)
                        .map_err(|_| "invalid enum type arguments in representation")?;
                    for field in variants.iter().flat_map(|v| &v.fields) {
                        self.visit(field, active, fuel, depth + 1)?;
                    }
                } else if let Some(def) = self.newtypes.get(declaration) {
                    let representation = def
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|_| "invalid or excessively large newtype representation")?;
                    self.visit(&representation, active, fuel, depth + 1)?;
                } else {
                    return Err("nominal declaration is missing from representation world");
                }
                active.remove(ty);
                self.done.insert(ty.clone());
                Ok(())
            }
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments,
            } if Some(*head) == self.tuple => {
                if let [Term::TypeListValue(items)] = arguments.as_slice() {
                    for item in items {
                        self.visit(item, active, fuel, depth + 1)?;
                    }
                }
                Ok(())
            }
            Term::Tuple(items)
            | Term::Closure {
                environment: items, ..
            } => {
                for item in items {
                    self.visit(item, active, fuel, depth + 1)?;
                }
                Ok(())
            }
            Term::Record(fields) => {
                for (_, field) in fields {
                    self.visit(field, active, fuel, depth + 1)?;
                }
                Ok(())
            }
            Term::Optional(inner) => self.visit(inner, active, fuel, depth + 1),
            Term::ErrorResult { errors, result } => {
                self.visit(result, active, fuel, depth + 1)?;
                for label in errors {
                    self.visit(label, active, fuel, depth + 1)?;
                }
                Ok(())
            }
            // Pointer and function signatures do not embed their referents or
            // parameter/result values. Other normal-form heads are not unfolded.
            _ => Ok(()),
        }
    }
}
