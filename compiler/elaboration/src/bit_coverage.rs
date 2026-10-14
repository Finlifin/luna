//! Symbolic coverage constraints for already type-checked bit projections.
//! No code is executed; unfamiliar computations/guards contribute no coverage.
use crate::ptc::{Budget, Context};
use nir::program::{Expr, ExprKind, Literal, LocalId, Pattern};
use nir::{Primitive, Term};
use std::collections::{BTreeMap, HashMap};

pub(crate) type Cube = BTreeMap<usize, bool>;
#[derive(Clone, Copy)]
struct View {
    start: usize,
    width: usize,
    scalar: Option<Primitive>,
}
#[derive(Clone, Default)]
struct State {
    cube: Cube,
    bindings: BTreeMap<LocalId, View>,
}
fn spend(steps: &mut usize, count: usize) -> Result<(), ()> {
    *steps = steps.checked_sub(count).ok_or(())?;
    Ok(())
}
fn copy(state: &State, steps: &mut usize) -> Result<State, ()> {
    spend(
        steps,
        state
            .cube
            .len()
            .saturating_add(state.bindings.len())
            .saturating_add(1),
    )?;
    Ok(state.clone())
}
// Subtract a conjunction of fixed bits, splitting only on constraints absent
// from the current region. This does not enumerate the value space.
fn subtract(mut state: State, excluded: &Cube, steps: &mut usize) -> Result<Vec<State>, ()> {
    for (bit, value) in excluded {
        spend(steps, 1)?;
        if state.cube.get(bit).is_some_and(|current| current != value) {
            return Ok(vec![state]);
        }
    }
    let mut out = Vec::new();
    for (bit, value) in excluded {
        spend(steps, 1)?;
        if !state.cube.contains_key(bit) {
            let mut outside = copy(&state, steps)?;
            outside.cube.insert(*bit, !value);
            out.push(outside);
            state.cube.insert(*bit, *value);
        }
    }
    Ok(out)
}
fn outside_regions(state: &State, regions: &[Cube], steps: &mut usize) -> Result<Vec<State>, ()> {
    let mut outside = vec![copy(state, steps)?];
    for cube in regions {
        let mut next = Vec::new();
        for region in outside {
            next.extend(subtract(region, cube, steps)?);
        }
        outside = next;
    }
    Ok(outside)
}

pub(crate) struct Analysis<'a> {
    context: &'a Context,
    domains: HashMap<Term, Option<Primitive>>,
}
impl<'a> Analysis<'a> {
    pub(crate) fn new(context: &'a Context) -> Self {
        Self {
            context,
            domains: HashMap::new(),
        }
    }
    // Only scoped type equality is queried here; runtime conditions stay in
    // the coverage analyzer. No source type or normal-form head is rewritten.
    fn primitive(&mut self, ty: &Term, steps: &mut usize) -> Result<Option<Primitive>, ()> {
        if let Term::Primitive(primitive) = ty {
            return Ok(Some(*primitive));
        }
        crate::associated::charge_projection_copy(ty, steps).map_err(|_| ())?;
        if let Some(domain) = self.domains.get(ty) {
            return Ok(*domain);
        }
        let (result, used) = self.context.structural_view_metered(
            ty,
            Budget {
                steps: (*steps / 2).min(10_000),
                max_depth: 128,
            },
        );
        spend(steps, used)?;
        let domain = match result {
            Ok(Some((Term::Primitive(primitive), _))) => Some(primitive),
            Ok(_) | Err(crate::ptc::UnknownReason::UnvalidatedModel) => None,
            Err(_) => return Err(()),
        };
        crate::associated::charge_projection_copy(ty, steps).map_err(|_| ())?;
        self.domains.insert(ty.clone(), domain);
        Ok(domain)
    }
    fn projection(
        &mut self,
        expr: &Expr,
        state: &State,
        steps: &mut usize,
        depth: usize,
    ) -> Result<Option<View>, ()> {
        spend(steps, 1)?;
        if depth > 128 {
            return Err(());
        }
        let Some(ty) = self.primitive(&expr.ty, steps)? else {
            return Ok(None);
        };
        let view = match &expr.kind {
            ExprKind::Local(id) => state.bindings.get(id).copied(),
            ExprKind::BitSlice { value, start, end } => {
                let Some(base) = self.projection(value, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                if base.scalar.is_some() || start > end || *end > base.width {
                    return Ok(None);
                }
                Some(View {
                    start: base.start + start,
                    width: end - start,
                    scalar: None,
                })
            }
            ExprKind::BitConvert(value) => {
                let Some(mut base) = self.projection(value, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                match ty {
                    Primitive::BitVec(width) if base.scalar.is_none() && width == base.width => {
                        Some(base)
                    }
                    primitive
                        if base.scalar.is_none()
                            && primitive.unrestricted_scalar_width().map(|w| w as usize)
                                == Some(base.width)
                            && base.width <= 128 =>
                    {
                        base.scalar = Some(primitive);
                        Some(base)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        // Do not infer a representation by unfolding an opaque type head.
        Ok(view.filter(|view| match ty {
            Primitive::BitVec(width) => view.scalar.is_none() && width == view.width,
            primitive => {
                view.scalar == Some(primitive)
                    && primitive.unrestricted_scalar_width().map(|w| w as usize) == Some(view.width)
                    && view.width <= 128
            }
        }))
    }
    fn condition(
        &mut self,
        expr: &Expr,
        state: &State,
        steps: &mut usize,
        depth: usize,
    ) -> Result<Option<Vec<Cube>>, ()> {
        spend(steps, 1)?;
        if depth > 128 {
            return Err(());
        }
        match &expr.kind {
            ExprKind::Block {
                statements,
                result: Some(result),
            } => {
                let mut scoped = copy(state, steps)?;
                for statement in statements {
                    spend(steps, 1)?;
                    let ExprKind::Let { binding, value } = &statement.kind else {
                        return Ok(None);
                    };
                    let Some(view) = self.projection(value, &scoped, steps, depth + 1)? else {
                        return Ok(None);
                    };
                    scoped.bindings.insert(*binding, view);
                }
                return self.condition(result, &scoped, steps, depth + 1);
            }
            ExprKind::If {
                condition: test,
                then_branch,
                else_branch: Some(else_branch),
            } => {
                let Some(selected) = self.condition(test, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                let rejected = outside_regions(state, &selected, steps)?;
                let mut truth = Vec::new();
                for cube in selected {
                    let mut region = copy(state, steps)?;
                    region.cube = cube;
                    let Some(regions) = self.condition(then_branch, &region, steps, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    spend(steps, regions.len())?;
                    truth.extend(regions);
                }
                for region in rejected {
                    let Some(regions) = self.condition(else_branch, &region, steps, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    spend(steps, regions.len())?;
                    truth.extend(regions);
                }
                return Ok(Some(truth));
            }
            ExprKind::Not(inner) => {
                let Some(regions) = self.condition(inner, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                return Ok(Some(
                    outside_regions(state, &regions, steps)?
                        .into_iter()
                        .map(|s| s.cube)
                        .collect(),
                ));
            }
            ExprKind::And(left, right) | ExprKind::Or(left, right) => {
                let Some(left) = self.condition(left, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                let (mut truth, pending) = if matches!(expr.kind, ExprKind::Or(..)) {
                    let pending = outside_regions(state, &left, steps)?;
                    (left, pending)
                } else {
                    let mut pending = Vec::new();
                    for cube in left {
                        let mut region = copy(state, steps)?;
                        region.cube = cube;
                        pending.push(region);
                    }
                    (Vec::new(), pending)
                };
                for region in pending {
                    let Some(right) = self.condition(right, &region, steps, depth + 1)? else {
                        return Ok(None);
                    };
                    spend(steps, right.len())?;
                    truth.extend(right);
                }
                return Ok(Some(truth));
            }
            _ => {}
        }
        if let ExprKind::Literal(Literal::Bool(result)) = expr.kind {
            spend(steps, state.cube.len())?;
            return Ok(Some(if result {
                vec![state.cube.clone()]
            } else {
                vec![]
            }));
        }
        let ExprKind::Match { value, arms } = &expr.kind else {
            return Ok(None);
        };
        let Some(view) = self.projection(value, state, steps, depth + 1)? else {
            return Ok(None);
        };
        let mut remaining = vec![copy(state, steps)?];
        let mut truth = Vec::new();
        for arm in arms {
            if remaining.is_empty() {
                break;
            }
            if arm.guard.is_some() {
                return Ok(None);
            }
            let mut leftovers = Vec::new();
            for region in remaining {
                let mut complete = true;
                let matched = self.pattern(
                    &arm.pattern,
                    view,
                    copy(&region, steps)?,
                    steps,
                    depth + 1,
                    &mut complete,
                )?;
                if !complete {
                    return Ok(None);
                }
                let mut outside = vec![region];
                for selected in &matched {
                    let mut next = Vec::new();
                    for region in outside {
                        next.extend(subtract(region, &selected.cube, steps)?);
                    }
                    outside = next;
                }
                spend(steps, outside.len().saturating_add(matched.len()))?;
                leftovers.extend(outside);
                for selected in matched {
                    let Some(regions) = self.condition(&arm.body, &selected, steps, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    spend(steps, regions.len())?;
                    truth.extend(regions);
                }
            }
            remaining = leftovers;
        }
        Ok(remaining.is_empty().then_some(truth))
    }

    fn pattern(
        &mut self,
        pat: &Pattern,
        view: View,
        mut state: State,
        steps: &mut usize,
        depth: usize,
        complete: &mut bool,
    ) -> Result<Vec<State>, ()> {
        spend(steps, 1)?;
        if depth > 128 {
            return Err(());
        }
        match pat {
            Pattern::Wild => Ok(vec![state]),
            Pattern::Binding { binding, inner } => {
                let mut matched = self.pattern(inner, view, state, steps, depth + 1, complete)?;
                for state in &mut matched {
                    spend(steps, 1)?;
                    state.bindings.insert(*binding, view);
                }
                Ok(matched)
            }
            Pattern::Remap {
                pattern: inner,
                bindings,
            } => {
                let mut states = self.pattern(inner, view, state, steps, depth + 1, complete)?;
                for state in &mut states {
                    spend(steps, bindings.len().saturating_mul(2))?;
                    let remapped: Vec<_> = bindings
                        .iter()
                        .filter_map(|(from, to)| match state.bindings.get(from) {
                            Some(v) => Some((*to, *v)),
                            None => {
                                *complete = false;
                                None
                            }
                        })
                        .collect();
                    let targets: std::collections::BTreeSet<_> =
                        remapped.iter().map(|(id, _)| *id).collect();
                    state.bindings.extend(remapped);
                    for (from, _) in bindings {
                        spend(steps, 1)?;
                        if !targets.contains(from) {
                            state.bindings.remove(from);
                        }
                    }
                }
                Ok(states)
            }
            Pattern::Or(alternatives) => {
                let mut result = Vec::new();
                let mut remaining = vec![state];
                for alternative in alternatives {
                    if remaining.is_empty() {
                        break;
                    }
                    let mut leftovers = Vec::new();
                    let mut exact = true;
                    for state in remaining {
                        let mut precise = true;
                        let matched = self.pattern(
                            alternative,
                            view,
                            copy(&state, steps)?,
                            steps,
                            depth + 1,
                            &mut precise,
                        )?;
                        if precise {
                            let mut regions = vec![state];
                            for selected in &matched {
                                let mut next = Vec::new();
                                for region in regions {
                                    next.extend(subtract(region, &selected.cube, steps)?);
                                }
                                regions = next;
                            }
                            spend(steps, regions.len())?;
                            leftovers.extend(regions);
                        } else {
                            exact = false;
                        }
                        spend(steps, matched.len())?;
                        result.extend(matched);
                    }
                    remaining = leftovers;
                    if !exact {
                        *complete = false;
                        break;
                    }
                }
                Ok(result)
            }
            Pattern::Then { first, value, next } => {
                let mut result = Vec::new();
                for state in self.pattern(first, view, state, steps, depth + 1, complete)? {
                    if let Some(next_view) = self.projection(value, &state, steps, 0)? {
                        result.extend(self.pattern(
                            next,
                            next_view,
                            state,
                            steps,
                            depth + 1,
                            complete,
                        )?);
                    } else {
                        *complete = false;
                    }
                }
                Ok(result)
            }
            Pattern::Guard {
                pattern: inner,
                condition: predicate,
            } => {
                let mut result = Vec::new();
                for state in self.pattern(inner, view, state, steps, depth + 1, complete)? {
                    let Some(regions) = self.condition(predicate, &state, steps, depth + 1)? else {
                        *complete = false;
                        continue;
                    };
                    for cube in regions {
                        // Match-local bindings in the predicate never escape it.
                        let mut selected = copy(&state, steps)?;
                        selected.cube = cube;
                        result.push(selected);
                    }
                }
                Ok(result)
            }
            Pattern::Literal(literal) => {
                let words = match literal {
                    Literal::BitVec { width, words }
                        if view.scalar.is_none() && *width == view.width =>
                    {
                        if words.len() != width.div_ceil(32)
                            || (width % 32 != 0
                                && words.last().is_some_and(|word| *word >> (width % 32) != 0))
                        {
                            return Ok(vec![]);
                        }
                        spend(steps, words.len())?;
                        words.clone()
                    }
                    Literal::UnsignedInteger(value)
                        if view.scalar.is_some_and(|p| {
                            p.fixed_integer_layout() == Some((false, view.width as u32))
                        }) && view.width <= 128 =>
                    {
                        if view.width < 128 && *value >> view.width != 0 {
                            return Ok(vec![]);
                        }
                        spend(steps, 4)?;
                        (0..view.width.div_ceil(32))
                            .map(|i| (*value >> (i * 32)) as u32)
                            .collect()
                    }
                    Literal::SignedInteger(value)
                        if view.scalar.is_some_and(|p| {
                            p.fixed_integer_layout() == Some((true, view.width as u32))
                        }) && view.width > 0
                            && view.width <= 128 =>
                    {
                        let shift = 128 - view.width;
                        if (*value << shift) >> shift != *value {
                            return Ok(vec![]);
                        }
                        spend(steps, 4)?;
                        (0..view.width.div_ceil(32))
                            .map(|i| ((*value as u128) >> (i * 32)) as u32)
                            .collect()
                    }
                    // Float equality is not bit equality (signed zero and NaN).
                    // Until represented explicitly, float literals contribute no coverage.
                    _ => {
                        *complete = false;
                        return Ok(vec![]);
                    }
                };
                spend(steps, view.width)?;
                for bit in 0..view.width {
                    let value = words[bit / 32] & (1 << (bit % 32)) != 0;
                    if state
                        .cube
                        .insert(view.start + bit, value)
                        .is_some_and(|old| old != value)
                    {
                        return Ok(vec![]);
                    }
                }
                Ok(vec![state])
            }
            _ => {
                *complete = false;
                Ok(vec![])
            }
        }
    }
    pub(crate) fn extract(
        &mut self,
        pat: &Pattern,
        width: usize,
        steps: &mut usize,
    ) -> Result<Vec<Cube>, ()> {
        let mut complete = true;
        Ok(self
            .pattern(
                pat,
                View {
                    start: 0,
                    width,
                    scalar: None,
                },
                State::default(),
                steps,
                0,
                &mut complete,
            )?
            .into_iter()
            .map(|state| state.cube)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_type_evidence_is_scoped_budgeted_and_keeps_opaque_heads() {
        use nir::program::SourceRange;
        use nir::{DeclarationId, Sort};
        let ty = Term::NormalForm {
            head: DeclarationId {
                package: 3,
                local: 5,
            },
            sort: Sort::Type,
            arguments: vec![Term::Primitive(Primitive::BitVec(2))],
        };
        let id = LocalId { owner: 3, index: 0 };
        let expr = Expr {
            ty: ty.clone(),
            kind: ExprKind::Local(id),
            source: SourceRange { start: 0, end: 0 },
        };
        let state = State {
            cube: Cube::new(),
            bindings: BTreeMap::from([(
                id,
                View {
                    start: 0,
                    width: 2,
                    scalar: None,
                },
            )]),
        };
        let empty = Context::default();
        assert!(
            Analysis::new(&empty)
                .projection(&expr, &state, &mut 100_000, 0)
                .unwrap()
                .is_none()
        );
        let mut context = Context::default();
        context.assume(crate::ptc::Formula::equal(
            ty.clone(),
            Term::Primitive(Primitive::BitVec(2)),
        ));
        let mut analysis = Analysis::new(&context);
        assert!(analysis.projection(&expr, &state, &mut 1, 0).is_err());
        assert!(
            analysis
                .projection(&expr, &state, &mut 100_000, 0)
                .unwrap()
                .is_some()
        );
        // A repeated projection reuses evidence only in this same context.
        assert!(
            analysis
                .projection(&expr, &state, &mut 100, 0)
                .unwrap()
                .is_some()
        );
        assert_eq!(expr.ty, ty);
        assert!(
            Analysis::new(&empty)
                .projection(&expr, &state, &mut 100_000, 0)
                .unwrap()
                .is_none()
        );
        let mut wrong_width = Context::default();
        wrong_width.assume(crate::ptc::Formula::equal(
            ty,
            Term::Primitive(Primitive::BitVec(3)),
        ));
        assert!(
            Analysis::new(&wrong_width)
                .projection(&expr, &state, &mut 100_000, 0)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn boolean_conditions_agree_with_all_two_bit_truth_tables() {
        use nir::program::{MatchArm, SourceRange};
        let subject = LocalId {
            owner: 10,
            index: 0,
        };
        let expr = |ty, kind| Expr {
            ty: Term::Primitive(ty),
            kind,
            source: SourceRange { start: 0, end: 0 },
        };
        let predicate = |mask: u8| {
            expr(
                Primitive::Bool,
                ExprKind::Match {
                    value: Box::new(expr(Primitive::BitVec(2), ExprKind::Local(subject))),
                    arms: (0..4)
                        .map(|value| MatchArm {
                            pattern: Pattern::Literal(Literal::BitVec {
                                width: 2,
                                words: vec![value],
                            }),
                            guard: None,
                            body: expr(
                                Primitive::Bool,
                                ExprKind::Literal(Literal::Bool(mask & (1 << value) != 0)),
                            ),
                        })
                        .collect(),
                },
            )
        };
        let state = State {
            cube: Cube::new(),
            bindings: BTreeMap::from([(
                subject,
                View {
                    start: 0,
                    width: 2,
                    scalar: None,
                },
            )]),
        };
        for left in 0u8..16 {
            for right in 0u8..16 {
                for (kind, expected) in [
                    (
                        ExprKind::And(Box::new(predicate(left)), Box::new(predicate(right))),
                        left & right,
                    ),
                    (
                        ExprKind::Or(Box::new(predicate(left)), Box::new(predicate(right))),
                        left | right,
                    ),
                    (ExprKind::Not(Box::new(predicate(left))), !left & 15),
                    (
                        ExprKind::If {
                            condition: Box::new(predicate(left)),
                            then_branch: Box::new(predicate(right)),
                            else_branch: Some(Box::new(expr(
                                Primitive::Bool,
                                ExprKind::Not(Box::new(predicate(right))),
                            ))),
                        },
                        ((left & right) | (!left & !right)) & 15,
                    ),
                ] {
                    let check = expr(Primitive::Bool, kind);
                    assert!(
                        Analysis::new(&Context::default())
                            .condition(&check, &state, &mut 1, 0)
                            .is_err()
                    );
                    let cubes = Analysis::new(&Context::default())
                        .condition(&check, &state, &mut 100_000, 0)
                        .unwrap()
                        .expect("exact Boolean condition");
                    for value in 0..4 {
                        let count = cubes
                            .iter()
                            .filter(|cube| {
                                cube.iter()
                                    .all(|(bit, set)| ((value >> bit) & 1 != 0) == *set)
                            })
                            .count();
                        assert_eq!(
                            count,
                            usize::from(expected & (1 << value) != 0),
                            "left={left} right={right} value={value}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn match_conditions_keep_candidate_bindings_local_and_recover_budget() {
        use nir::program::{MatchArm, SourceRange};
        let subject = LocalId { owner: 9, index: 0 };
        let private = LocalId { owner: 9, index: 1 };
        let view = View {
            start: 0,
            width: 2,
            scalar: None,
        };
        let expr = |ty, kind| Expr {
            ty: Term::Primitive(ty),
            kind,
            source: SourceRange { start: 0, end: 0 },
        };
        let check = Pattern::Guard {
            pattern: Box::new(Pattern::Wild),
            condition: Box::new(expr(
                Primitive::Bool,
                ExprKind::Match {
                    value: Box::new(expr(Primitive::BitVec(2), ExprKind::Local(subject))),
                    arms: vec![
                        MatchArm {
                            pattern: Pattern::Binding {
                                binding: private,
                                inner: Box::new(Pattern::Literal(Literal::BitVec {
                                    width: 2,
                                    words: vec![0],
                                })),
                            },
                            guard: None,
                            body: expr(Primitive::Bool, ExprKind::Literal(Literal::Bool(false))),
                        },
                        MatchArm {
                            pattern: Pattern::Binding {
                                binding: private,
                                inner: Box::new(Pattern::Wild),
                            },
                            guard: None,
                            body: expr(Primitive::Bool, ExprKind::Literal(Literal::Bool(true))),
                        },
                    ],
                },
            )),
        };
        let state = State {
            cube: Cube::new(),
            bindings: BTreeMap::from([(subject, view)]),
        };
        assert!(
            Analysis::new(&Context::default())
                .pattern(&check, view, state.clone(), &mut 1, 0, &mut true)
                .is_err()
        );
        let mut complete = true;
        let states = Analysis::new(&Context::default())
            .pattern(&check, view, state, &mut 10000, 0, &mut complete)
            .unwrap();
        assert!(complete);
        for state in &states {
            assert!(state.bindings.contains_key(&subject));
            assert!(!state.bindings.contains_key(&private));
        }
        for value in 0..4 {
            let matched = states.iter().any(|s| {
                s.cube
                    .iter()
                    .all(|(bit, expected)| ((value >> bit) & 1 != 0) == *expected)
            });
            assert_eq!(matched, value != 0);
        }
    }

    #[test]
    fn cube_subtraction_agrees_with_small_domain_oracle() {
        let cube = |mut code: usize| {
            let mut cube = Cube::new();
            for bit in 0..3 {
                match code % 3 {
                    1 => {
                        cube.insert(bit, false);
                    }
                    2 => {
                        cube.insert(bit, true);
                    }
                    _ => {}
                }
                code /= 3;
            }
            cube
        };
        let matches = |cube: &Cube, value: usize| {
            cube.iter()
                .all(|(bit, expected)| ((value >> bit) & 1 != 0) == *expected)
        };
        for left in 0..27 {
            for right in 0..27 {
                let input = cube(left);
                let excluded = cube(right);
                let regions = subtract(
                    State {
                        cube: input.clone(),
                        bindings: BTreeMap::new(),
                    },
                    &excluded,
                    &mut 10000,
                )
                .unwrap();
                for value in 0..8 {
                    let count = regions.iter().filter(|s| matches(&s.cube, value)).count();
                    assert_eq!(
                        count,
                        usize::from(matches(&input, value) && !matches(&excluded, value)),
                        "left={left} right={right} value={value}"
                    );
                }
            }
        }
        assert!(subtract(State::default(), &cube(26), &mut 1).is_err());
        assert!(
            !subtract(State::default(), &cube(26), &mut 1000)
                .unwrap()
                .is_empty()
        );
    }
}
