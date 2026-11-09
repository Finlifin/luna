//! Passive description inspection separates structural limits from query work.
//! Registered heads stay opaque; scoped declaration inputs require explicit evidence.
use super::*;
use nir::NominalAdmissionError as AdmissionError;

impl Interpreter<'_> {
    pub(super) fn inspect_type_description(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        if ty.sort() != nir::Sort::Type {
            return Err(ErrorKind::Type);
        }
        inspect(
            ty,
            &self.normal_forms,
            &self.abstract_type_parameters,
            &self.abstract_value_parameters,
            |ty, variant, fuel| self.query_normal_value_fields(ty, variant, fuel),
            |value, frames, fuel| self.admitted_callable_signature(value, frames, fuel),
            self.target,
            fuel,
        )
    }

    pub(super) fn check_type_description(
        &mut self,
        ty: &Term,
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = self.inspect_type_description(ty, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|kind| self.failure(kind, source))
    }
}

fn spend(fuel: &mut usize, amount: usize) -> Result<(), ErrorKind> {
    *fuel = fuel.checked_sub(amount).ok_or(ErrorKind::StepsExhausted)?;
    Ok(())
}
fn computation_error(error: nir::type_computations::Error) -> ErrorKind {
    match error {
        nir::type_computations::Error::Budget => ErrorKind::StepsExhausted,
        _ => ErrorKind::Type,
    }
}
fn application_error(error: nir::normal_forms::ApplicationError) -> ErrorKind {
    match error {
        nir::normal_forms::ApplicationError::Budget { .. } => ErrorKind::StepsExhausted,
        nir::normal_forms::ApplicationError::Unresolved { .. } => ErrorKind::TypeContractUnresolved,
        _ => ErrorKind::Type,
    }
}

pub(super) fn inspect<'signature>(
    ty: &Term,
    heads: &impl declaration_index::HeadLookup,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
    nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    callable_signature: impl Fn(
        nir::CallableValueRef<'_>,
        &[Vec<nir::Sort>],
        &mut usize,
    )
        -> Result<Option<std::borrow::Cow<'signature, Term>>, AdmissionError>,
    target: Option<nir::target::TargetData>,
    fuel: &mut usize,
) -> Result<(), ErrorKind> {
    // A leaf needs neither a heap traversal stack nor an empty shared frame.
    // Allocate pending nodes only for children, and shared binder frames only
    // when entering a function telescope. Query work does not enlarge the
    // structural description-size counter.
    let mut pending = Vec::new();
    let mut first = Some((ty, 0usize, None::<std::rc::Rc<Vec<&[nir::Parameter]>>>));
    let mut work = 0usize;
    while let Some((ty, depth, frames)) = first.take().or_else(|| pending.pop()) {
        let frame_slice = frames.as_deref().map_or(&[][..], Vec::as_slice);
        spend(fuel, 1)?;
        work += 1;
        if depth > 128 {
            return Err(ErrorKind::DepthExhausted);
        }
        if work > 100_000 {
            return Err(ErrorKind::RepresentationSizeLimit);
        }
        // A concatenation is a pending list operation, not a completed
        // value identity. Even literal-only parts must first pass through the
        // canonical constructor/substitution that produces an immutable list value.
        if matches!(ty, Term::ListConcat { .. }) {
            return Err(ErrorKind::Type);
        }
        let children = match ty {
            Term::MethodValue(method) => 1usize
                .saturating_add(method.interface.arguments.len())
                .saturating_add(method.method_arguments.len()),
            Term::OptionalValue { value, .. } => 1 + usize::from(value.is_some()),
            Term::ErrorValue { .. } => 3,
            Term::ListConcat { parts: fields, .. } | Term::NominalValue { fields, .. } => {
                fields.len().saturating_add(1)
            }
            Term::Record(fields) | Term::RecordValue(fields) => fields.len(),
            Term::Tuple(items)
            | Term::ClosureContextValue(items)
            | Term::TupleValue(items)
            | Term::ClosureSnapshot {
                arguments: items, ..
            }
            | Term::FunctionValue {
                arguments: items, ..
            }
            | Term::Nominal {
                arguments: items, ..
            }
            | Term::NormalForm {
                arguments: items, ..
            } => items.len(),
            Term::ErrorResult { errors, .. } => errors.len().saturating_add(1),
            Term::Computation { effects, .. } => {
                work = work
                    .checked_add(effects.len())
                    .ok_or(ErrorKind::RepresentationSizeLimit)?;
                spend(fuel, effects.len())?;
                effects
                    .iter()
                    .try_fold(1usize, |sum, op| sum.checked_add(op.arguments.len()))
                    .ok_or(ErrorKind::RepresentationSizeLimit)?
            }
            Term::Closure { environment, .. } => environment.len().saturating_add(1),
            Term::ContextualClosure { .. } => 2,
            Term::Function { parameters, .. } => parameters.len().saturating_add(1),
            Term::Optional(_)
            | Term::Pointer(_)
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::UnsafeFunction(_) => 1,
            _ => 0,
        };
        if work.saturating_add(pending.len()).saturating_add(children) > 100_000 {
            return Err(ErrorKind::RepresentationSizeLimit);
        }
        if children > *fuel {
            return Err(ErrorKind::StepsExhausted);
        }
        let scalar_work = match ty {
            Term::Integer(integer) => integer.bit_len().div_ceil(32),
            Term::Float(value) => value
                .digits()
                .len()
                .saturating_add(value.exponent().bit_len().div_ceil(32)),
            Term::String(value) | Term::Symbol(value) => value.len(),
            Term::BitVec(value) => value.words().len(),
            Term::Record(fields) | Term::RecordValue(fields) => fields
                .iter()
                .try_fold(0usize, |sum, (name, _)| sum.checked_add(name.len()))
                .ok_or(ErrorKind::RepresentationSizeLimit)?,
            _ => 0,
        };
        work = work
            .checked_add(scalar_work)
            .ok_or(ErrorKind::RepresentationSizeLimit)?;
        if work > 100_000 {
            return Err(ErrorKind::RepresentationSizeLimit);
        }
        spend(fuel, scalar_work)?;
        if let Term::NormalForm {
            head,
            sort,
            arguments,
        } = ty
        {
            if let Some(metadata) = heads.head(head) {
                if *sort != nir::Sort::Type {
                    return Err(ErrorKind::Type);
                }
                let mut bound_domains = BTreeMap::new();
                for (frame_depth, frame) in frame_slice.iter().rev().enumerate() {
                    spend(fuel, frame.len().saturating_add(1))?;
                    for (index, parameter) in frame.iter().enumerate() {
                        if parameter.sort != nir::Sort::ComptimeValue {
                            continue;
                        }
                        nir::type_computations::charge_term(&parameter.ty, fuel)
                            .map_err(computation_error)?;
                        let mut domain = parameter.ty.clone();
                        for _ in 0..frame_depth {
                            nir::type_computations::charge_term(&domain, fuel)
                                .map_err(computation_error)?;
                            domain = domain
                                .abstract_parameters(&[])
                                .map_err(|_| ErrorKind::Type)?;
                        }
                        bound_domains.insert((frame_depth as u32, index), domain);
                    }
                }
                metadata
                    .check_arguments(
                        arguments,
                        &frame_slice
                            .iter()
                            .map(|frame| frame.iter().map(|p| p.sort).collect())
                            .collect::<Vec<_>>(),
                        fuel,
                        &mut nir::value_admission::DomainQueries {
                            layout: &mut |ty, variant, fuel| {
                                if variant.is_none() {
                                    if let Some(fields) =
                                        nir::normal_forms::registered_tuple_fields(ty, |id| {
                                            heads.head(&id)
                                        })
                                    {
                                        nir::type_computations::charge_term(ty, fuel)?;
                                        return Ok(fields.to_vec());
                                    }
                                }
                                nominal_fields(ty, variant, fuel)
                            },
                            declared: &mut |value, domain, _| {
                                Ok(match value {
                                    Term::Rigid {
                                        declaration,
                                        sort: nir::Sort::ComptimeValue,
                                    } => value_parameters.get(declaration) == Some(domain),
                                    Term::Bound {
                                        depth,
                                        index,
                                        sort: nir::Sort::ComptimeValue,
                                    } => bound_domains.get(&(*depth, *index)) == Some(domain),
                                    _ => false,
                                })
                            },
                            collection: &mut |domain, fuel| {
                                let Term::NormalForm { head, .. } = domain else {
                                    return Ok(None);
                                };
                                let Some(element) =
                                    heads.head(head).and_then(|h| h.list_element(domain))
                                else {
                                    return Ok(None);
                                };
                                nir::type_computations::charge_term(element, fuel)?;
                                Ok(Some(element.clone()))
                            },
                            callable: &mut |value, frames, fuel| {
                                callable_signature(value, frames, fuel)
                            },
                            target,
                        },
                    )
                    .map_err(application_error)?;
            }
        }
        match ty {
            Term::MethodValue(method) => {
                pending.extend(method.terms().map(|term| (term, depth + 1, frames.clone())));
            }
            Term::OptionalValue { ty, value } => {
                pending.push((ty, depth + 1, frames.clone()));
                if let Some(value) = value {
                    pending.push((value, depth + 1, frames.clone()));
                }
            }
            Term::ErrorValue { ty, label, value } => {
                pending.push((ty, depth + 1, frames.clone()));
                if let Some(label) = label {
                    pending.push((label, depth + 1, frames.clone()));
                }
                pending.push((value, depth + 1, frames.clone()));
            }
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
                pending.push((ty, depth + 1, frames.clone()));
                pending.extend(
                    fields
                        .iter()
                        .map(|field| (field, depth + 1, frames.clone())),
                );
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                pending.extend(fields.iter().map(|(_, ty)| (ty, depth + 1, frames.clone())));
            }
            Term::Rigid {
                declaration,
                sort: nir::Sort::Type,
            } if parameters.contains(declaration) => {}
            Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            } if value_parameters.contains_key(declaration) => {}
            Term::AssociatedType { .. } => return Err(ErrorKind::UnmaterializedDispatch),
            Term::Rigid { .. } | Term::Runtime(_) | Term::Value { .. } | Term::Hole { .. } => {
                return Err(ErrorKind::Type);
            }
            Term::Tuple(items)
            | Term::ClosureContextValue(items)
            | Term::TupleValue(items)
            | Term::ClosureSnapshot {
                arguments: items, ..
            }
            | Term::FunctionValue {
                arguments: items, ..
            }
            | Term::Nominal {
                arguments: items, ..
            }
            | Term::NormalForm {
                arguments: items, ..
            } => {
                pending.extend(items.iter().map(|t| (t, depth + 1, frames.clone())));
            }
            Term::ErrorResult { errors, result } => {
                pending.push((result, depth + 1, frames.clone()));
                pending.extend(errors.iter().map(|ty| (ty, depth + 1, frames.clone())));
            }
            Term::Computation { effects, result } => {
                pending.push((result, depth + 1, frames.clone()));
                for operation in effects {
                    pending.extend(
                        operation
                            .arguments
                            .iter()
                            .map(|t| (t, depth + 1, frames.clone())),
                    );
                }
            }
            Term::Closure {
                environment,
                signature,
            } => {
                pending.extend(environment.iter().map(|t| (t, depth + 1, frames.clone())));
                pending.push((signature, depth + 1, frames.clone()));
            }
            Term::ContextualClosure { context, signature } => {
                pending.push((context, depth + 1, frames.clone()));
                pending.push((signature, depth + 1, frames.clone()));
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::NormalFormFunction(inner, _)
            | Term::UnsafeFunction(inner) => pending.push((inner, depth + 1, frames.clone())),
            Term::Function { parameters, result } => {
                let frame_work = frame_slice.len().saturating_add(1);
                work = work
                    .checked_add(frame_work)
                    .ok_or(ErrorKind::RepresentationSizeLimit)?;
                if work > 100_000 {
                    return Err(ErrorKind::RepresentationSizeLimit);
                }
                spend(fuel, frame_work)?;
                let mut nested = frame_slice.to_vec();
                nested.push(parameters);
                let frames = Some(std::rc::Rc::new(nested));
                pending.extend(
                    parameters
                        .iter()
                        .map(|p| (&p.ty, depth + 1, frames.clone())),
                );
                pending.push((result, depth + 1, frames.clone()));
            }
            Term::Primitive(_)
            | Term::ClosureContextType
            | Term::Bound { .. }
            | Term::Unit
            | Term::Bool(_)
            | Term::Char(_)
            | Term::BitVec(_)
            | Term::Float16(_)
            | Term::BFloat16(_)
            | Term::Float32(_)
            | Term::Float64(_)
            | Term::String(_)
            | Term::Symbol(_)
            | Term::Integer(_)
            | Term::Float(_) => {}
        }
    }
    ty.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
    Ok(())
}

#[cfg(test)]
mod operation_tests;
#[cfg(test)]
mod tests;
