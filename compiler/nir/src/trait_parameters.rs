//! Declaration domains for trait indices. Identity and domain formation do not
//! establish an implementation, a runtime environment, or a program contract.
use crate::{DeclarationParameter, Primitive, Sort, Term, program::Trait};
use std::collections::BTreeSet;

impl Term {
    /// Structural index values have scalar or Type leaves. Only Type leaves
    /// participate in type equality; the enclosing record is a program value.
    pub fn is_structural_index_value(&self) -> bool {
        let mut pending = vec![(self, 0usize)];
        let mut remaining = 100_000usize;
        while let Some((value, depth)) = pending.pop() {
            if remaining == 0 || depth >= 128 {
                return false;
            }
            remaining -= 1;
            if value.scalar_index_domain().is_some() || (depth > 0 && value.sort() == Sort::Type) {
                continue;
            }
            let Self::RecordValue(fields) = value else {
                return false;
            };
            if fields.len() > remaining.saturating_sub(pending.len()) {
                return false;
            }
            pending.extend(fields.iter().map(|(_, value)| (value, depth + 1)));
        }
        true
    }

    /// Compare the value skeleton and extract only its Type-valued relations.
    /// Inputs must have passed scope and domain validation. This does not prove
    /// the returned type equalities, and never unfolds a normal-form head.
    pub fn index_type_relations<'a>(
        &'a self,
        other: &'a Self,
    ) -> Option<Vec<(&'a Term, &'a Term)>> {
        let mut pending = vec![(self, other, 0usize)];
        let mut relations = Vec::new();
        let mut remaining = 100_000usize;
        while let Some((left, right, depth)) = pending.pop() {
            if remaining == 0 || depth >= 128 {
                return None;
            }
            remaining -= 1;
            if depth > 0 && left.sort() == Sort::Type && right.sort() == Sort::Type {
                relations.push((left, right));
            } else if let (Self::RecordValue(a), Self::RecordValue(b)) = (left, right) {
                if a.len() != b.len() || a.len() > remaining.saturating_sub(pending.len()) {
                    return None;
                }
                for ((name, a), (other_name, b)) in a.iter().zip(b).rev() {
                    if name != other_name {
                        return None;
                    }
                    pending.push((a, b, depth + 1));
                }
            } else if left.scalar_index_domain().is_none() || left != right {
                return None;
            }
        }
        Some(relations)
    }

    /// Intrinsic domain evidence for closed scalar interface indices. Equality
    /// of these indices is value identity, never a program equation sent to PTC.
    pub fn scalar_index_domain(&self) -> Option<Primitive> {
        Some(match self {
            Self::Symbol(_) => Primitive::Symbol,
            Self::String(_) => Primitive::Str,
            Self::Char(_) => Primitive::Char,
            Self::Bool(_) => Primitive::Bool,
            Self::Integer(_) => Primitive::Integer,
            Self::Float(_) => Primitive::Float,
            Self::Float32(_) => Primitive::F32,
            Self::Float64(_) => Primitive::F64,
            Self::BitVec(value) => Primitive::BitVec(value.width()),
            Self::Unit => Primitive::Void,
            _ => return None,
        })
    }
}

impl Trait {
    pub fn parameter_binders(&self, fuel: &mut usize) -> Result<Vec<DeclarationParameter>, String> {
        *fuel = fuel
            .checked_sub(self.type_parameters.len().saturating_add(1))
            .ok_or("trait parameter budget exhausted")?;
        if !self.parameter_types.is_empty()
            && self.parameter_types.len() != self.type_parameters.len()
        {
            return Err("trait parameter domain count mismatch".into());
        }
        let mut seen = BTreeSet::from([self.self_parameter]);
        let mut result = Vec::new();
        for (index, declaration) in self.type_parameters.iter().enumerate() {
            if !seen.insert(*declaration) {
                return Err("duplicate trait parameter identity".into());
            }
            let domain = self
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            crate::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
            if domain.sort() != Sort::Type {
                return Err("trait parameter domain must be a Type".into());
            }
            result.push(DeclarationParameter {
                declaration: *declaration,
                sort: if *domain == Term::Primitive(Primitive::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                },
            });
        }
        Ok(result)
    }

    pub fn binders_with_self(&self, fuel: &mut usize) -> Result<Vec<DeclarationParameter>, String> {
        let parameters = self.parameter_binders(fuel)?;
        Ok(std::iter::once(DeclarationParameter {
            declaration: self.self_parameter,
            sort: Sort::Type,
        })
        .chain(parameters)
        .collect())
    }

    /// Validate completed input domains in order. An untyped store handle or
    /// symbolic value is not guessed to inhabit the requested source type.
    pub fn validate_arguments(
        &self,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), String> {
        self.validate_arguments_in_declarations(arguments, scope, &[], fuel)
    }

    /// Open trait definitions may supply rigid value indices. Their domains must
    /// come from a unique declaration, never from the demanded interface slot.
    pub fn validate_arguments_in_declarations(
        &self,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        fuel: &mut usize,
    ) -> Result<(), String> {
        self.validate_argument_domains(None, arguments, scope, declarations, fuel)
    }

    /// Validate an interface instance with its actual Self type. The subject is
    /// type evidence supplied by the caller, not inferred from a value index.
    pub fn validate_arguments_for_subject(
        &self,
        subject: &Term,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        fuel: &mut usize,
    ) -> Result<(), String> {
        crate::type_computations::charge_term(subject, fuel).map_err(|e| e.to_string())?;
        subject.check_scope(scope).map_err(|e| e.to_string())?;
        if subject.sort() != Sort::Type {
            return Err("trait subject must be a Type".into());
        }
        self.validate_argument_domains(Some(subject), arguments, scope, declarations, fuel)
    }

    fn validate_argument_domains(
        &self,
        subject: Option<&Term>,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        fuel: &mut usize,
    ) -> Result<(), String> {
        let binders = self.parameter_binders(fuel)?;
        if arguments.len() != binders.len() {
            return Err("trait interface argument count mismatch".into());
        }
        for (index, argument) in arguments.iter().enumerate() {
            crate::type_computations::charge_term(argument, fuel).map_err(|e| e.to_string())?;
            argument.check_scope(scope).map_err(|e| e.to_string())?;
            if argument.sort() != binders[index].sort {
                return Err("trait interface argument sort mismatch".into());
            }
            if binders[index].sort == Sort::Type {
                continue;
            }
            let domain = crate::type_computations::instantiate_parameters_bounded(
                &self.parameter_types[index],
                &binders[..index],
                &arguments[..index],
                scope,
                fuel,
            )
            .map_err(|e| e.to_string())?;
            let domain = if let Some(subject) = subject {
                crate::type_computations::instantiate_parameters_bounded(
                    &domain,
                    &[DeclarationParameter {
                        declaration: self.self_parameter,
                        sort: Sort::Type,
                    }],
                    std::slice::from_ref(subject),
                    scope,
                    fuel,
                )
                .map_err(|e| e.to_string())?
            } else {
                domain
            };
            if matches!(argument, Term::RecordValue(_)) && argument.is_structural_index_value() {
                validate_record_index(argument, &domain, fuel)?;
                continue;
            }
            // Fixed-width proxies freeze to the same exact integer payload as
            // Integer. The declared domain supplies width and signedness; the
            // payload must fit exactly, without truncation or host-size guesses.
            if let (Term::Integer(value), Term::Primitive(primitive)) = (argument, &domain) {
                if let Some((signed, width)) = primitive.fixed_integer_layout() {
                    let fits = if signed {
                        value.to_signed(width).is_ok()
                    } else {
                        value.to_unsigned(width).is_ok()
                    };
                    if !fits {
                        return Err("trait integer argument is outside its declared domain".into());
                    }
                    continue;
                }
            }
            let actual = match argument {
                value if value.scalar_index_domain().is_some() => {
                    Term::Primitive(value.scalar_index_domain().unwrap())
                }
                Term::Rigid {
                    declaration,
                    sort: Sort::ComptimeValue,
                } => {
                    let mut found = None;
                    for owner in declarations {
                        let binders = owner.parameter_binders(fuel)?;
                        for (slot, binder) in binders.iter().enumerate() {
                            if binder.declaration == *declaration {
                                if found.is_some() || binder.sort != Sort::ComptimeValue {
                                    return Err(
                                        "ambiguous or mismatched trait value declaration".into()
                                    );
                                }
                                crate::type_computations::charge_term(
                                    &owner.parameter_types[slot],
                                    fuel,
                                )
                                .map_err(|e| e.to_string())?;
                                found = Some(owner.parameter_types[slot].clone());
                            }
                        }
                    }
                    found.ok_or("trait value argument requires declared domain evidence")?
                }
                _ => return Err("trait value argument requires declared domain evidence".into()),
            };
            if domain != actual {
                return Err("trait value argument domain mismatch".into());
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_record_index(
    value: &Term,
    domain: &Term,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![(value, domain)];
    while let Some((value, domain)) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("trait record index budget exhausted")?;
        if let (Term::RecordValue(values), Term::Record(types)) = (value, domain) {
            if values.len() != types.len() {
                return Err("trait record index field count mismatch".into());
            }
            *fuel = fuel
                .checked_sub(values.len())
                .ok_or("trait record index budget exhausted")?;
            for ((name, value), (expected, ty)) in values.iter().zip(types) {
                *fuel = fuel
                    .checked_sub(name.len().saturating_add(expected.len()))
                    .ok_or("trait record index budget exhausted")?;
                if name != expected {
                    return Err("trait record index field mismatch".into());
                }
                pending.push((value, ty));
            }
            continue;
        }
        if *domain == Term::Primitive(Primitive::Type) && value.sort() == Sort::Type {
            continue;
        }
        if let (Term::Integer(value), Term::Primitive(primitive)) = (value, domain) {
            if let Some((signed, width)) = primitive.fixed_integer_layout() {
                if if signed {
                    value.to_signed(width).is_ok()
                } else {
                    value.to_unsigned(width).is_ok()
                } {
                    continue;
                }
                return Err("trait integer argument is outside its declared domain".into());
            }
        }
        if value.scalar_index_domain().map(Term::Primitive).as_ref() != Some(domain) {
            return Err("trait record index field domain mismatch".into());
        }
    }
    Ok(())
}

impl crate::program::Package {
    /// Sort evidence inherited from trait, nominal and impl declarations, keyed by stable binder
    /// identity. Never infer a binder's sort from a supplied argument.
    pub fn declaration_parameter_sorts(
        &self,
        fuel: &mut usize,
    ) -> Result<std::collections::BTreeMap<crate::DeclarationId, Sort>, String> {
        let mut sorts = std::collections::BTreeMap::new();
        for declaration in &self.traits {
            for binder in declaration.binders_with_self(fuel)? {
                if sorts.insert(binder.declaration, binder.sort).is_some() {
                    return Err("duplicate trait parameter identity across declarations".into());
                }
            }
        }
        let mut inherited_domains = std::collections::BTreeMap::new();
        for (ids, domains) in self
            .structs
            .iter()
            .map(|d| (&d.type_parameters, &d.parameter_types))
            .chain(
                self.enums
                    .iter()
                    .map(|d| (&d.type_parameters, &d.parameter_types)),
            )
            .chain(
                self.implementations
                    .iter()
                    .map(|d| (&d.type_parameters, &d.parameter_types)),
            )
        {
            *fuel = fuel
                .checked_sub(ids.len())
                .ok_or("declaration parameter budget exhausted")?;
            for domain in domains {
                crate::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
            }
            for (index, binder) in crate::declaration_parameters(ids, domains)
                .map_err(|e| e.to_string())?
                .into_iter()
                .enumerate()
            {
                let domain = domains
                    .get(index)
                    .cloned()
                    .unwrap_or(Term::Primitive(Primitive::Type));
                // Synthetic inherent implementations inherit the container's
                // binder identities. Reuse is valid only with the same domain.
                if let Some(previous) = inherited_domains.get(&binder.declaration) {
                    if previous != &domain {
                        return Err("conflicting inherited declaration parameter domain".into());
                    }
                    continue;
                }
                if sorts.insert(binder.declaration, binder.sort).is_some() {
                    return Err("duplicate declaration parameter identity".into());
                }
                inherited_domains.insert(binder.declaration, domain);
            }
        }
        Ok(sorts)
    }
}
