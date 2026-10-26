//! Distinguish an unknown pointer origin from a field absent in this layout.
use super::{EnumDomains, Error, SourceRange, VARIANT_PATH, error};
use nir::{Term, program::Package};

/// Bind symbolic inputs without treating an absent payload as unknown storage.
/// Empty sets are reserved for leaves excluded by established tag evidence;
/// missing physical origins remain missing and therefore unknown to callers.
pub(super) fn bind_input_origins(
    layout: &[crate::verification::memory::PointerInputLeaf],
    origins: &super::ValueOrigins,
    domains: &EnumDomains,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::ValueOrigins, Error> {
    let mut inputs = super::ValueOrigins::new();
    for leaf in layout {
        *fuel = fuel
            .checked_sub(
                leaf.input_fields
                    .len()
                    .saturating_add(leaf.storage_fields.len())
                    .saturating_add(1),
            )
            .ok_or_else(|| error(source, "pointer input binding budget exhausted"))?;
        if super::enum_path_excluded(&leaf.input_fields, domains) {
            inputs.insert(leaf.input_fields.clone(), Default::default());
        } else if let Some(targets) = origins
            .get(&leaf.storage_fields)
            .filter(|targets| !targets.is_empty())
        {
            for target in targets {
                *fuel = fuel
                    .checked_sub(target.fields.len() + 1)
                    .ok_or_else(|| error(source, "pointer input binding budget exhausted"))?;
            }
            inputs.insert(leaf.input_fields.clone(), targets.clone());
        }
    }
    Ok(inputs)
}

pub(super) fn pointer_possible(
    package: Option<&Package>,
    ty: &Term,
    fields: &[usize],
    domains: &EnumDomains,
    qualified: &mut Vec<usize>,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<bool, Error> {
    if depth > 128 {
        return Err(error(source, "pointer layout depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(source, "pointer layout budget exhausted"))?;
    // A closure's captured environment has the same physical field indexing
    // as its ownership shape. Inspect that layout without exporting a memory
    // contract for the closure or unfolding its code.
    let fields_layout = match ty {
        Term::Closure { environment, .. } => Some(environment.as_slice()),
        _ => nir::normal_forms::tuple_fields(package, ty),
    };
    if let Some(types) = fields_layout {
        let Some((&field, rest)) = fields.split_first() else {
            return Ok(false);
        };
        let Some(ty) = types.get(field) else {
            return Ok(false);
        };
        qualified.push(field);
        let result = pointer_possible(
            package,
            ty,
            rest,
            domains,
            qualified,
            fuel,
            source,
            depth + 1,
        );
        qualified.pop();
        return result;
    }
    match ty {
        Term::Pointer(_) => Ok(fields.is_empty()),
        Term::Primitive(_) => Ok(false),
        Term::Record(types) => {
            let Some((&field, rest)) = fields.split_first() else {
                return Ok(false);
            };
            let Some((_, ty)) = types.get(field) else {
                return Ok(false);
            };
            qualified.push(field);
            let result = pointer_possible(
                package,
                ty,
                rest,
                domains,
                qualified,
                fuel,
                source,
                depth + 1,
            );
            qualified.pop();
            result
        }
        Term::Optional(payload) => {
            crate::verification::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            let Some((&0, rest)) = fields.split_first() else {
                return Ok(false);
            };
            if domains
                .get(qualified)
                .is_some_and(|tags| !tags.contains(&1))
            {
                return Ok(false);
            }
            qualified.extend([VARIANT_PATH, 1, 0]);
            let possible = pointer_possible(
                package,
                payload,
                rest,
                domains,
                qualified,
                fuel,
                source,
                depth + 1,
            );
            qualified.truncate(qualified.len() - 3);
            possible
        }
        Term::ErrorResult { errors, result } => {
            crate::verification::memory::bound_type(ty, fuel).map_err(|e| error(source, e))?;
            let Some((&0, rest)) = fields.split_first() else {
                return Ok(false);
            };
            let allowed = domains.get(qualified);
            for (tag, payload) in std::iter::once(result.as_ref())
                .chain(errors.iter())
                .enumerate()
            {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(source, "error pointer layout budget exhausted"))?;
                if allowed.is_some_and(|tags| !tags.contains(&tag)) {
                    continue;
                }
                qualified.extend([VARIANT_PATH, tag, 0]);
                let possible = pointer_possible(
                    package,
                    payload,
                    rest,
                    domains,
                    qualified,
                    fuel,
                    source,
                    depth + 1,
                );
                qualified.truncate(qualified.len() - 3);
                if possible? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Term::Nominal {
            declaration,
            arguments,
        } => {
            let Some((&field, rest)) = fields.split_first() else {
                return Ok(false);
            };
            let Some(package) = package else {
                return Ok(true);
            };
            if let Some(definition) = package
                .structs
                .iter()
                .find(|item| item.declaration == *declaration)
            {
                for (_, ty) in &definition.fields {
                    crate::verification::memory::bound_type(ty, fuel)
                        .map_err(|e| error(source, e))?;
                }
                let types = definition
                    .instantiate_fields(arguments)
                    .map_err(|e| error(source, e.to_string()))?;
                let Some((_, ty)) = types.get(field) else {
                    return Ok(false);
                };
                qualified.push(field);
                let result = pointer_possible(
                    Some(package),
                    ty,
                    rest,
                    domains,
                    qualified,
                    fuel,
                    source,
                    depth + 1,
                );
                qualified.pop();
                return result;
            }
            if let Some(definition) = package
                .newtypes
                .iter()
                .find(|item| item.declaration == *declaration)
            {
                // Ownership represents a newtype's private representation as
                // field zero. This is layout inspection, not source projection.
                if field != 0 {
                    return Ok(false);
                }
                let representation = definition
                    .representation_budgeted(arguments, &[], fuel)
                    .map_err(|e| error(source, e.to_string()))?;
                qualified.push(0);
                let result = pointer_possible(
                    Some(package),
                    &representation,
                    rest,
                    domains,
                    qualified,
                    fuel,
                    source,
                    depth + 1,
                );
                qualified.pop();
                return result;
            }
            if let Some(definition) = package
                .enums
                .iter()
                .find(|item| item.declaration == *declaration)
            {
                for variant in &definition.variants {
                    for ty in &variant.fields {
                        crate::verification::memory::bound_type(ty, fuel)
                            .map_err(|e| error(source, e))?;
                    }
                }
                let variants = definition
                    .instantiate_variants(arguments)
                    .map_err(|e| error(source, e.to_string()))?;
                let allowed = domains.get(qualified);
                for (index, variant) in variants.iter().enumerate() {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or_else(|| error(source, "pointer variant budget exhausted"))?;
                    if allowed.is_some_and(|tags| !tags.contains(&index)) {
                        continue;
                    }
                    let Some(ty) = variant.fields.get(field) else {
                        continue;
                    };
                    qualified.extend([VARIANT_PATH, index, field]);
                    let possible = pointer_possible(
                        Some(package),
                        ty,
                        rest,
                        domains,
                        qualified,
                        fuel,
                        source,
                        depth + 1,
                    );
                    qualified.truncate(qualified.len() - 3);
                    if possible? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            Ok(true)
        }
        // Opaque normal forms, dynamic environments and unresolved descriptions
        // do not prove absence. Never unfold a type-producing function here.
        _ => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditional_input_binding_distinguishes_absence_unknown_and_live_targets() {
        use crate::verification::{PlaceId, memory::PointerInputLeaf, ownership::Place};
        use std::collections::{BTreeMap, BTreeSet};
        let source = SourceRange { start: 1, end: 2 };
        let leaf = |tag, field| PointerInputLeaf {
            input_fields: vec![VARIANT_PATH, tag, field],
            storage_fields: vec![field],
            guards: vec![(vec![], tag)],
            pointee: Term::Primitive(nir::Primitive::Bool),
        };
        let layout = vec![leaf(0, 0), leaf(1, 0), leaf(2, 1)];
        let target = Place {
            root: PlaceId {
                owner: 10,
                local: 1,
            },
            fields: vec![2],
        };
        let origins = BTreeMap::from([(vec![0], BTreeSet::from([target.clone()]))]);
        let selected = BTreeMap::from([(vec![], BTreeSet::from([1]))]);
        assert!(bind_input_origins(&layout, &origins, &selected, &mut 1, source).is_err());
        let result = bind_input_origins(&layout, &origins, &selected, &mut 1000, source).unwrap();
        assert!(result[&layout[0].input_fields].is_empty());
        assert_eq!(
            result[&layout[1].input_fields],
            BTreeSet::from([target.clone()])
        );
        assert!(result[&layout[2].input_fields].is_empty());
        let unknown =
            bind_input_origins(&layout, &origins, &EnumDomains::new(), &mut 1000, source).unwrap();
        assert_eq!(
            unknown[&layout[0].input_fields],
            BTreeSet::from([target.clone()])
        );
        assert_eq!(unknown[&layout[1].input_fields], BTreeSet::from([target]));
        assert!(!unknown.contains_key(&layout[2].input_fields));
        let empty = BTreeMap::from([(vec![0], BTreeSet::new())]);
        let unknown = bind_input_origins(&layout, &empty, &selected, &mut 1000, source).unwrap();
        assert!(!unknown.contains_key(&layout[1].input_fields));
        assert!(unknown[&layout[0].input_fields].is_empty());
        let nested = PointerInputLeaf {
            input_fields: vec![VARIANT_PATH, 1, 0, VARIANT_PATH, 2, 0],
            storage_fields: vec![0, 0],
            guards: vec![(vec![], 1), (vec![0], 2)],
            pointee: Term::Primitive(nir::Primitive::Bool),
        };
        let domains = BTreeMap::from([(vec![], BTreeSet::from([0]))]);
        let absent = bind_input_origins(
            &[nested.clone()],
            &Default::default(),
            &domains,
            &mut 1000,
            source,
        )
        .unwrap();
        assert!(absent[&nested.input_fields].is_empty());
        let domains = BTreeMap::from([(vec![VARIANT_PATH, 1, 0], BTreeSet::from([1]))]);
        let absent = bind_input_origins(
            &[nested.clone()],
            &Default::default(),
            &domains,
            &mut 1000,
            source,
        )
        .unwrap();
        assert!(absent[&nested.input_fields].is_empty());
    }

    #[test]
    fn opaque_types_and_exhausted_analysis_are_not_absence_evidence() {
        let source = SourceRange { start: 1, end: 2 };
        let opaque = Term::NormalForm {
            head: nir::DeclarationId {
                package: 1,
                local: 1,
            },
            sort: nir::Sort::Type,
            arguments: vec![],
        };
        let domains = EnumDomains::new();
        assert!(
            pointer_possible(
                None,
                &opaque,
                &[0],
                &domains,
                &mut vec![],
                &mut 100,
                source,
                0
            )
            .unwrap()
        );
        let scalar = Term::Primitive(nir::Primitive::Bool);
        assert!(
            !pointer_possible(
                None,
                &scalar,
                &[],
                &domains,
                &mut vec![],
                &mut 100,
                source,
                0
            )
            .unwrap()
        );
        assert!(
            pointer_possible(None, &scalar, &[], &domains, &mut vec![], &mut 0, source, 0).is_err()
        );
        assert!(
            pointer_possible(
                None,
                &scalar,
                &[],
                &domains,
                &mut vec![],
                &mut 100,
                source,
                129
            )
            .is_err()
        );
    }
}

/// An observation may learn conditional pointees, but cannot remove existing
/// candidates or weaken their conditions. New candidates need exact read-origin
/// evidence; a synthetic root alone is not permission to alter pointer storage.
pub(super) fn observation_preserves_pointers(
    before: &super::Flow,
    after: &super::Flow,
    report: &super::Report,
    fuel: &mut usize,
    source: SourceRange,
) -> Result<bool, Error> {
    let mut charge = |cost: usize| {
        *fuel = fuel
            .checked_sub(cost)
            .ok_or_else(|| error(source, "observation pointer evidence budget exhausted"))?;
        Ok::<_, Error>(())
    };
    fn conditions<'a>(
        flow: &'a super::Flow,
        slot: &super::Place,
        target: &super::Place,
    ) -> &'a [(Vec<usize>, usize)] {
        flow.pointer_guards
            .get(slot)
            .and_then(|entries| entries.get(target))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    for (slot, targets) in &before.pointers {
        charge(
            slot.fields
                .len()
                .saturating_add(targets.len())
                .saturating_add(1),
        )?;
        let Some(current) = after.pointers.get(slot) else {
            return Ok(false);
        };
        if targets.is_empty() && !current.is_empty() {
            return Ok(false);
        }
        for target in targets {
            charge(target.fields.len().saturating_add(1))?;
            if !current.contains(target) {
                return Ok(false);
            }
            let previous = conditions(before, slot, target);
            let observed = conditions(after, slot, target);
            for (path, _) in previous.iter().chain(observed) {
                charge(path.len().saturating_add(1))?;
            }
            if previous != observed {
                return Ok(false);
            }
        }
    }
    for (slot, targets) in &after.pointers {
        charge(
            slot.fields
                .len()
                .saturating_add(targets.len())
                .saturating_add(1),
        )?;
        if targets.is_empty() && !before.pointers.contains_key(slot) {
            return Ok(false);
        }
        for target in targets {
            charge(target.fields.len().saturating_add(1))?;
            if before
                .pointers
                .get(slot)
                .is_some_and(|old| old.contains(target))
            {
                continue;
            }
            if !target.fields.is_empty() || !report.borrowed_roots.contains_key(&slot.root) {
                return Ok(false);
            }
            let observed = conditions(after, slot, target);
            for (path, _) in observed {
                charge(path.len().saturating_add(1))?;
            }
            match report.borrowed_roots.get(&target.root) {
                Some(super::BorrowedRoot::Indirect { slot: origin, .. }) => {
                    if origin != slot || !observed.is_empty() {
                        return Ok(false);
                    }
                }
                Some(super::BorrowedRoot::GuardedIndirect {
                    base,
                    fields,
                    guards,
                    ..
                }) => {
                    charge(
                        base.fields
                            .len()
                            .saturating_add(fields.len())
                            .saturating_add(1),
                    )?;
                    let mut expected = base.clone();
                    expected
                        .fields
                        .extend(fields.iter().map(|(_, index)| *index));
                    if &expected != slot || observed.len() != guards.len() {
                        return Ok(false);
                    }
                    for ((path, tag), (actual, selected)) in guards.iter().zip(observed) {
                        charge(
                            base.fields
                                .len()
                                .saturating_add(path.len())
                                .saturating_add(1),
                        )?;
                        if tag != selected
                            || !actual
                                .iter()
                                .copied()
                                .eq(base.fields.iter().chain(path).copied())
                        {
                            return Ok(false);
                        }
                    }
                }
                _ => return Ok(false),
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    use crate::verification::{
        PlaceId,
        moves::{BorrowedRoot, Flow, Report},
        ownership::Place,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn new_conditional_knowledge_preserves_existing_targets_and_guards() {
        let source = SourceRange { start: 1, end: 2 };
        let base = Place {
            root: PlaceId {
                owner: u64::MAX - 1,
                local: 0,
            },
            fields: vec![2],
        };
        let slot = Place {
            root: base.root,
            fields: vec![2, 0],
        };
        let left = Place {
            root: PlaceId {
                owner: u64::MAX - 4,
                local: 1,
            },
            fields: vec![],
        };
        let right = Place {
            root: PlaceId {
                owner: u64::MAX - 4,
                local: 2,
            },
            fields: vec![],
        };
        let ty = Term::Primitive(nir::Primitive::Bool);
        let mut report = Report::default();
        report.borrowed_roots.insert(
            base.root,
            BorrowedRoot::Parameter {
                parameter: 0,
                input_fields: vec![],
                pointee: ty.clone(),
            },
        );
        for (target, tag) in [(&left, 0), (&right, 1)] {
            report.borrowed_roots.insert(
                target.root,
                BorrowedRoot::GuardedIndirect {
                    base: base.clone(),
                    fields: vec![(nir::program::FieldOwner::Anonymous, 0)],
                    guards: vec![(vec![], tag)],
                    pointee: ty.clone(),
                },
            );
        }
        let mut before = Flow::default();
        before
            .pointers
            .insert(slot.clone(), BTreeSet::from([left.clone()]));
        before.pointer_guards.insert(
            slot.clone(),
            BTreeMap::from([(left.clone(), vec![(vec![2], 0)])]),
        );
        let mut after = before.clone();
        after.pointers.get_mut(&slot).unwrap().insert(right.clone());
        after
            .pointer_guards
            .get_mut(&slot)
            .unwrap()
            .insert(right.clone(), vec![(vec![2], 1)]);
        let check = |after: &Flow| {
            observation_preserves_pointers(&before, after, &report, &mut 1000, source).unwrap()
        };
        assert!(check(&after));
        assert!(observation_preserves_pointers(&before, &after, &report, &mut 0, source).is_err());
        let mut altered = after.clone();
        altered.pointers.get_mut(&slot).unwrap().remove(&left);
        assert!(!check(&altered));
        let mut altered = after.clone();
        altered.pointer_guards.get_mut(&slot).unwrap().remove(&left);
        assert!(!check(&altered));
        let mut altered = after.clone();
        altered
            .pointer_guards
            .get_mut(&slot)
            .unwrap()
            .insert(right.clone(), vec![(vec![2], 0)]);
        assert!(!check(&altered));
        let mut altered = after.clone();
        altered
            .pointer_guards
            .get_mut(&slot)
            .unwrap()
            .insert(right, vec![(vec![], 1)]);
        assert!(!check(&altered));
    }
}

#[cfg(test)]
mod optional_tests {
    use super::*;

    #[test]
    fn absent_optional_excludes_pointer_but_unknown_presence_does_not() {
        let ty = Term::Optional(Box::new(Term::Pointer(Box::new(Term::Primitive(
            nir::Primitive::Bool,
        )))));
        let source = SourceRange { start: 0, end: 1 };
        for (tags, expected) in [(Some(vec![0]), false), (Some(vec![1]), true), (None, true)] {
            let domains = tags
                .map(|tags| EnumDomains::from([(vec![], tags.into_iter().collect())]))
                .unwrap_or_default();
            let mut path = vec![];
            assert_eq!(
                pointer_possible(None, &ty, &[0], &domains, &mut path, &mut 1000, source, 0)
                    .unwrap(),
                expected
            );
            assert!(path.is_empty());
        }
    }
}
