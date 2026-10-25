//! Constructor specialization for already type-checked runtime patterns.
//! This checks a pattern matrix, not the Cartesian product of runtime values.
//! Supported pure scalar guards contribute interval evidence; unfamiliar guards
//! contribute no coverage. This is not a PTC proposition.
#[cfg(test)]
use crate::ptc::Formula;
use crate::ptc::{Budget, Context};
use nir::program::{Enum, Literal, Pattern, Struct};
use nir::{DeclarationId, Primitive, Term};
use std::collections::{BTreeMap, HashMap};
mod negation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Coverage {
    Exhaustive,
    Missing,
    Unknown,
}

#[cfg(test)]
pub(crate) fn check(
    enums: &BTreeMap<DeclarationId, Enum>,
    ty: &Term,
    patterns: &[&Pattern],
    steps: usize,
) -> Coverage {
    check_in_context(
        &BTreeMap::new(),
        enums,
        ty,
        patterns,
        steps,
        &Context::default(),
    )
}

pub(crate) fn check_in_context(
    structs: &BTreeMap<DeclarationId, Struct>,
    enums: &BTreeMap<DeclarationId, Enum>,
    ty: &Term,
    patterns: &[&Pattern],
    steps: usize,
    context: &Context,
) -> Coverage {
    check_in_context_with_tuple_head(structs, enums, ty, patterns, steps, context, None)
}

pub(crate) fn check_in_context_with_tuple_head(
    structs: &BTreeMap<DeclarationId, Struct>,
    enums: &BTreeMap<DeclarationId, Enum>,
    ty: &Term,
    patterns: &[&Pattern],
    steps: usize,
    context: &Context,
    tuple_head: Option<DeclarationId>,
) -> Coverage {
    check_metered(structs, enums, ty, patterns, steps, context, tuple_head).0
}

pub(crate) fn check_metered(
    structs: &BTreeMap<DeclarationId, Struct>,
    enums: &BTreeMap<DeclarationId, Enum>,
    ty: &Term,
    patterns: &[&Pattern],
    steps: usize,
    context: &Context,
    tuple_head: Option<DeclarationId>,
) -> (Coverage, usize) {
    check_metered_in(
        structs, enums, ty, patterns, steps, context, tuple_head, None,
    )
}

pub(crate) fn check_metered_in(
    structs: &BTreeMap<DeclarationId, Struct>,
    enums: &BTreeMap<DeclarationId, Enum>,
    ty: &Term,
    patterns: &[&Pattern],
    steps: usize,
    context: &Context,
    tuple_head: Option<DeclarationId>,
    target: Option<nir::target::TargetData>,
) -> (Coverage, usize) {
    let mut engine = Engine {
        target,
        tuple_head,
        structs,
        enums,
        steps,
        context,
        domains: HashMap::new(),
    };
    let outcome = match engine.matrix(vec![ty], patterns.iter().map(|p| vec![*p]).collect(), 0) {
        Ok(true) => Coverage::Exhaustive,
        Ok(false) => Coverage::Missing,
        Err(()) => Coverage::Unknown,
    };
    (outcome, steps - engine.steps)
}

#[derive(Clone, Copy)]
enum Head<'a> {
    AnonymousRecord(&'a [(String, Term)]),
    Record(DeclarationId),
    Tuple,
    Bool(bool),
    Unit,
    Variant(DeclarationId, usize),
    ErrorResult(Option<&'a Term>),
    // The residual domain of an open scalar or unsupported type. A literal
    // cannot cover this domain; only the default (wildcard) rows contribute.
    Other,
}
struct Engine<'a> {
    target: Option<nir::target::TargetData>,
    tuple_head: Option<DeclarationId>,
    structs: &'a BTreeMap<DeclarationId, Struct>,
    enums: &'a BTreeMap<DeclarationId, Enum>,
    context: &'a Context,
    domains: HashMap<Term, Term>,
    steps: usize,
}
static WILD: Pattern = Pattern::Wild;
impl Engine<'_> {
    fn spend(&mut self, n: usize) -> Result<(), ()> {
        let Some(remaining) = self.steps.checked_sub(n) else {
            self.steps = 0;
            return Err(());
        };
        self.steps = remaining;
        Ok(())
    }
    // Equality evidence changes only the type domain inspected by this matrix.
    // Runtime patterns, guards and coverage propositions never enter PTC.
    // The cache is local to one matrix/context; no assumptions escape its scope.
    fn domain(&mut self, ty: &Term) -> Result<Term, ()> {
        if let Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            arguments,
        } = ty
        {
            if Some(*head) == self.tuple_head {
                if let [Term::TypeListValue(elements)] = arguments.as_slice() {
                    crate::associated::charge_projection_copy(ty, &mut self.steps)
                        .map_err(|_| ())?;
                    return Ok(Term::Tuple(elements.clone()));
                }
            }
        }
        if matches!(
            ty,
            Term::Primitive(_)
                | Term::ErrorResult { .. }
                | Term::Nominal { .. }
                | Term::Tuple(_)
                | Term::Record(_)
                | Term::Function { .. }
                | Term::ComptimeFunction(_)
                | Term::UnsafeFunction(_)
                | Term::Closure { .. }
                | Term::Pointer { .. }
        ) {
            return Ok(ty.clone());
        }
        if let Some(domain) = self.domains.get(ty) {
            return Ok(domain.clone());
        }
        // Reserve all subsolver work from the enclosing matrix budget.
        const ALLOWANCE: usize = 16384;
        self.spend(ALLOWANCE)?;
        let domain = match self
            .context
            .structural_view_with_heads_metered(
                ty,
                Budget {
                    steps: ALLOWANCE,
                    max_depth: 128,
                },
                |head| Some(head) == self.tuple_head,
            )
            .0
        {
            Ok(Some((shape, _))) => shape,
            Ok(None) | Err(crate::ptc::UnknownReason::UnvalidatedModel) => ty.clone(),
            Err(_) => return Err(()),
        };
        let domain = if let Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            arguments,
        } = &domain
        {
            if Some(*head) == self.tuple_head {
                if let [Term::TypeListValue(elements)] = arguments.as_slice() {
                    crate::associated::charge_projection_copy(&domain, &mut self.steps)
                        .map_err(|_| ())?;
                    Term::Tuple(elements.clone())
                } else {
                    domain
                }
            } else {
                domain
            }
        } else {
            domain
        };
        self.domains.insert(ty.clone(), domain.clone());
        Ok(domain)
    }
    fn wildcard(&mut self, pat: &Pattern) -> Result<bool, ()> {
        enum Work<'a> {
            Visit(&'a Pattern),
            Both,
            Any(usize),
        }
        let mut pending = vec![Work::Visit(pat)];
        let mut values = Vec::new();
        while let Some(work) = pending.pop() {
            self.spend(1)?;
            match work {
                Work::Visit(Pattern::Wild) => values.push(true),
                Work::Visit(
                    Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. },
                ) => pending.push(Work::Visit(inner)),
                Work::Visit(Pattern::Then { first, next, .. }) => {
                    pending.push(Work::Both);
                    pending.push(Work::Visit(next));
                    pending.push(Work::Visit(first));
                }
                Work::Visit(Pattern::Or(patterns)) => {
                    self.spend(patterns.len())?;
                    pending.push(Work::Any(patterns.len()));
                    pending.extend(patterns.iter().rev().map(Work::Visit));
                }
                Work::Visit(_) => values.push(false),
                Work::Both => {
                    let b = values.pop().ok_or(())?;
                    let a = values.pop().ok_or(())?;
                    values.push(a && b);
                }
                Work::Any(count) => {
                    let start = values.len().checked_sub(count).ok_or(())?;
                    let any = values[start..].iter().any(|v| *v);
                    values.truncate(start);
                    values.push(any);
                }
            }
        }
        if values.len() != 1 {
            return Err(());
        }
        values.pop().ok_or(())
    }
    // Universal coverage independent of the subject domain. The ordinary
    // wildcard path stays iterative even for very long sequential patterns.
    fn unconditional(&mut self, pat: &Pattern, depth: usize) -> Result<bool, ()> {
        if self.wildcard(pat)? {
            return Ok(true);
        }
        if depth > 256 {
            return Err(());
        }
        self.spend(1)?;
        match pat {
            Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } => {
                self.unconditional(inner, depth + 1)
            }
            Pattern::Then { first, value, next } => Ok(self.unconditional(first, depth + 1)?
                && self.matrix(vec![&value.ty], vec![vec![next]], depth + 1)?),
            Pattern::Or(alternatives) => {
                for alternative in alternatives {
                    if self.unconditional(alternative, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }
    fn matrix<'a>(
        &mut self,
        types: Vec<&'a Term>,
        rows: Vec<Vec<&'a Pattern>>,
        depth: usize,
    ) -> Result<bool, ()> {
        self.spend(1)?;
        if depth > 256 {
            return Err(());
        }
        if types.is_empty() {
            return Ok(!rows.is_empty());
        }
        for row in &rows {
            let mut all = true;
            for pat in row {
                if !self.unconditional(pat, depth)? {
                    all = false;
                    break;
                }
            }
            if all {
                return Ok(true);
            }
        }
        // Strip an unconstrained column without branching over its values. If
        // the remaining columns are exhaustive this is a proof even when this
        // column is empty. On failure still specialize to detect empty domains.
        if !rows.is_empty() {
            let mut all = true;
            for row in &rows {
                if !self.unconditional(row[0], depth)? {
                    all = false;
                    break;
                }
            }
            if all {
                let rest = rows.iter().map(|row| row[1..].to_vec()).collect();
                let covered = self.matrix(types[1..].to_vec(), rest, depth + 1)?;
                if covered || matches!(types[0], Term::Primitive(Primitive::Bool | Primitive::Void))
                {
                    return Ok(covered);
                }
            }
        }
        let domain = self.domain(types[0])?;
        let mut types = types;
        types[0] = &domain;
        // Coverage-only constructor complements. Executable guards and their
        // candidate rollback remain unchanged in the program representation.
        let expanded = rows
            .iter()
            .map(|row| {
                // Scalar engines already understand their executable guards.
                // Rewrapping a constructor's scalar complement here repeats
                // exact analysis and can exhaust the budget for wide BitVecs.
                let specialized_scalar = matches!(&domain, Term::Primitive(p)
                    if !matches!(p, Primitive::Bool | Primitive::Void));
                if !specialized_scalar {
                    // Wrapper traversal has its own bound; matrix depth also
                    // counts already-specialized subject columns.
                    self.expand_negations(row[0], types[0], 0)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Vec<_>, ()>>()?;
        let rows = if expanded.iter().any(Option::is_some) {
            self.spend(rows.iter().map(Vec::len).sum())?;
            rows.iter()
                .zip(&expanded)
                .map(|(row, expanded)| {
                    let mut row = row.clone();
                    if let Some(expanded) = expanded {
                        row[0] = expanded;
                    }
                    row
                })
                .collect::<Vec<_>>()
        } else {
            rows
        };
        if *types[0] == Term::Primitive(Primitive::Symbol) {
            let cells =
                crate::symbol_coverage::cases(rows.iter().map(|row| row[0]), &mut self.steps)?;
            for cell in cells {
                let mut next = Vec::new();
                for row in &rows {
                    if crate::symbol_coverage::covers_case(row[0], &cell, &mut self.steps)? {
                        self.spend(row.len())?;
                        next.push(row[1..].to_vec());
                    }
                }
                if !self.matrix(types[1..].to_vec(), next, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if *types[0] == Term::Primitive(Primitive::Float) {
            return self.float_matrix(&types[1..], rows, depth);
        }
        if let Term::Primitive(Primitive::BitVec(width)) = types[0] {
            return self.bit_matrix(*width, &types[1..], rows, depth);
        }
        if let Term::Primitive(primitive) = types[0] {
            if let Some(domain) = crate::machine_float_coverage::Domain::new(*primitive) {
                return self.interval_matrix_with_float(
                    &domain.ranges(),
                    &types[1..],
                    rows,
                    depth,
                    Some(domain),
                );
            }
            if let Some(domain) = crate::interval_coverage::domain_in(*primitive, self.target) {
                return self.interval_matrix(&domain, &types[1..], rows, depth);
            }
        }
        let instantiated = match types[0] {
            Term::Nominal {
                declaration,
                arguments,
            } if self.enums.contains_key(declaration) => Some(
                self.enums[declaration]
                    .instantiate_variants_in_scope(arguments, self.context.scope())
                    .map_err(|_| ())?,
            ),
            _ => None,
        };
        let record_fields = match types[0] {
            Term::Nominal {
                declaration,
                arguments,
            } if self.structs.contains_key(declaration) => Some(
                self.structs[declaration]
                    .instantiate_fields_in_scope(arguments, self.context.scope())
                    .map_err(|_| ())?,
            ),
            _ => None,
        };
        let mut branches = Vec::new();
        match types[0] {
            Term::ErrorResult { errors, result } => {
                self.spend(errors.len().saturating_add(1))?;
                branches.push((Head::ErrorResult(None), vec![result.as_ref()]));
                for label in errors {
                    branches.push((Head::ErrorResult(Some(label)), vec![label]));
                }
            }
            Term::Record(fields) => branches.push((
                Head::AnonymousRecord(fields),
                fields.iter().map(|(_, ty)| ty).collect(),
            )),
            Term::Tuple(fields) => branches.push((Head::Tuple, fields.iter().collect())),
            Term::Nominal { declaration, .. } if self.structs.contains_key(declaration) => {
                branches.push((
                    Head::Record(*declaration),
                    record_fields
                        .as_ref()
                        .unwrap()
                        .iter()
                        .map(|(_, ty)| ty)
                        .collect(),
                ));
            }
            Term::Primitive(Primitive::Never) => return Ok(true),
            Term::Primitive(Primitive::Bool) => {
                branches.push((Head::Bool(false), vec![]));
                branches.push((Head::Bool(true), vec![]));
            }
            Term::Primitive(Primitive::Void) => branches.push((Head::Unit, vec![])),
            Term::Nominal { declaration, .. } if self.enums.contains_key(declaration) => {
                let variants = instantiated.as_ref().unwrap();
                self.spend(variants.len())?;
                for (index, variant) in variants.iter().enumerate() {
                    branches.push((
                        Head::Variant(*declaration, index),
                        variant.fields.iter().collect(),
                    ));
                }
            }
            _ => branches.push((Head::Other, vec![])),
        }
        let mut solved = Vec::new();
        for (head, fields) in branches {
            let mut specialized = Vec::new();
            for row in &rows {
                self.specialize(
                    row[0],
                    head,
                    fields.len(),
                    &row[1..],
                    &mut specialized,
                    depth,
                )?;
            }
            let mut next = fields;
            next.extend_from_slice(&types[1..]);
            // Distinct constructors often yield the same residual matrix (for
            // example `true or false` in many adjacent fields). Reuse that
            // proof instead of traversing an exponential constructor product.
            if solved
                .iter()
                .any(|(types, rows)| *types == next && *rows == specialized)
            {
                continue;
            }
            if !self.matrix(next.clone(), specialized.clone(), depth + 1)? {
                return Ok(false);
            }
            solved.push((next, specialized));
        }
        Ok(true)
    }
    fn float_matrix<'a>(
        &mut self,
        tail_types: &[&'a Term],
        rows: Vec<Vec<&'a Pattern>>,
        depth: usize,
    ) -> Result<bool, ()> {
        let mut predicates = Vec::new();
        for row in &rows {
            predicates.push(crate::float_coverage::extract(row[0], &mut self.steps)?);
        }
        let mut atoms = Vec::new();
        for predicate in predicates.iter().flatten() {
            predicate.atoms(&mut atoms, &mut self.steps)?;
        }
        // Preserve each endpoint and every open interval separately, so
        // strict comparisons cannot accidentally cover a missing boundary.
        for cell in crate::float_coverage::cells(atoms, &mut self.steps)? {
            let mut next = Vec::new();
            for (row, predicate) in rows.iter().zip(&predicates) {
                if let Some(predicate) = predicate {
                    if predicate.matches(cell, &mut self.steps)? {
                        self.spend(row.len())?;
                        next.push(row[1..].to_vec());
                    }
                }
            }
            if !self.matrix(tail_types.to_vec(), next, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn interval_matrix<'a>(
        &mut self,
        domain: &[crate::interval_coverage::Interval],
        tail_types: &[&'a Term],
        rows: Vec<Vec<&'a Pattern>>,
        depth: usize,
    ) -> Result<bool, ()> {
        self.interval_matrix_with_float(domain, tail_types, rows, depth, None)
    }
    fn interval_matrix_with_float<'a>(
        &mut self,
        domain: &[crate::interval_coverage::Interval],
        tail_types: &[&'a Term],
        rows: Vec<Vec<&'a Pattern>>,
        depth: usize,
        float: Option<crate::machine_float_coverage::Domain>,
    ) -> Result<bool, ()> {
        use crate::interval_coverage::{covers, cuts, extract};
        let mut intervals = Vec::new();
        for row in &rows {
            intervals.push(match float {
                Some(float) => float.extract(row[0], &mut self.steps)?,
                None => extract(row[0], &mut self.steps)?,
            });
        }
        let cuts = cuts(
            domain.iter().chain(intervals.iter().flatten()),
            &mut self.steps,
        )?;
        for cut in cuts.windows(2) {
            let mut in_domain = false;
            for range in domain {
                if covers(range, &cut[0], &cut[1], &mut self.steps)? {
                    in_domain = true;
                    break;
                }
            }
            if !in_domain {
                continue;
            }
            let mut next = Vec::new();
            for (row, ranges) in rows.iter().zip(&intervals) {
                for range in ranges {
                    if covers(range, &cut[0], &cut[1], &mut self.steps)? {
                        self.spend(row.len())?;
                        next.push(row[1..].to_vec());
                        break;
                    }
                }
            }
            if !self.matrix(tail_types.to_vec(), next, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn bit_matrix<'a>(
        &mut self,
        width: usize,
        tail_types: &[&'a Term],
        rows: Vec<Vec<&'a Pattern>>,
        depth: usize,
    ) -> Result<bool, ()> {
        // Keep each sorted cube once. Decision nodes advance a cursor instead
        // of repeatedly removing BTreeMap keys or cloning the remaining bits.
        let mut cubes = Vec::new();
        let mut tails = Vec::new();
        let mut initial = Vec::new();
        let mut analysis = crate::bit_coverage::Analysis::new(self.context, self.target);
        for row in rows {
            for cube in analysis.extract(row[0], width, &mut self.steps)? {
                self.spend(cube.len().saturating_add(row.len()).saturating_add(1))?;
                initial.push((cubes.len(), 0usize));
                cubes.push(cube.into_iter().collect::<Vec<_>>());
                tails.push(row[1..].to_vec());
            }
        }
        let mut pending = vec![initial];
        while let Some(rows) = pending.pop() {
            self.spend(rows.len().saturating_add(1))?;
            let mut unconditional = Vec::new();
            for (id, offset) in &rows {
                if *offset == cubes[*id].len() {
                    self.spend(tails[*id].len().saturating_add(1))?;
                    unconditional.push(tails[*id].clone());
                }
            }
            if self.matrix(tail_types.to_vec(), unconditional, depth + 1)? {
                continue;
            }
            let Some(bit) = rows
                .iter()
                .filter_map(|(id, offset)| cubes[*id].get(*offset).map(|(bit, _)| *bit))
                .min()
            else {
                return Ok(false);
            };
            let mut branches = [Vec::new(), Vec::new()];
            for (id, offset) in rows {
                self.spend(1)?;
                if let Some((_, value)) = cubes[id].get(offset).filter(|(next, _)| *next == bit) {
                    branches[usize::from(*value)].push((id, offset + 1));
                } else {
                    self.spend(1)?;
                    branches[0].push((id, offset));
                    branches[1].push((id, offset));
                }
            }
            let [zero, one] = branches;
            self.spend(zero.len().min(one.len()))?;
            if zero == one {
                pending.push(zero);
            } else if zero.len() > one.len() {
                pending.push(zero);
                pending.push(one);
            } else {
                pending.push(one);
                pending.push(zero);
            }
        }
        Ok(true)
    }
    fn specialize<'a>(
        &mut self,
        pat: &'a Pattern,
        head: Head<'_>,
        arity: usize,
        tail: &[&'a Pattern],
        out: &mut Vec<Vec<&'a Pattern>>,
        depth: usize,
    ) -> Result<(), ()> {
        let mut pending = vec![pat];
        while let Some(pat) = pending.pop() {
            self.spend(1)?;
            match pat {
                Pattern::Binding { inner, .. } | Pattern::Remap { pattern: inner, .. } => {
                    pending.push(inner);
                    continue;
                }
                Pattern::Then { first, value, next } => {
                    // A computed suffix observes its own result domain, not
                    // the original subject. Only universal coverage of that
                    // domain allows us to retain the first pattern's coverage.
                    // Reuse this engine's fuel and depth; do not execute value.
                    if self.wildcard(next)?
                        || self.matrix(vec![&value.ty], vec![vec![next]], depth + 1)?
                    {
                        pending.push(first);
                    } else if let nir::program::ExprKind::Literal(Literal::Bool(selected)) =
                        value.kind
                    {
                        // A constant computation selects one result case. Do
                        // not recursively re-evaluate a long first pattern.
                        if crate::interval_coverage::boolean_case(next, selected, &mut self.steps)?
                            == Some(true)
                        {
                            pending.push(first);
                        }
                    } else if let Head::Bool(value) = head {
                        // A non-universal suffix may still cover this subject
                        // after its candidate-local computation. Preserve the
                        // iterative universal path above for long chains.
                        if crate::interval_coverage::boolean_case(pat, value, &mut self.steps)?
                            == Some(true)
                        {
                            self.spend(tail.len().saturating_add(1))?;
                            out.push(tail.to_vec());
                        }
                    }
                    continue;
                }
                Pattern::Or(patterns) => {
                    self.spend(patterns.len())?;
                    pending.extend(patterns.iter().rev());
                    continue;
                }
                _ => {}
            }
            if let Head::Bool(value) = head {
                if matches!(pat, Pattern::Guard { .. } | Pattern::Observe { .. }) {
                    if crate::interval_coverage::boolean_case(pat, value, &mut self.steps)?
                        == Some(true)
                    {
                        self.spend(tail.len().saturating_add(1))?;
                        out.push(tail.to_vec());
                    }
                    continue;
                }
            }
            let prefix: Vec<_> = match (pat, head) {
                (Pattern::AnonymousRecord { names, fields }, Head::AnonymousRecord(types))
                    if fields.len() == arity && names.len() == types.len() =>
                {
                    for (name, (expected, _)) in names.iter().zip(types) {
                        self.spend(name.len().saturating_add(expected.len()))?;
                    }
                    if !names
                        .iter()
                        .zip(types)
                        .all(|(name, (expected, _))| name == expected)
                    {
                        continue;
                    }
                    fields.iter().collect()
                }
                (Pattern::Tuple(fields), Head::Tuple) if fields.len() == arity => {
                    fields.iter().collect()
                }
                (
                    Pattern::Record {
                        declaration,
                        fields,
                    },
                    Head::Record(actual),
                ) if *declaration == actual && fields.len() == arity => fields.iter().collect(),
                (Pattern::ErrorResult { label, inner }, Head::ErrorResult(actual)) => {
                    for ty in label.iter().chain(actual) {
                        crate::associated::charge_projection_copy(ty, &mut self.steps)
                            .map_err(|_| ())?;
                    }
                    if label.as_ref() != actual || arity != 1 {
                        continue;
                    }
                    vec![inner.as_ref()]
                }
                (Pattern::Wild, _) => vec![&WILD; arity],
                (Pattern::Unit, Head::Unit) => vec![],
                (Pattern::Literal(Literal::Bool(a)), Head::Bool(b)) if *a == b => vec![],
                (
                    Pattern::Variant {
                        declaration,
                        index,
                        fields,
                    },
                    Head::Variant(d, i),
                ) if *declaration == d && *index == i && fields.len() == arity => {
                    fields.iter().collect()
                }
                _ => continue,
            };
            self.spend(prefix.len() + tail.len() + 1)?;
            let mut row = prefix;
            row.extend_from_slice(tail);
            out.push(row);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{Variant, VariantShape};
    fn boolean(value: bool) -> Pattern {
        Pattern::Literal(Literal::Bool(value))
    }
    fn product(fields: Vec<Pattern>) -> Pattern {
        Pattern::Variant {
            declaration: id(),
            index: 0,
            fields,
        }
    }
    #[test]
    fn anonymous_record_matrix_preserves_labels_and_correlated_fields() {
        let ty = Term::Record(vec![
            ("left".into(), Term::Primitive(Primitive::Bool)),
            ("right".into(), Term::Primitive(Primitive::Bool)),
        ]);
        let pattern = |name: &str, left, right| Pattern::AnonymousRecord {
            names: vec![name.into(), "right".into()],
            fields: vec![left, right],
        };
        let yes = pattern("left", Pattern::Literal(Literal::Bool(true)), Pattern::Wild);
        let no = pattern(
            "left",
            Pattern::Literal(Literal::Bool(false)),
            Pattern::Wild,
        );
        let bad = pattern("wrong", Pattern::Wild, Pattern::Wild);
        let enums = BTreeMap::new();
        assert_eq!(
            check(&enums, &ty, &[&yes, &no], 10_000),
            Coverage::Exhaustive
        );
        assert_eq!(check(&enums, &ty, &[&yes], 10_000), Coverage::Missing);
        assert_eq!(check(&enums, &ty, &[&bad], 10_000), Coverage::Missing);
        let tt = pattern(
            "left",
            Pattern::Literal(Literal::Bool(true)),
            Pattern::Literal(Literal::Bool(true)),
        );
        let ff = pattern(
            "left",
            Pattern::Literal(Literal::Bool(false)),
            Pattern::Literal(Literal::Bool(false)),
        );
        assert_eq!(check(&enums, &ty, &[&tt, &ff], 10_000), Coverage::Missing);
    }

    fn id() -> DeclarationId {
        DeclarationId {
            package: 0,
            local: 1,
        }
    }
    fn setup(fields: Vec<Term>) -> (BTreeMap<DeclarationId, Enum>, Term) {
        (
            BTreeMap::from([(
                id(),
                Enum {
                    parameter_types: vec![],
                    groups: vec![],
                    encoding: None,
                    is_intrinsic: false,
                    symbol_encoding: None,
                    capabilities: Vec::new(),
                    requirements: vec![],
                    type_parameters: vec![],
                    declaration: id(),
                    name: "Product".into(),
                    variants: vec![Variant {
                        name: "pair".into(),
                        shape: VariantShape::Tuple,
                        fields,
                    }],
                },
            )]),
            Term::Nominal {
                declaration: id(),
                arguments: vec![],
            },
        )
    }
    #[test]
    fn matrix_agrees_with_exhaustive_boolean_oracle() {
        // Every subset of a pattern basis, including correlated columns and
        // overlapping wildcards. The independent oracle enumerates four values.
        let basis = [
            (Some(false), Some(false)),
            (Some(false), Some(true)),
            (Some(true), Some(false)),
            (Some(true), Some(true)),
            (None, Some(false)),
            (Some(true), None),
            (None, None),
        ];
        let (enums, ty) = setup(vec![Term::Primitive(Primitive::Bool); 2]);
        for mask in 0..1usize << basis.len() {
            let selected: Vec<_> = basis
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, p)| *p)
                .collect();
            let patterns: Vec<_> = selected
                .iter()
                .map(|(a, b)| {
                    product(vec![
                        a.map(boolean).unwrap_or(Pattern::Wild),
                        b.map(boolean).unwrap_or(Pattern::Wild),
                    ])
                })
                .collect();
            let complete = [false, true].into_iter().all(|a| {
                [false, true].into_iter().all(|b| {
                    selected
                        .iter()
                        .any(|(x, y)| x.is_none_or(|x| x == a) && y.is_none_or(|y| y == b))
                })
            });
            let structs = BTreeMap::from([(
                id(),
                Struct {
                    parameter_types: vec![],
                    capabilities: Vec::new(),
                    requirements: vec![],
                    type_parameters: vec![],
                    declaration: id(),
                    name: "Pair".into(),
                    fields: vec![
                        ("left".into(), Term::Primitive(Primitive::Bool)),
                        ("right".into(), Term::Primitive(Primitive::Bool)),
                    ],
                },
            )]);
            let record_patterns: Vec<_> = selected
                .iter()
                .map(|(a, b)| Pattern::Record {
                    declaration: id(),
                    fields: vec![
                        a.map(boolean).unwrap_or(Pattern::Wild),
                        b.map(boolean).unwrap_or(Pattern::Wild),
                    ],
                })
                .collect();
            assert_eq!(
                check_in_context(
                    &structs,
                    &BTreeMap::new(),
                    &ty,
                    &record_patterns.iter().collect::<Vec<_>>(),
                    100_000,
                    &Context::default()
                ),
                if complete {
                    Coverage::Exhaustive
                } else {
                    Coverage::Missing
                },
                "record mask {mask}"
            );
            let tuple_patterns: Vec<_> = selected
                .iter()
                .map(|(a, b)| {
                    Pattern::Tuple(vec![
                        a.map(boolean).unwrap_or(Pattern::Wild),
                        b.map(boolean).unwrap_or(Pattern::Wild),
                    ])
                })
                .collect();
            assert_eq!(
                check(
                    &BTreeMap::new(),
                    &Term::Tuple(vec![Term::Primitive(Primitive::Bool); 2]),
                    &tuple_patterns.iter().collect::<Vec<_>>(),
                    100_000
                ),
                if complete {
                    Coverage::Exhaustive
                } else {
                    Coverage::Missing
                },
                "tuple mask {mask}"
            );
            assert_eq!(
                check(&enums, &ty, &patterns.iter().collect::<Vec<_>>(), 100_000),
                if complete {
                    Coverage::Exhaustive
                } else {
                    Coverage::Missing
                },
                "mask {mask}"
            );
        }
    }
    #[test]
    fn wide_products_do_not_enumerate_values() {
        let (enums, ty) = setup(vec![Term::Primitive(Primitive::Bool); 64]);
        let full = product(vec![Pattern::Or(vec![boolean(false), boolean(true)]); 64]);
        assert_eq!(check(&enums, &ty, &[&full], 100_000), Coverage::Exhaustive);
        let mut partial = vec![Pattern::Wild; 64];
        partial[63] = boolean(true);
        assert_eq!(
            check(&enums, &ty, &[&product(partial)], 100_000),
            Coverage::Missing
        );
    }
    #[test]
    fn exhaustion_is_not_a_counterexample_or_a_proof() {
        let enums = BTreeMap::new();
        let ty = Term::Primitive(Primitive::Bool);
        assert_eq!(check(&enums, &ty, &[&Pattern::Wild], 0), Coverage::Unknown);
        assert_eq!(
            check(&enums, &ty, &[&boolean(true)], 100),
            Coverage::Missing
        );
        assert_eq!(
            check(&enums, &ty, &[&boolean(true), &boolean(false)], 100),
            Coverage::Exhaustive
        );
    }
    #[test]
    fn empty_payloads_and_open_literal_domains_are_distinct() {
        let (enums, ty) = setup(vec![Term::Primitive(Primitive::Never)]);
        assert_eq!(check(&enums, &ty, &[], 100), Coverage::Exhaustive);
        let (enums, ty) = setup(vec![Term::Primitive(Primitive::Char)]);
        assert_eq!(
            check(
                &enums,
                &ty,
                &[&product(vec![Pattern::Literal(Literal::Char('x'))])],
                100_000
            ),
            Coverage::Missing
        );
        assert_eq!(
            check(&enums, &ty, &[&product(vec![Pattern::Wild])], 100),
            Coverage::Exhaustive
        );
    }
}

#[cfg(test)]
mod contextual_tests {
    use super::*;
    use nir::Sort;
    fn variable() -> Term {
        Term::Rigid {
            declaration: DeclarationId {
                package: 0,
                local: 99,
            },
            sort: Sort::Type,
        }
    }
    #[test]
    fn equality_domains_are_scoped_and_do_not_unfold_opaque_heads() {
        let enums = BTreeMap::new();
        let yes = Pattern::Literal(Literal::Bool(true));
        let no = Pattern::Literal(Literal::Bool(false));
        let patterns = [&yes, &no];
        let opaque = Term::NormalForm {
            head: DeclarationId {
                package: 0,
                local: 1,
            },
            sort: Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Bool)],
        };
        for ty in [variable(), opaque] {
            let mut context = Context::default();
            assert_eq!(
                check_in_context(&BTreeMap::new(), &enums, &ty, &patterns, 100_000, &context),
                Coverage::Missing
            );
            context.assume(Formula::equal(ty.clone(), Term::Primitive(Primitive::Bool)));
            assert_eq!(
                check_in_context(&BTreeMap::new(), &enums, &ty, &patterns, 100_000, &context),
                Coverage::Exhaustive
            );
            assert_eq!(
                check_in_context(&BTreeMap::new(), &enums, &ty, &[&yes], 100_000, &context),
                Coverage::Missing
            );
            assert_eq!(
                check_in_context(&BTreeMap::new(), &enums, &ty, &patterns, 100, &context),
                Coverage::Unknown
            );
            assert_eq!(
                check_in_context(
                    &BTreeMap::new(),
                    &enums,
                    &ty,
                    &patterns,
                    100_000,
                    &Context::default()
                ),
                Coverage::Missing
            );
        }
    }
    #[test]
    fn disjunctive_information_does_not_choose_a_runtime_domain() {
        let mut context = Context::default();
        context.assume(Formula::Or(vec![
            Formula::equal(variable(), Term::Primitive(Primitive::Bool)),
            Formula::equal(variable(), Term::Primitive(Primitive::Char)),
        ]));
        let yes = Pattern::Literal(Literal::Bool(true));
        let no = Pattern::Literal(Literal::Bool(false));
        assert_eq!(
            check_in_context(
                &BTreeMap::new(),
                &BTreeMap::new(),
                &variable(),
                &[&yes, &no],
                100_000,
                &context
            ),
            Coverage::Missing
        );
    }
    #[test]
    fn empty_and_unit_domains_require_evidence() {
        let mut context = Context::default();
        context.assume(Formula::equal(
            variable(),
            Term::Primitive(Primitive::Never),
        ));
        assert_eq!(
            check_in_context(
                &BTreeMap::new(),
                &BTreeMap::new(),
                &variable(),
                &[],
                100_000,
                &context
            ),
            Coverage::Exhaustive
        );
        let mut context = Context::default();
        context.assume(Formula::equal(variable(), Term::Primitive(Primitive::Void)));
        assert_eq!(
            check_in_context(
                &BTreeMap::new(),
                &BTreeMap::new(),
                &variable(),
                &[],
                100_000,
                &context
            ),
            Coverage::Missing
        );
        assert_eq!(
            check_in_context(
                &BTreeMap::new(),
                &BTreeMap::new(),
                &variable(),
                &[&Pattern::Unit],
                100_000,
                &context
            ),
            Coverage::Exhaustive
        );
    }
}

#[cfg(test)]
mod sequential_budget_tests {
    use super::*;
    use nir::program::{Expr, ExprKind, SourceRange};
    fn next(first: Pattern, next: Pattern) -> Pattern {
        Pattern::Then {
            first: Box::new(first),
            value: Box::new(Expr {
                ty: Term::Primitive(Primitive::Bool),
                source: SourceRange { start: 0, end: 0 },
                kind: ExprKind::Literal(Literal::Bool(true)),
            }),
            next: Box::new(next),
        }
    }
    #[test]
    fn long_sequential_wildcards_use_bounded_iterative_work() {
        let mut pattern = Pattern::Wild;
        for _ in 0..1024 {
            pattern = next(Pattern::Wild, pattern);
        }
        let ty = Term::Primitive(Primitive::Bool);
        assert_eq!(
            check(&BTreeMap::new(), &ty, &[&pattern], 16),
            Coverage::Unknown
        );
        assert_eq!(
            check(&BTreeMap::new(), &ty, &[&pattern], 100_000),
            Coverage::Exhaustive
        );
        let partial = next(pattern, Pattern::Literal(Literal::Bool(true)));
        assert_eq!(
            check(&BTreeMap::new(), &ty, &[&partial], 100_000),
            Coverage::Exhaustive
        );
        let rejected = next(partial, Pattern::Literal(Literal::Bool(false)));
        assert_eq!(
            check(&BTreeMap::new(), &ty, &[&rejected], 100_000),
            Coverage::Missing
        );
    }
}

#[cfg(test)]
mod bit_tests {
    use super::*;
    use nir::program::{Expr, ExprKind, LocalId, SourceRange};
    fn expression(ty: Term, kind: ExprKind) -> Expr {
        Expr {
            ty,
            kind,
            source: SourceRange { start: 0, end: 0 },
        }
    }
    fn mask(width: usize, bits: u64, value: u64) -> Pattern {
        let binding = LocalId { owner: 0, index: 0 };
        let mut pattern = Pattern::Binding {
            binding,
            inner: Box::new(Pattern::Wild),
        };
        for bit in 0..width {
            if bits & (1 << bit) == 0 {
                continue;
            }
            let slice = expression(
                Term::Primitive(Primitive::BitVec(1)),
                ExprKind::BitSlice {
                    value: Box::new(expression(
                        Term::Primitive(Primitive::BitVec(width)),
                        ExprKind::Local(binding),
                    )),
                    start: bit,
                    end: bit + 1,
                },
            );
            pattern = Pattern::Then {
                first: Box::new(pattern),
                value: Box::new(slice),
                next: Box::new(Pattern::Literal(Literal::BitVec {
                    width: 1,
                    words: vec![((value >> bit) & 1) as u32],
                })),
            };
        }
        pattern
    }
    #[test]
    fn symbolic_bit_masks_agree_with_exhaustive_small_value_oracle() {
        let mut random = 0x123456789abcdefu64;
        for case in 0..1200 {
            let width = case % 7;
            let domain = (1u64 << width) - 1;
            let mut cubes = Vec::new();
            for _ in 0..case % 9 {
                random = random
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                cubes.push(((random >> 24) & domain, (random >> 40) & domain));
            }
            let complete = (0..=domain).all(|input| {
                cubes
                    .iter()
                    .any(|(mask, value)| input & mask == value & mask)
            });
            let patterns: Vec<_> = cubes
                .iter()
                .map(|(bits, value)| mask(width, *bits, *value))
                .collect();
            let refs: Vec<_> = patterns.iter().collect();
            assert_eq!(
                check(
                    &BTreeMap::new(),
                    &Term::Primitive(Primitive::BitVec(width)),
                    &refs,
                    100_000
                ),
                if complete {
                    Coverage::Exhaustive
                } else {
                    Coverage::Missing
                },
                "width={width}, cubes={cubes:?}"
            );
        }
    }
    #[test]
    fn typed_unsigned_segments_cover_exactly_their_values() {
        let binding = LocalId { owner: 1, index: 0 };
        let patterns: Vec<_> = (0..256)
            .map(|value| Pattern::Then {
                first: Box::new(Pattern::Binding {
                    binding,
                    inner: Box::new(Pattern::Wild),
                }),
                value: Box::new(expression(
                    Term::Primitive(Primitive::U8),
                    ExprKind::BitConvert(Box::new(expression(
                        Term::Primitive(Primitive::BitVec(8)),
                        ExprKind::Local(binding),
                    ))),
                )),
                next: Box::new(Pattern::Literal(Literal::UnsignedInteger(value))),
            })
            .collect();
        let mut refs: Vec<_> = patterns.iter().collect();
        let ty = Term::Primitive(Primitive::BitVec(8));
        assert_eq!(
            check(&BTreeMap::new(), &ty, &refs, 100_000),
            Coverage::Exhaustive
        );
        refs.remove(137);
        assert_eq!(
            check(&BTreeMap::new(), &ty, &refs, 100_000),
            Coverage::Missing
        );
    }
    #[test]
    fn typed_signed_segments_cover_exactly_their_values() {
        let binding = LocalId { owner: 1, index: 0 };
        let patterns: Vec<_> = (-128..128)
            .map(|value| Pattern::Then {
                first: Box::new(Pattern::Binding {
                    binding,
                    inner: Box::new(Pattern::Wild),
                }),
                value: Box::new(expression(
                    Term::Primitive(Primitive::I8),
                    ExprKind::BitConvert(Box::new(expression(
                        Term::Primitive(Primitive::BitVec(8)),
                        ExprKind::Local(binding),
                    ))),
                )),
                next: Box::new(Pattern::Literal(Literal::SignedInteger(value))),
            })
            .collect();
        let mut refs: Vec<_> = patterns.iter().collect();
        let ty = Term::Primitive(Primitive::BitVec(8));
        assert_eq!(
            check(&BTreeMap::new(), &ty, &refs, 100_000),
            Coverage::Exhaustive
        );
        refs.remove(137);
        assert_eq!(
            check(&BTreeMap::new(), &ty, &refs, 100_000),
            Coverage::Missing
        );
    }
    #[test]
    fn sparse_masks_cover_large_widths_without_enumerating_values() {
        let width = 65536;
        let binding = LocalId { owner: 2, index: 0 };
        let patterns: Vec<_> = (0..2)
            .map(|value| Pattern::Then {
                first: Box::new(Pattern::Binding {
                    binding,
                    inner: Box::new(Pattern::Wild),
                }),
                value: Box::new(expression(
                    Term::Primitive(Primitive::BitVec(1)),
                    ExprKind::BitSlice {
                        value: Box::new(expression(
                            Term::Primitive(Primitive::BitVec(width)),
                            ExprKind::Local(binding),
                        )),
                        start: width - 1,
                        end: width,
                    },
                )),
                next: Box::new(Pattern::Literal(Literal::BitVec {
                    width: 1,
                    words: vec![value],
                })),
            })
            .collect();
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::BitVec(width)),
                &patterns.iter().collect::<Vec<_>>(),
                200
            ),
            Coverage::Exhaustive
        );
    }
    #[test]
    fn bit_coverage_exhaustion_is_unknown_and_does_not_poison_later_checks() {
        let ty = Term::Primitive(Primitive::BitVec(1024));
        let pattern = Pattern::Literal(Literal::BitVec {
            width: 1024,
            words: vec![0; 32],
        });
        assert_eq!(
            check(&BTreeMap::new(), &ty, &[&pattern], 200),
            Coverage::Unknown
        );
        assert_eq!(
            check(&BTreeMap::new(), &ty, &[&pattern], 100_000),
            Coverage::Missing
        );
        assert_eq!(
            check(&BTreeMap::new(), &ty, &[&Pattern::Wild], 100_000),
            Coverage::Exhaustive
        );
    }
}

#[cfg(test)]
mod exact_float_tests {
    use super::*;
    use nir::program::{Expr, ExprKind as K, LocalId, MatchArm, SourceRange};
    fn lit(text: &str) -> Pattern {
        Pattern::Literal(Literal::Float(text.into()))
    }
    fn negate(pattern: Pattern) -> Pattern {
        let binding = LocalId { owner: 1, index: 0 };
        let boolean = |value| Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(value)),
        };
        Pattern::Guard {
            pattern: Box::new(Pattern::Binding {
                binding,
                inner: Box::new(Pattern::Wild),
            }),
            condition: Box::new(Expr {
                source: boolean(true).source,
                ty: boolean(true).ty,
                kind: K::Match {
                    value: Box::new(Expr {
                        source: boolean(true).source,
                        ty: Term::Primitive(Primitive::Float),
                        kind: K::Local(binding),
                    }),
                    arms: vec![
                        MatchArm {
                            pattern,
                            guard: None,
                            body: boolean(false),
                        },
                        MatchArm {
                            pattern: Pattern::Wild,
                            guard: None,
                            body: boolean(true),
                        },
                    ],
                },
            }),
        }
    }
    #[test]
    fn singleton_partitions_agree_with_independent_three_cell_oracle() {
        let patterns = [
            (lit("0.0"), 1u8),
            (lit("1.0"), 2),
            (negate(lit("-0.0")), 6),
            (negate(lit("10.0e-1")), 5),
            (Pattern::Or(vec![lit("0.0"), lit("1.0")]), 3),
            (negate(Pattern::Or(vec![lit("0.0"), lit("1.0")])), 4),
        ];
        for subset in 0..64 {
            let mut selected = Vec::new();
            let mut covered = 0;
            for (index, (pattern, mask)) in patterns.iter().enumerate() {
                if subset & (1 << index) != 0 {
                    selected.push(pattern);
                    covered |= mask;
                }
            }
            let result = check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Float),
                &selected,
                100_000,
            );
            assert_eq!(
                result,
                if covered == 7 {
                    Coverage::Exhaustive
                } else {
                    Coverage::Missing
                },
                "subset={subset}"
            );
        }
        let complete = [negate(lit("0.0")), lit("-0.0")];
        let refs = complete.iter().collect::<Vec<_>>();
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Float),
                &refs,
                0
            ),
            Coverage::Unknown
        );
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Float),
                &refs,
                100_000
            ),
            Coverage::Exhaustive
        );
    }
    #[test]
    fn float_complements_require_the_same_subject_and_exact_numeric_identity() {
        let excluded = negate(lit("1.0000000000000000000000000000001"));
        let adjacent = lit("1.0000000000000000000000000000002");
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Float),
                &[&excluded, &adjacent],
                100_000
            ),
            Coverage::Missing
        );
        let mut wrong_subject = negate(lit("0.0"));
        let Pattern::Guard { condition, .. } = &mut wrong_subject else {
            unreachable!()
        };
        let K::Match { value, .. } = &mut condition.kind else {
            unreachable!()
        };
        value.kind = K::Local(LocalId {
            owner: 99,
            index: 99,
        });
        assert_eq!(
            check(
                &BTreeMap::new(),
                &Term::Primitive(Primitive::Float),
                &[&wrong_subject, &lit("0.0")],
                100_000
            ),
            Coverage::Missing
        );
    }
}

#[cfg(test)]
mod computed_structure_tests {
    use super::*;
    use nir::program::{Expr, ExprKind, SourceRange};

    fn computed(first: Pattern, ty: Term, next: Pattern) -> Pattern {
        Pattern::Then {
            first: Box::new(first),
            value: Box::new(Expr {
                source: SourceRange { start: 0, end: 0 },
                ty,
                // Coverage uses the result type, never evaluates this body.
                kind: ExprKind::Unit,
            }),
            next: Box::new(next),
        }
    }

    #[test]
    fn computed_suffix_uses_its_result_domain_and_preserves_first_restriction() {
        let bool_ty = Term::Primitive(Primitive::Bool);
        let tuple_ty = Term::Tuple(vec![bool_ty.clone(), bool_ty.clone()]);
        let total = computed(
            Pattern::Wild,
            tuple_ty.clone(),
            Pattern::Tuple(vec![Pattern::Wild, Pattern::Wild]),
        );
        assert_eq!(
            check(&BTreeMap::new(), &bool_ty, &[&total], 100_000),
            Coverage::Exhaustive
        );
        for subject in [
            Term::Primitive(Primitive::U32),
            Term::Primitive(Primitive::Float),
            Term::Primitive(Primitive::BitVec(8)),
            tuple_ty.clone(),
        ] {
            assert_eq!(
                check(&BTreeMap::new(), &subject, &[&total], 100_000),
                Coverage::Exhaustive,
                "{subject:?}"
            );
        }
        let partial = computed(
            Pattern::Literal(Literal::Bool(true)),
            tuple_ty.clone(),
            Pattern::Tuple(vec![Pattern::Wild, Pattern::Wild]),
        );
        assert_eq!(
            check(&BTreeMap::new(), &bool_ty, &[&partial], 100_000),
            Coverage::Missing
        );
        let refutable = computed(
            Pattern::Wild,
            tuple_ty,
            Pattern::Tuple(vec![Pattern::Literal(Literal::Bool(true)), Pattern::Wild]),
        );
        assert_eq!(
            check(&BTreeMap::new(), &bool_ty, &[&refutable], 100_000),
            Coverage::Missing
        );
        assert_eq!(
            check(&BTreeMap::new(), &bool_ty, &[&total], 2),
            Coverage::Unknown
        );
    }

    #[test]
    fn computed_suffix_can_cover_all_variants_without_erasing_guards() {
        let bool_ty = Term::Primitive(Primitive::Bool);
        let all = Pattern::Or(vec![
            Pattern::Literal(Literal::Bool(true)),
            Pattern::Literal(Literal::Bool(false)),
        ]);
        let total = computed(Pattern::Wild, bool_ty.clone(), all);
        assert_eq!(
            check(&BTreeMap::new(), &bool_ty, &[&total], 100_000),
            Coverage::Exhaustive
        );
        let guard = Pattern::Guard {
            pattern: Box::new(Pattern::Wild),
            condition: Box::new(Expr {
                source: SourceRange { start: 0, end: 0 },
                ty: bool_ty.clone(),
                kind: ExprKind::Literal(Literal::Bool(false)),
            }),
        };
        let guarded = computed(Pattern::Wild, bool_ty.clone(), guard);
        assert_eq!(
            check(&BTreeMap::new(), &bool_ty, &[&guarded], 100_000),
            Coverage::Missing
        );
    }
}

#[test]
fn abstract_pattern_code_is_not_coverage_evidence() {
    let pattern = Pattern::CodeEvidence { outputs: vec![] };
    assert_ne!(
        check(
            &BTreeMap::new(),
            &Term::Primitive(Primitive::Bool),
            &[&pattern],
            1000
        ),
        Coverage::Exhaustive
    );
}

#[test]
fn error_result_coverage_matches_the_finite_branch_payload_oracle() {
    let boolean = Term::Primitive(Primitive::Bool);
    let ty = Term::error_result(vec![boolean.clone()], boolean.clone());
    let patterns: Vec<_> = (0..4)
        .map(|index| Pattern::ErrorResult {
            label: (index >= 2).then(|| boolean.clone()),
            inner: Box::new(Pattern::Literal(Literal::Bool(index % 2 == 1))),
        })
        .collect();
    for mask in 0..16 {
        let selected: Vec<_> = patterns
            .iter()
            .enumerate()
            .filter_map(|(i, p)| (mask & (1 << i) != 0).then_some(p))
            .collect();
        assert_eq!(
            check(&BTreeMap::new(), &ty, &selected, 100_000),
            if mask == 15 {
                Coverage::Exhaustive
            } else {
                Coverage::Missing
            },
            "mask {mask}"
        );
    }
    let all: Vec<_> = patterns.iter().collect();
    assert_eq!(check(&BTreeMap::new(), &ty, &all, 0), Coverage::Unknown);
    assert_eq!(
        check(&BTreeMap::new(), &ty, &all, 100_000),
        Coverage::Exhaustive
    );
    let no_success = Term::error_result(vec![boolean], Term::Primitive(Primitive::Never));
    assert_eq!(
        check(
            &BTreeMap::new(),
            &no_success,
            &[&patterns[2], &patterns[3]],
            100_000
        ),
        Coverage::Exhaustive
    );
}

#[test]
fn metered_error_coverage_charges_failed_work_and_can_be_retried() {
    let ty = Term::error_result(
        vec![Term::Primitive(Primitive::Bool)],
        Term::Primitive(Primitive::Bool),
    );
    let patterns = [
        Pattern::ErrorResult {
            label: None,
            inner: Box::new(Pattern::Wild),
        },
        Pattern::ErrorResult {
            label: Some(Term::Primitive(Primitive::Bool)),
            inner: Box::new(Pattern::Wild),
        },
    ];
    let refs: Vec<_> = patterns.iter().collect();
    let structs = BTreeMap::new();
    let enums = BTreeMap::new();
    let context = Context::default();
    assert_eq!(
        check_metered(&structs, &enums, &ty, &refs, 1, &context, None),
        (Coverage::Unknown, 1)
    );
    let (outcome, used) = check_metered(&structs, &enums, &ty, &refs, 100_000, &context, None);
    assert_eq!(outcome, Coverage::Exhaustive);
    assert!(used > 1 && used < 100_000);
}
