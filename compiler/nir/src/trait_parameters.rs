//! Declaration domains for trait indices. Identity and domain formation do not
//! establish an implementation, a runtime environment, or a program contract.
use crate::{DeclarationParameter, Primitive, Sort, Term, program::Trait};
use std::collections::{BTreeMap, BTreeSet};

impl Term {
    /// Structural index values have scalar or Type leaves. Only Type leaves
    /// participate in type equality; the enclosing record is a program value.
    pub fn is_structural_index_value(&self) -> bool {
        self.index_value_shape(false)
    }

    /// Shape only: symbolic leaves still require declaration-domain admission.
    pub fn is_symbolic_index_value(&self) -> bool {
        self.index_value_shape(true)
    }

    fn index_value_shape(&self, symbolic: bool) -> bool {
        let mut pending = vec![(self, 0usize)];
        let mut remaining = 100_000usize;
        while let Some((value, depth)) = pending.pop() {
            if remaining == 0 || depth >= 128 {
                return false;
            }
            remaining -= 1;
            if symbolic
                && matches!(
                    value,
                    Self::Rigid {
                        sort: Sort::ComptimeValue,
                        ..
                    }
                )
            {
                continue;
            }
            if value.scalar_index_domain().is_some() || (depth > 0 && value.sort() == Sort::Type) {
                continue;
            }
            match value {
                Self::ClosureContextValue(fields) => {
                    if fields.len() > remaining.saturating_sub(pending.len())
                        || fields.iter().any(|field| field.sort() != Sort::Type)
                    {
                        return false;
                    }
                    pending.extend(fields.iter().map(|field| (field, depth + 1)));
                }
                Self::RecordValue(fields) => {
                    if fields.len() > remaining.saturating_sub(pending.len()) {
                        return false;
                    }
                    pending.extend(fields.iter().map(|(_, value)| (value, depth + 1)));
                }
                Self::OptionalValue { ty, value } => {
                    pending.push((ty, depth + 1));
                    if let Some(value) = value {
                        pending.push((value, depth + 1));
                    }
                }
                Self::ErrorValue { ty, label, value } => {
                    pending.push((ty, depth + 1));
                    if let Some(label) = label {
                        pending.push((label, depth + 1));
                    }
                    pending.push((value, depth + 1));
                }
                Self::TupleValue(fields) | Self::NominalValue { fields, .. } => {
                    if fields.len() > remaining.saturating_sub(pending.len()) {
                        return false;
                    }
                    pending.extend(fields.iter().map(|value| (value, depth + 1)));
                }
                _ => return false,
            }
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
            } else if let (
                Self::NominalValue {
                    ty: a,
                    variant: av,
                    fields: af,
                },
                Self::NominalValue {
                    ty: b,
                    variant: bv,
                    fields: bf,
                },
            ) = (left, right)
            {
                if av != bv
                    || af.len() != bf.len()
                    || af.len().saturating_add(1) > remaining.saturating_sub(pending.len())
                {
                    return None;
                }
                pending.push((a, b, depth + 1));
                pending.extend(af.iter().zip(bf).rev().map(|(a, b)| (a, b, depth + 1)));
            } else if let (
                Self::OptionalValue { ty: a, value: av },
                Self::OptionalValue { ty: b, value: bv },
            ) = (left, right)
            {
                if av.is_some() != bv.is_some() {
                    return None;
                }
                pending.push((a, b, depth + 1));
                if let (Some(a), Some(b)) = (av, bv) {
                    pending.push((a, b, depth + 1));
                }
            } else if let (
                Self::ErrorValue {
                    ty: a,
                    label: al,
                    value: av,
                },
                Self::ErrorValue {
                    ty: b,
                    label: bl,
                    value: bv,
                },
            ) = (left, right)
            {
                if al.is_some() != bl.is_some() {
                    return None;
                }
                pending.push((a, b, depth + 1));
                if let (Some(a), Some(b)) = (al, bl) {
                    pending.push((a, b, depth + 1));
                }
                pending.push((av, bv, depth + 1));
            } else if let (Self::TupleValue(a), Self::TupleValue(b)) = (left, right) {
                if a.len() != b.len() || a.len() > remaining.saturating_sub(pending.len()) {
                    return None;
                }
                pending.extend(a.iter().zip(b).rev().map(|(a, b)| (a, b, depth + 1)));
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
            } else if left == right
                && matches!(
                    left,
                    Self::Rigid {
                        sort: Sort::ComptimeValue,
                        ..
                    }
                )
            {
                continue;
            } else if left.scalar_index_domain().is_none() || left != right {
                return None;
            }
        }
        Some(relations)
    }

    /// Refute value identity using only known scalar or record-shape differences.
    /// Unknown value leaves and Type relations cannot establish a mismatch.
    pub fn index_values_definitely_differ(&self, other: &Self, fuel: &mut usize) -> bool {
        if crate::type_computations::charge_term(self, fuel).is_err()
            || crate::type_computations::charge_term(other, fuel).is_err()
        {
            return false;
        }
        let mut pending = vec![(self, other)];
        while let Some((left, right)) = pending.pop() {
            let Some(remaining) = fuel.checked_sub(1) else {
                return false;
            };
            *fuel = remaining;
            if left.sort() == Sort::Type || right.sort() == Sort::Type {
                continue;
            }
            if left.has_pending_error_branch() || right.has_pending_error_branch() {
                // Unknown inner branches cannot establish disjoint indices.
                continue;
            }
            match (left, right) {
                (
                    Self::NominalValue {
                        variant: av,
                        fields: a,
                        ..
                    },
                    Self::NominalValue {
                        variant: bv,
                        fields: b,
                        ..
                    },
                ) => {
                    if av != bv || a.len() != b.len() {
                        return true;
                    }
                    if a.len() > fuel.saturating_sub(pending.len()) {
                        return false;
                    }
                    pending.extend(a.iter().zip(b));
                }
                (
                    Self::OptionalValue { ty: a, value: av },
                    Self::OptionalValue { ty: b, value: bv },
                ) => {
                    if av.is_some() != bv.is_some() {
                        return true;
                    }
                    if a == b {
                        if let (Some(a), Some(b)) = (av, bv) {
                            pending.push((a, b));
                        }
                    }
                }
                (
                    Self::ErrorValue {
                        label: a,
                        value: av,
                        ..
                    },
                    Self::ErrorValue {
                        label: b,
                        value: bv,
                        ..
                    },
                ) => {
                    if a.is_some() != b.is_some() {
                        return true;
                    }
                    // Distinct-looking labels may be equal by type evidence.
                    // Payload mismatches refute only an identical branch domain.
                    if a == b {
                        pending.push((av, bv));
                    }
                }
                (Self::TupleValue(a), Self::TupleValue(b)) => {
                    if a.len() != b.len() {
                        return true;
                    }
                    if a.len() > fuel.saturating_sub(pending.len()) {
                        return false;
                    }
                    pending.extend(a.iter().zip(b));
                }
                (Self::RecordValue(a), Self::RecordValue(b)) => {
                    if a.len() != b.len() {
                        return true;
                    }
                    if a.len() > fuel.saturating_sub(pending.len()) {
                        return false;
                    }
                    for ((name, a), (other_name, b)) in a.iter().zip(b) {
                        if name != other_name {
                            return true;
                        }
                        pending.push((a, b));
                    }
                }
                (Self::RecordValue(_), value) | (value, Self::RecordValue(_))
                    if value.scalar_index_domain().is_some() =>
                {
                    return true;
                }
                (a, b)
                    if a.scalar_index_domain().is_some() && b.scalar_index_domain().is_some() =>
                {
                    if a != b {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
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
            Self::Float16(_) => Primitive::F16,
            Self::BFloat16(_) => Primitive::Bf16,
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

    /// Structural substitution for Self followed by interface inputs. The
    /// caller separately admits values and establishes trait obligations.
    pub fn input_substitution<'a>(
        &self,
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<crate::DeclarationSubstitution<'a>, crate::type_computations::Error> {
        use crate::type_computations::{Error, charge_term};
        *fuel = fuel
            .checked_sub(self.type_parameters.len().saturating_add(1))
            .ok_or(Error::Budget)?;
        let parameters = self.parameters_with_self();
        let mut domains = vec![Term::Primitive(Primitive::Type)];
        for index in 0..self.type_parameters.len() {
            let domain = self
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            charge_term(domain, fuel)?;
            domains.push(domain.clone());
        }
        if !self.parameter_types.is_empty()
            && self.parameter_types.len() != self.type_parameters.len()
        {
            return Err(Error::Scope(crate::TypeError::ArgumentCount {
                expected: self.type_parameters.len(),
                actual: self.parameter_types.len(),
            }));
        }
        crate::DeclarationSubstitution::bounded(&parameters, &domains, arguments, frames, fuel)
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
        self.validate_argument_domains(
            None,
            arguments,
            scope,
            declarations,
            &BTreeMap::new(),
            fuel,
            &mut |_, _, _| false,
            None,
        )
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
        self.validate_argument_domains(
            Some(subject),
            arguments,
            scope,
            declarations,
            &BTreeMap::new(),
            fuel,
            &mut |_, _, _| false,
            None,
        )
    }

    /// Validate rigid value indices using the caller's declaration-domain table.
    /// Evidence is keyed by declaration identity, never inferred from this slot.
    pub fn validate_arguments_with_domains(
        &self,
        subject: &Term,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        domains: &BTreeMap<crate::DeclarationId, Term>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        self.validate_arguments_with_domain_relations(
            subject,
            arguments,
            scope,
            declarations,
            domains,
            fuel,
            |_, _, _| false,
        )
    }

    /// The caller may discharge equality of Type domains using its checked context.
    /// This callback never establishes equality of value indices.
    pub fn validate_arguments_with_domain_relations(
        &self,
        subject: &Term,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        domains: &BTreeMap<crate::DeclarationId, Term>,
        fuel: &mut usize,
        mut equivalent: impl FnMut(&Term, &Term, &mut usize) -> bool,
    ) -> Result<(), String> {
        self.validate_arguments_with_layouts(
            subject,
            arguments,
            scope,
            declarations,
            domains,
            fuel,
            &mut equivalent,
            None,
        )
    }

    /// Validate immutable nominal payloads against the supplied declaration world.
    /// Domain relations apply only to Types; payload identity remains structural.
    pub fn validate_arguments_with_layouts(
        &self,
        subject: &Term,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        domains: &BTreeMap<crate::DeclarationId, Term>,
        fuel: &mut usize,
        mut equivalent: impl FnMut(&Term, &Term, &mut usize) -> bool,
        layouts: Option<&crate::program::Package>,
    ) -> Result<(), String> {
        crate::type_computations::charge_term(subject, fuel).map_err(|e| e.to_string())?;
        subject.check_scope(scope).map_err(|e| e.to_string())?;
        if subject.sort() != Sort::Type {
            return Err("trait subject must be a Type".into());
        }
        self.validate_argument_domains(
            Some(subject),
            arguments,
            scope,
            declarations,
            domains,
            fuel,
            &mut equivalent,
            layouts,
        )
    }

    fn validate_argument_domains(
        &self,
        subject: Option<&Term>,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        domains: &BTreeMap<crate::DeclarationId, Term>,
        fuel: &mut usize,
        equivalent: &mut impl FnMut(&Term, &Term, &mut usize) -> bool,
        layouts: Option<&crate::program::Package>,
    ) -> Result<(), String> {
        let target = layouts.and_then(|package| package.target);
        let binders = self.parameter_binders(fuel)?;
        if arguments.len() != binders.len() {
            return Err("trait interface argument count mismatch".into());
        }
        if binders.iter().all(|binder| binder.sort == Sort::Type) {
            for argument in arguments {
                crate::type_computations::charge_term(argument, fuel).map_err(|e| e.to_string())?;
                argument.check_scope(scope).map_err(|e| e.to_string())?;
                if argument.sort() != Sort::Type {
                    return Err("trait interface argument sort mismatch".into());
                }
            }
            return Ok(());
        }
        let mut ids = Vec::with_capacity(binders.len() + 1);
        let mut declared_domains = Vec::with_capacity(binders.len() + 1);
        let mut inputs = Vec::with_capacity(binders.len() + 1);
        ids.push(self.self_parameter);
        declared_domains.push(Term::Primitive(Primitive::Type));
        inputs.push(if let Some(subject) = subject {
            crate::type_computations::charge_term(subject, fuel).map_err(|e| e.to_string())?;
            subject.clone()
        } else {
            Term::Rigid {
                declaration: self.self_parameter,
                sort: Sort::Type,
            }
        });
        for (index, argument) in arguments.iter().enumerate() {
            let domain = self
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            crate::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
            crate::type_computations::charge_term(argument, fuel).map_err(|e| e.to_string())?;
            ids.push(self.type_parameters[index]);
            declared_domains.push(domain.clone());
            inputs.push(argument.clone());
        }
        let substitution =
            crate::DeclarationSubstitution::bounded(&ids, &declared_domains, &inputs, scope, fuel)
                .map_err(|e| e.to_string())?;
        for (index, argument) in arguments.iter().enumerate() {
            let domain = substitution
                .instantiate_bounded(&declared_domains[index + 1], fuel)
                .map_err(|e| e.to_string())?;
            if domain == Term::Primitive(Primitive::Type) {
                continue;
            }
            if matches!(
                argument,
                Term::OptionalValue { .. }
                    | Term::ErrorValue { .. }
                    | Term::NominalValue { .. }
                    | Term::TupleValue(_)
            ) {
                crate::nominal_parameters::validate_symbolic_value_in(
                    argument,
                    &domain,
                    fuel,
                    |ty, variant, fuel| {
                        let package = layouts.ok_or_else(|| {
                            crate::NominalAdmissionError::from(
                                "nominal value admission requires declaration layout evidence",
                            )
                        })?;
                        crate::normal_forms::nominal_value_fields_budgeted(
                            package, ty, variant, fuel,
                        )
                        .ok_or_else(|| {
                            if *fuel == 0 {
                                crate::NominalAdmissionError::Budget
                            } else {
                                crate::NominalAdmissionError::from("invalid nominal value layout")
                            }
                        })
                    },
                    |value, expected, fuel| {
                        let Term::Rigid {
                            declaration,
                            sort: Sort::ComptimeValue,
                        } = value
                        else {
                            return false;
                        };
                        declared_value_domain(*declaration, declarations, domains, scope, fuel)
                            .is_ok_and(|actual| {
                                actual == *expected || equivalent(&actual, expected, fuel)
                            })
                    },
                    target,
                )
                .map_err(|e| e.to_string())?;
                continue;
            }
            if matches!(argument, Term::RecordValue(_)) && argument.is_symbolic_index_value() {
                validate_record_index_with_admission_in(
                    argument,
                    &domain,
                    fuel,
                    |value, expected, fuel| {
                        if matches!(value, Term::NominalValue { .. }) {
                            crate::nominal_parameters::validate_symbolic_value_in(
                                value,
                                expected,
                                fuel,
                                |ty, variant, fuel| {
                                    let package = layouts.ok_or_else(|| {
                                        crate::NominalAdmissionError::from(
                                            "nominal record index requires declaration layouts",
                                        )
                                    })?;
                                    crate::normal_forms::nominal_value_fields_budgeted(
                                        package, ty, variant, fuel,
                                    )
                                    .ok_or_else(|| {
                                        if *fuel == 0 {
                                            crate::NominalAdmissionError::Budget
                                        } else {
                                            crate::NominalAdmissionError::from(
                                                "invalid nominal record index layout",
                                            )
                                        }
                                    })
                                },
                                |value, expected, fuel| {
                                    let Term::Rigid {
                                        declaration,
                                        sort: Sort::ComptimeValue,
                                    } = value
                                    else {
                                        return false;
                                    };
                                    declared_value_domain(
                                        *declaration,
                                        declarations,
                                        domains,
                                        scope,
                                        fuel,
                                    )
                                    .is_ok_and(|actual| {
                                        actual == *expected || equivalent(&actual, expected, fuel)
                                    })
                                },
                                target,
                            )?;
                            return Ok(true);
                        }
                        let Term::Rigid {
                            declaration,
                            sort: Sort::ComptimeValue,
                        } = value
                        else {
                            return Ok(false);
                        };
                        Ok(
                            declared_value_domain(*declaration, declarations, domains, scope, fuel)
                                .is_ok_and(|actual| {
                                    actual == *expected || equivalent(&actual, expected, fuel)
                                }),
                        )
                    },
                    target,
                )?;
                continue;
            }
            // Fixed-width proxies freeze to the same exact integer payload as
            // Integer. The declared domain supplies width and signedness; the
            // payload must fit exactly, without truncation or host-size guesses.
            if let (Term::Integer(value), Term::Primitive(primitive)) = (argument, &domain) {
                if let Some((signed, width)) = target.map_or_else(
                    || primitive.fixed_integer_layout(),
                    |target| target.integer_layout(*primitive),
                ) {
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
                } => declared_value_domain(*declaration, declarations, domains, scope, fuel)?,
                _ => return Err("trait value argument requires declared domain evidence".into()),
            };
            if domain != actual && !equivalent(&domain, &actual, fuel) {
                return Err("trait value argument domain mismatch".into());
            }
        }
        Ok(())
    }
}

fn declared_value_domain(
    declaration: crate::DeclarationId,
    declarations: &[Trait],
    domains: &BTreeMap<crate::DeclarationId, Term>,
    scope: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Term, String> {
    let mut found = None;
    for owner in declarations {
        let binders = owner.parameter_binders(fuel)?;
        for (slot, binder) in binders.iter().enumerate() {
            if binder.declaration == declaration {
                if found.is_some() || binder.sort != Sort::ComptimeValue {
                    return Err("ambiguous or mismatched trait value declaration".into());
                }
                crate::type_computations::charge_term(&owner.parameter_types[slot], fuel)
                    .map_err(|e| e.to_string())?;
                found = Some(owner.parameter_types[slot].clone());
            }
        }
    }
    if let Some(domain) = domains.get(&declaration) {
        crate::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
        if domain.sort() != Sort::Type || found.as_ref().is_some_and(|old| old != domain) {
            return Err("conflicting trait value declaration domain".into());
        }
        domain.check_scope(scope).map_err(|e| e.to_string())?;
        found = Some(domain.clone());
    }
    found.ok_or_else(|| "trait value argument requires declared domain evidence".into())
}

pub(crate) fn validate_record_index(
    value: &Term,
    domain: &Term,
    fuel: &mut usize,
) -> Result<(), crate::nominal_parameters::AdmissionError> {
    validate_record_index_with(value, domain, fuel, |_, _, _| false)
}
/// Check record shape and concrete leaves; the callback admits only symbolic
/// leaves whose declaration domain is established by the caller.
pub fn validate_record_index_with(
    value: &Term,
    domain: &Term,
    fuel: &mut usize,
    mut symbolic: impl FnMut(&Term, &Term, &mut usize) -> bool,
) -> Result<(), crate::nominal_parameters::AdmissionError> {
    validate_record_index_with_admission(value, domain, fuel, |value, domain, fuel| {
        Ok(symbolic(value, domain, fuel))
    })
}

/// Ordered structural index validation with fallible declaration-backed leaf admission.
pub fn validate_record_index_with_admission(
    value: &Term,
    domain: &Term,
    fuel: &mut usize,
    admit: impl FnMut(&Term, &Term, &mut usize) -> Result<bool, crate::NominalAdmissionError>,
) -> Result<(), crate::NominalAdmissionError> {
    validate_record_index_with_admission_in(value, domain, fuel, admit, None)
}

pub fn validate_record_index_with_admission_in(
    value: &Term,
    domain: &Term,
    fuel: &mut usize,
    mut admit: impl FnMut(&Term, &Term, &mut usize) -> Result<bool, crate::NominalAdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<(), crate::NominalAdmissionError> {
    let mut pending = vec![(value, domain)];
    while let Some((value, domain)) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or(crate::nominal_parameters::AdmissionError::Budget)?;
        if let (Term::RecordValue(values), Term::Record(types)) = (value, domain) {
            if values.len() != types.len() {
                return Err("trait record index field count mismatch".into());
            }
            *fuel = fuel
                .checked_sub(values.len())
                .ok_or(crate::nominal_parameters::AdmissionError::Budget)?;
            for ((name, value), (expected, ty)) in values.iter().zip(types) {
                *fuel = fuel
                    .checked_sub(name.len().saturating_add(expected.len()))
                    .ok_or(crate::nominal_parameters::AdmissionError::Budget)?;
                if name != expected {
                    return Err("trait record index field mismatch".into());
                }
                pending.push((value, ty));
            }
            continue;
        }
        if let Term::OptionalValue { ty, value } = value {
            let Term::Optional(payload) = domain else {
                return Err("trait optional index requires an optional domain".into());
            };
            if ty.as_ref() != domain {
                return Err("trait optional index domain mismatch".into());
            }
            if let Some(value) = value {
                pending.push((value, payload));
            }
            continue;
        }
        if let Term::ErrorValue {
            value: payload_value,
            ..
        } = value
        {
            let payload = crate::error_results::frozen_payload_domain(value, domain, fuel)
                .map_err(|error| {
                    if error == crate::error_results::Error::Budget {
                        crate::NominalAdmissionError::Budget
                    } else {
                        crate::NominalAdmissionError::from(
                            "trait error index domain or branch mismatch",
                        )
                    }
                })?;
            pending.push((payload_value, payload));
            continue;
        }
        if let (Term::TupleValue(values), Term::Tuple(types)) = (value, domain) {
            if values.len() != types.len() {
                return Err("trait tuple index arity mismatch".into());
            }
            *fuel = fuel
                .checked_sub(values.len())
                .ok_or(crate::nominal_parameters::AdmissionError::Budget)?;
            pending.extend(values.iter().zip(types));
            continue;
        }
        if *domain == Term::Primitive(Primitive::Type) && value.sort() == Sort::Type {
            continue;
        }
        if let (Term::Integer(value), Term::Primitive(primitive)) = (value, domain) {
            if let Some((signed, width)) = target.map_or_else(
                || primitive.fixed_integer_layout(),
                |target| target.integer_layout(*primitive),
            ) {
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
        if admit(value, domain, fuel)? {
            continue;
        }
        if value.scalar_index_domain().map(Term::Primitive).as_ref() != Some(domain) {
            return Err("trait record index field domain mismatch".into());
        }
    }
    Ok(())
}

impl crate::program::Package {
    /// Recover declaration sorts for substitution, including legacy Type-only
    /// binders without explicit domain metadata. Supplied values are not evidence.
    pub fn declaration_binders(
        &self,
        parameters: &[crate::DeclarationId],
        fuel: &mut usize,
    ) -> Result<Vec<DeclarationParameter>, String> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or("declaration binder budget exhausted")?;
        if parameters.is_empty() {
            return Ok(vec![]);
        }
        let sorts = self.declaration_parameter_sorts(fuel)?;
        Ok(parameters
            .iter()
            .map(|id| DeclarationParameter {
                declaration: *id,
                sort: sorts.get(id).copied().unwrap_or(Sort::Type),
            })
            .collect())
    }

    /// Sort evidence inherited from trait, nominal and impl declarations, keyed by stable binder
    /// identity. Never infer a binder's sort from a supplied argument.
    pub fn declaration_parameter_sorts(
        &self,
        fuel: &mut usize,
    ) -> Result<std::collections::BTreeMap<crate::DeclarationId, Sort>, String> {
        Ok(self
            .declaration_parameter_domains(fuel)?
            .into_iter()
            .map(|(id, domain)| {
                (
                    id,
                    if domain == Term::Primitive(Primitive::Type) {
                        Sort::Type
                    } else {
                        Sort::ComptimeValue
                    },
                )
            })
            .collect())
    }

    /// Domains retain the declaration telescope, including value dependencies.
    pub fn declaration_parameter_domains(
        &self,
        fuel: &mut usize,
    ) -> Result<std::collections::BTreeMap<crate::DeclarationId, Term>, String> {
        let mut result = std::collections::BTreeMap::new();
        for declaration in &self.traits {
            for (index, binder) in declaration.binders_with_self(fuel)?.into_iter().enumerate() {
                let domain = index
                    .checked_sub(1)
                    .and_then(|i| declaration.parameter_types.get(i))
                    .unwrap_or(&Term::Primitive(Primitive::Type));
                crate::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
                if result.insert(binder.declaration, domain.clone()).is_some() {
                    return Err("duplicate trait parameter identity across declarations".into());
                }
            }
        }
        let mut callable_ids = BTreeSet::new();
        for declaration in &self.function_parameters {
            *fuel = fuel
                .checked_sub(declaration.parameters.len().saturating_add(1))
                .ok_or("function parameter budget exhausted")?;
            if !callable_ids.insert(declaration.declaration) {
                return Err("duplicate function parameter declaration".into());
            }
            for domain in &declaration.domains {
                crate::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
            }
            declaration.binders().map_err(|e| e.to_string())?;
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
                if result.insert(binder.declaration, domain.clone()).is_some() {
                    return Err("duplicate declaration parameter identity".into());
                }
                inherited_domains.insert(binder.declaration, domain);
            }
        }
        // Newtypes and transparent alias guards can repeat lexical captures
        // from traits or nominal containers. Reuse requires the same domain;
        // their own inputs also provide declaration evidence to generated code.
        for (ids, domains) in self
            .newtypes
            .iter()
            .map(|d| (&d.type_parameters, &d.parameter_types))
            .chain(
                self.alias_contracts
                    .iter()
                    .map(|d| (&d.parameters, &d.parameter_types)),
            )
            .chain(
                self.function_parameters
                    .iter()
                    .map(|d| (&d.parameters, &d.domains)),
            )
        {
            *fuel = fuel
                .checked_sub(ids.len())
                .ok_or("captured declaration parameter budget exhausted")?;
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
                if let Some(previous) = result.get(&binder.declaration) {
                    if previous != &domain {
                        return Err("conflicting captured declaration parameter domain".into());
                    }
                } else {
                    result.insert(binder.declaration, domain);
                }
            }
        }
        Ok(result)
    }
}
