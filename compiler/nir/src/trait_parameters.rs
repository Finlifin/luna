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
                Self::RecordValue(fields) => {
                    if fields.len() > remaining.saturating_sub(pending.len()) {
                        return false;
                    }
                    pending.extend(fields.iter().map(|(_, value)| (value, depth + 1)));
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
            if matches!(argument, Term::NominalValue { .. } | Term::TupleValue(_)) {
                crate::nominal_parameters::validate_symbolic_value(
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
                )
                .map_err(|e| e.to_string())?;
                continue;
            }
            if matches!(argument, Term::RecordValue(_)) && argument.is_symbolic_index_value() {
                validate_record_index_with(argument, &domain, fuel, |value, expected, fuel| {
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
                })?;
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
        if symbolic(value, domain, fuel) {
            continue;
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
