//! Runtime payload admission in a caller's sort environment. Only nominal
//! expansion can introduce cycles. Ordinary structural children stay borrowed.
use super::{NominalStorageResolver, Package, Sort, StorageError, Term, storage_domains::Domains};
use crate::Primitive;
use std::collections::HashSet;

pub(super) fn validate(
    ty: &Term,
    package: &Package,
    frames: &[Vec<Sort>],
    resolve: NominalStorageResolver<'_>,
    domains: &Domains<'_>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    crate::type_computations::charge_term(ty, fuel)?;
    if ty.sort() != Sort::Type || ty.check_scope(frames).is_err() {
        return Err(StorageError::Invalid(
            "runtime storage requires a Type in the declared input scope",
        ));
    }
    let mut query = Payloads {
        package,
        resolve,
        domains,
        seen: HashSet::new(),
    };
    query.check(ty, frames, 0, fuel)
}

struct Payloads<'a> {
    package: &'a Package,
    resolve: NominalStorageResolver<'a>,
    domains: &'a Domains<'a>,
    seen: HashSet<(Term, Vec<Vec<Sort>>)>,
}

fn charge(fuel: &mut usize, count: usize) -> Result<(), StorageError> {
    *fuel = fuel.checked_sub(count).ok_or(StorageError::Budget)?;
    Ok(())
}

const COMPTIME_ONLY: StorageError =
    StorageError::Invalid("comptime-only value cannot inhabit runtime storage; Splice is required");

impl Payloads<'_> {
    fn check(
        &mut self,
        ty: &Term,
        frames: &[Vec<Sort>],
        depth: usize,
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        charge(fuel, 1)?;
        if depth > 128 {
            return Err(StorageError::Budget);
        }
        let next = depth + 1;
        match ty {
            Term::ClosureContextType
            | Term::ClosureContextValue(_)
            | Term::Primitive(
                Primitive::Integer | Primitive::Float | Primitive::Type | Primitive::Object,
            )
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::Lift(_) => Err(COMPTIME_ONLY),
            Term::ContextualClosure { context, signature } => {
                let scoped = match context.as_ref() {
                    Term::Bound {
                        sort: Sort::ComptimeValue,
                        ..
                    } => context.check_scope(frames).is_ok(),
                    Term::Rigid {
                        declaration,
                        sort: Sort::ComptimeValue,
                    } => self.domains.get(declaration, fuel)? == Some(&Term::ClosureContextType),
                    _ => false,
                };
                if !scoped {
                    return Err(StorageError::Invalid(
                        "closure storage context lacks scoped evidence",
                    ));
                }
                self.check(signature, frames, next, fuel)
            }
            Term::NormalForm { head, .. } => {
                let Some(metadata) = self
                    .package
                    .normal_forms
                    .iter()
                    .find(|h| h.declaration == *head)
                else {
                    return Ok(());
                };
                if metadata.role == crate::normal_forms::Role::TypeList {
                    return Err(COMPTIME_ONLY);
                }
                if let Some(fields) = metadata.tuple_elements(ty) {
                    self.fields(fields, frames, next, fuel)?;
                }
                Ok(())
            }
            Term::ErrorResult { errors, result } => {
                self.fields(errors, frames, next, fuel)?;
                self.check(result, frames, next, fuel)
            }
            Term::Computation { result, .. }
            | Term::Optional(result)
            | Term::Pointer(result)
            | Term::UnsafeFunction(result) => self.check(result, frames, next, fuel),
            Term::Tuple(items) => self.fields(items, frames, next, fuel),
            Term::Record(fields) => {
                self.fields(fields.iter().map(|(_, ty)| ty), frames, next, fuel)
            }
            Term::Function { parameters, result } => {
                charge(fuel, parameters.len())?;
                let mut nested = copy_frames(frames, fuel)?;
                nested.push(parameters.iter().map(|p| p.sort).collect());
                self.fields(
                    parameters
                        .iter()
                        .filter(|p| p.sort == Sort::Runtime)
                        .map(|p| &p.ty),
                    &nested,
                    next,
                    fuel,
                )?;
                self.check(result, &nested, next, fuel)
            }
            Term::Closure {
                environment,
                signature,
            } => {
                self.fields(environment, frames, next, fuel)?;
                self.check(signature, frames, next, fuel)
            }
            Term::Nominal {
                declaration,
                arguments,
            } => {
                // A closed recursive callback may reach the same nominal under additional
                // function frames. An open instance retains its caller's binder sorts.
                crate::type_computations::charge_term(ty, fuel)?;
                let scope = if ty.check_scope(&[]).is_ok() {
                    Vec::new()
                } else {
                    copy_frames(frames, fuel)?
                };
                if !self.seen.insert((ty.clone(), scope)) {
                    return Ok(());
                }
                let fields = if let Some(resolve) = self.resolve {
                    let fields = resolve(ty, fuel)?;
                    for field in &fields {
                        crate::type_computations::charge_term(field, fuel)?;
                    }
                    fields
                } else if let Some(def) = self
                    .package
                    .structs
                    .iter()
                    .find(|d| d.declaration == *declaration)
                {
                    def.instantiate_stored_fields(arguments, frames, fuel)?
                } else if let Some(def) = self
                    .package
                    .enums
                    .iter()
                    .find(|d| d.declaration == *declaration)
                {
                    def.instantiate_stored_fields(arguments, frames, fuel)?
                } else if let Some(def) = self
                    .package
                    .newtypes
                    .iter()
                    .find(|d| d.declaration == *declaration)
                {
                    return self.check(
                        &def.representation_budgeted(arguments, frames, fuel)?,
                        frames,
                        next,
                        fuel,
                    );
                } else {
                    return Ok(());
                };
                self.fields(&fields, frames, next, fuel)
            }
            _ => Ok(()),
        }
    }

    fn fields<'t>(
        &mut self,
        fields: impl IntoIterator<Item = &'t Term>,
        frames: &[Vec<Sort>],
        depth: usize,
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        for field in fields {
            self.check(field, frames, depth, fuel)?;
        }
        Ok(())
    }
}

fn copy_frames(frames: &[Vec<Sort>], fuel: &mut usize) -> Result<Vec<Vec<Sort>>, StorageError> {
    charge(fuel, frames.len())?;
    for frame in frames {
        charge(fuel, frame.len())?;
    }
    Ok(frames.to_vec())
}
