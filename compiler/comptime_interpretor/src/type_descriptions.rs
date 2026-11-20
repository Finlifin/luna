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
        self.inspect_term_description(ty, fuel)
    }

    pub(super) fn inspect_term_description(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        self.inspect_scoped_term(term, &[], fuel)
    }

    pub(super) fn inspect_scoped_term(
        &self,
        term: &Term,
        enclosing: &[&[nir::Parameter]],
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        inspect(
            term,
            enclosing,
            &self.normal_forms,
            &self.abstract_type_parameters,
            &self.abstract_value_parameters,
            |ty, variant, fuel| self.query_normal_value_fields(ty, variant, fuel),
            |value, frames, fuel| self.admitted_callable_signature(value, frames, fuel),
            self.target,
            fuel,
        )
        .map(|_| ())
    }

    pub(super) fn inspect_closed_term_description(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        inspect(
            term,
            &[],
            &self.normal_forms,
            &BTreeSet::new(),
            &BTreeMap::new(),
            |ty, variant, fuel| self.query_normal_value_fields(ty, variant, fuel),
            |value, frames, fuel| self.admitted_callable_signature(value, frames, fuel),
            self.target,
            fuel,
        )
        .map(|_| ())
    }

    pub(super) fn inspect_closed_type_description(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        if ty.sort() != nir::Sort::Type {
            return Err(ErrorKind::Type);
        }
        self.inspect_closed_term_description(ty, fuel)
    }

    /// Borrow the caller's allowance for one or more passive descriptions.
    /// This never publishes evidence or authorizes execution or resource copying.
    pub(super) fn query_descriptions<T>(
        &mut self,
        source: Option<SourceRange>,
        query: impl FnOnce(&Self, &mut usize) -> Result<T, ErrorKind>,
    ) -> Result<T, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = query(self, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|kind| self.failure(kind, source))
    }

    pub(super) fn check_type_description(
        &mut self,
        ty: &Term,
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        self.query_descriptions(source, |this, fuel| this.inspect_type_description(ty, fuel))
    }

    pub(super) fn check_closed_type_description(
        &mut self,
        ty: &Term,
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        self.query_descriptions(source, |this, fuel| {
            this.inspect_closed_type_description(ty, fuel)
        })
    }

    pub(super) fn check_closed_term_description(
        &mut self,
        term: &Term,
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        self.query_descriptions(source, |this, fuel| {
            this.inspect_closed_term_description(term, fuel)
        })
    }
}

pub(super) fn spend(fuel: &mut usize, amount: usize) -> Result<(), ErrorKind> {
    *fuel = fuel.checked_sub(amount).ok_or(ErrorKind::StepsExhausted)?;
    Ok(())
}
pub(super) fn application_error(error: nir::normal_forms::ApplicationError) -> ErrorKind {
    match error {
        nir::normal_forms::ApplicationError::Budget { .. } => ErrorKind::StepsExhausted,
        nir::normal_forms::ApplicationError::Unresolved { .. } => ErrorKind::TypeContractUnresolved,
        _ => ErrorKind::Type,
    }
}

/// Each frame exposes only parameters whose own domains have been checked.
/// The sort view feeds NIR substitution; the typed view supplies domain evidence.
#[derive(Default)]
pub(super) struct Telescope<'t> {
    pub(super) parameters: Vec<&'t [nir::Parameter]>,
    pub(super) sorts: Vec<Vec<nir::Sort>>,
}

impl<'t> Telescope<'t> {
    pub(super) fn enter(&mut self) {
        self.parameters.push(&[]);
        self.sorts.push(Vec::new());
    }

    pub(super) fn advance(&mut self, parameters: &'t [nir::Parameter], index: usize) {
        *self.parameters.last_mut().unwrap() = &parameters[..=index];
        self.sorts.last_mut().unwrap().push(parameters[index].sort);
    }

    pub(super) fn leave(&mut self) {
        self.parameters.pop();
        self.sorts.pop();
    }

    fn enclosing(frames: &[&'t [nir::Parameter]], fuel: &mut usize) -> Result<Self, ErrorKind> {
        let mut scope = Self::default();
        for frame in frames {
            spend(fuel, frame.len() + 1)?;
            scope.enter();
            for index in 0..frame.len() {
                scope.advance(frame, index);
            }
        }
        Ok(scope)
    }
}

/// Resolve just the requested value binder. A parameter domain is relative to
/// its declaration frame, so lift its outer references to the observation site.
pub(super) fn declared_domain_matches(
    frames: &[&[nir::Parameter]],
    values: &BTreeMap<DeclarationId, Term>,
    value: &Term,
    domain: &Term,
    fuel: &mut usize,
) -> Result<bool, AdmissionError> {
    *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
    let (expected, distance) = match value {
        Term::Rigid {
            declaration,
            sort: nir::Sort::ComptimeValue,
        } => {
            let Some(expected) = values.get(declaration) else {
                return Ok(false);
            };
            (expected, 0)
        }
        Term::Bound {
            depth,
            index,
            sort: nir::Sort::ComptimeValue,
        } => {
            let parameter = frames
                .iter()
                .rev()
                .nth(*depth as usize)
                .and_then(|frame| frame.get(*index));
            let Some(parameter) = parameter.filter(|p| p.sort == nir::Sort::ComptimeValue) else {
                return Ok(false);
            };
            (&parameter.ty, *depth)
        }
        _ => return Ok(false),
    };
    nir::type_computations::charge_term(expected, fuel)?;
    nir::type_computations::charge_term(domain, fuel)?;
    if distance == 0 {
        Ok(expected == domain)
    } else {
        Ok(expected
            .under_binders(distance)
            .map_err(|error| error.to_string())?
            == *domain)
    }
}

/// Inspect a structural term without resolving declarations or running code.
pub(super) fn inspect_data_term(
    term: &Term,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
    fuel: &mut usize,
) -> Result<usize, ErrorKind> {
    inspect(
        term,
        &[],
        &BTreeMap::new(),
        parameters,
        value_parameters,
        |_, _, _| Err("nominal layout unavailable during structural inspection".into()),
        |_, _, _| Ok(None),
        None,
        fuel,
    )
}

pub(super) fn inspect<'signature>(
    ty: &Term,
    enclosing: &[&[nir::Parameter]],
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
) -> Result<usize, ErrorKind> {
    enum Work<'t> {
        Visit(&'t Term, usize),
        Parameter {
            parameters: &'t [nir::Parameter],
            next: usize,
            result: &'t Term,
            depth: usize,
        },
        Leave,
    }
    let mut scope = Telescope::enclosing(enclosing, fuel)?;
    let mut pending = Vec::new();
    let mut first = Some(Work::Visit(ty, 0));
    let mut work = 0usize;
    while let Some(action) = first.take().or_else(|| pending.pop()) {
        let (ty, depth) = match action {
            Work::Visit(term, depth) => (term, depth),
            Work::Leave => {
                scope.leave();
                continue;
            }
            Work::Parameter {
                parameters,
                next,
                result,
                depth,
            } => {
                if next > 0 {
                    scope.advance(parameters, next - 1);
                }
                if let Some(parameter) = parameters.get(next) {
                    pending.push(Work::Parameter {
                        parameters,
                        next: next + 1,
                        result,
                        depth,
                    });
                    (&parameter.ty, depth)
                } else {
                    pending.push(Work::Leave);
                    (result, depth)
                }
            }
        };
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
            | Term::Lift(_)
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
                metadata
                    .check_arguments(
                        arguments,
                        &scope.sorts,
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
                            declared: &mut |value, domain, fuel| {
                                declared_domain_matches(
                                    &scope.parameters,
                                    value_parameters,
                                    value,
                                    domain,
                                    fuel,
                                )
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
                pending.extend(method.terms().map(|term| Work::Visit(term, depth + 1)));
            }
            Term::OptionalValue { ty, value } => {
                pending.push(Work::Visit(ty, depth + 1));
                if let Some(value) = value {
                    pending.push(Work::Visit(value, depth + 1));
                }
            }
            Term::ErrorValue { ty, label, value } => {
                pending.push(Work::Visit(ty, depth + 1));
                if let Some(label) = label {
                    pending.push(Work::Visit(label, depth + 1));
                }
                pending.push(Work::Visit(value, depth + 1));
            }
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
                pending.push(Work::Visit(ty, depth + 1));
                pending.extend(fields.iter().map(|field| Work::Visit(field, depth + 1)));
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                pending.extend(fields.iter().map(|(_, ty)| Work::Visit(ty, depth + 1)));
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
                pending.extend(items.iter().map(|t| Work::Visit(t, depth + 1)));
            }
            Term::ErrorResult { errors, result } => {
                pending.push(Work::Visit(result, depth + 1));
                pending.extend(errors.iter().map(|ty| Work::Visit(ty, depth + 1)));
            }
            Term::Computation { effects, result } => {
                pending.push(Work::Visit(result, depth + 1));
                for operation in effects {
                    pending.extend(
                        operation
                            .arguments
                            .iter()
                            .map(|t| Work::Visit(t, depth + 1)),
                    );
                }
            }
            Term::Closure {
                environment,
                signature,
            } => {
                pending.extend(environment.iter().map(|t| Work::Visit(t, depth + 1)));
                pending.push(Work::Visit(signature, depth + 1));
            }
            Term::ContextualClosure { context, signature } => {
                pending.push(Work::Visit(context, depth + 1));
                pending.push(Work::Visit(signature, depth + 1));
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::Lift(inner)
            | Term::ComptimeFunction(inner)
            | Term::NormalFormFunction(inner, _)
            | Term::UnsafeFunction(inner) => pending.push(Work::Visit(inner, depth + 1)),
            Term::Function { parameters, result } => {
                scope.enter();
                pending.push(Work::Parameter {
                    parameters,
                    next: 0,
                    result,
                    depth: depth + 1,
                });
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
    ty.check_scope(&scope.sorts).map_err(|_| ErrorKind::Type)?;
    Ok(work)
}

#[cfg(test)]
mod operation_tests;
#[cfg(test)]
mod tests;
