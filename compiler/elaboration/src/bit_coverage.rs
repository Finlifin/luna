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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bit {
    Position { index: usize, inverted: bool },
    Constant(bool),
}
impl Bit {
    fn inverted(self) -> Self {
        match self {
            Self::Position { index, inverted } => Self::Position {
                index,
                inverted: !inverted,
            },
            Self::Constant(value) => Self::Constant(!value),
        }
    }
}
#[derive(Clone)]
struct BitRegions {
    truth: std::rc::Rc<[Cube]>,
    falsehood: std::rc::Rc<[Cube]>,
}
#[derive(Clone)]
enum Binding {
    View(View),
    Mapped(std::rc::Rc<[Bit]>),
    Relations(std::rc::Rc<[BitRegions]>),
    Bits {
        width: usize,
        words: std::rc::Rc<[u32]>,
    },
    // Immutable truth/false regions are shared across local snapshots. A not
    // operation swaps the regions instead of repeatedly subtracting cubes.
    Predicate {
        truth: std::rc::Rc<[Cube]>,
        falsehood: std::rc::Rc<[Cube]>,
    },
}
impl Binding {
    fn width(&self) -> Option<usize> {
        match self {
            Self::View(View {
                width,
                scalar: None,
                ..
            })
            | Self::Bits { width, .. } => Some(*width),
            Self::Mapped(bits) => Some(bits.len()),
            Self::Relations(bits) => Some(bits.len()),
            _ => None,
        }
    }
    fn bit(&self, index: usize) -> Result<Bit, ()> {
        Ok(match self {
            Self::View(view) if view.scalar.is_none() && index < view.width => Bit::Position {
                index: view.start.checked_add(index).ok_or(())?,
                inverted: false,
            },
            Self::Bits { width, words } if index < *width => {
                Bit::Constant(words[index / 32] & (1 << (index % 32)) != 0)
            }
            Self::Mapped(bits) => *bits.get(index).ok_or(())?,
            _ => return Err(()),
        })
    }
    fn copy(&self, steps: &mut usize) -> Result<Self, ()> {
        spend(steps, 1)?;
        Ok(self.clone())
    }
    fn regions(&self, index: usize, selected: bool, steps: &mut usize) -> Result<Vec<Cube>, ()> {
        spend(steps, 1)?;
        if let Self::Relations(bits) = self {
            let bit = bits.get(index).ok_or(())?;
            let regions = if selected { &bit.truth } else { &bit.falsehood };
            let mut result = Vec::new();
            for cube in regions.iter() {
                spend(steps, cube.len().saturating_add(1))?;
                result.push(cube.clone());
            }
            return Ok(result);
        }
        Ok(match self.bit(index)? {
            Bit::Constant(value) => {
                if value == selected {
                    vec![Cube::new()]
                } else {
                    vec![]
                }
            }
            Bit::Position { index, inverted } => vec![Cube::from([(index, selected ^ inverted)])],
        })
    }
}
#[derive(Clone, Default)]
struct State {
    cube: Cube,
    bindings: BTreeMap<LocalId, Binding>,
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
    Ok(State {
        cube: state.cube.clone(),
        bindings: state
            .bindings
            .iter()
            .map(|(id, value)| Ok((*id, value.copy(steps)?)))
            .collect::<Result<_, ()>>()?,
    })
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

fn intersect_regions(left: &[Cube], right: &[Cube], steps: &mut usize) -> Result<Vec<Cube>, ()> {
    let mut result = Vec::new();
    for a in left {
        spend(steps, 1)?;
        for b in right {
            spend(steps, a.len().saturating_add(b.len()).saturating_add(1))?;
            if a.iter()
                .any(|(bit, value)| b.get(bit).is_some_and(|other| other != value))
            {
                continue;
            }
            let mut cube = a.clone();
            cube.extend(b.iter().map(|(bit, value)| (*bit, *value)));
            result.push(cube);
        }
    }
    Ok(result)
}

pub(crate) struct Analysis<'a> {
    target: Option<nir::target::TargetData>,
    context: &'a Context,
    domains: HashMap<Term, Option<Primitive>>,
}
impl<'a> Analysis<'a> {
    pub(crate) fn new(context: &'a Context, target: Option<nir::target::TargetData>) -> Self {
        Self {
            target,
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
            ExprKind::Local(id) => match state.bindings.get(id) {
                Some(Binding::View(view)) => Some(*view),
                _ => None,
            },
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
                            && primitive
                                .unrestricted_scalar_width_in(self.target)
                                .map(|w| w as usize)
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
                    && primitive
                        .unrestricted_scalar_width_in(self.target)
                        .map(|w| w as usize)
                        == Some(view.width)
                    && view.width <= 128
            }
        }))
    }
    fn observed_binding(
        &mut self,
        expr: &Expr,
        state: &State,
        steps: &mut usize,
        depth: usize,
    ) -> Result<Option<Binding>, ()> {
        spend(steps, 1)?;
        if depth > 128 {
            return Err(());
        }
        let primitive = self.primitive(&expr.ty, steps)?;
        if primitive == Some(Primitive::Bool) {
            self.predicate_binding(expr, state, steps, depth)
        } else {
            if let Some(Primitive::BitVec(width)) = primitive {
                match &expr.kind {
                    ExprKind::Not(value) => {
                        let Some(value) = self.observed_binding(value, state, steps, depth + 1)?
                        else {
                            return Ok(None);
                        };
                        if value.width() != Some(width) {
                            return Ok(None);
                        }
                        spend(steps, width)?;
                        if let Binding::Relations(bits) = value {
                            return Ok(Some(Binding::Relations(
                                bits.iter()
                                    .map(|bit| BitRegions {
                                        truth: bit.falsehood.clone(),
                                        falsehood: bit.truth.clone(),
                                    })
                                    .collect::<Vec<_>>()
                                    .into(),
                            )));
                        }
                        let bits = (0..width)
                            .map(|i| value.bit(i).map(Bit::inverted))
                            .collect::<Result<Vec<_>, ()>>()?;
                        return Ok(Some(Binding::Mapped(bits.into())));
                    }
                    ExprKind::And(left, right) | ExprKind::Or(left, right) => {
                        // Bitwise operators evaluate both operands. Even a zero
                        // mask cannot erase an unknown computation on the other side.
                        let Some(left) = self.observed_binding(left, state, steps, depth + 1)?
                        else {
                            return Ok(None);
                        };
                        let Some(right) = self.observed_binding(right, state, steps, depth + 1)?
                        else {
                            return Ok(None);
                        };
                        if left.width() != Some(width) || right.width() != Some(width) {
                            return Ok(None);
                        }
                        let is_or = matches!(expr.kind, ExprKind::Or(..));
                        if matches!(left, Binding::Relations(_))
                            || matches!(right, Binding::Relations(_))
                        {
                            return self
                                .bitwise_relations(&left, &right, is_or, steps)
                                .map(Some);
                        }
                        spend(steps, width)?;
                        let mut bits = Vec::with_capacity(width);
                        for i in 0..width {
                            let a = left.bit(i)?;
                            let b = right.bit(i)?;
                            bits.push(match (a, b) {
                                (Bit::Constant(a), Bit::Constant(b)) => {
                                    Bit::Constant(if is_or { a || b } else { a && b })
                                }
                                (Bit::Constant(c), value) | (value, Bit::Constant(c)) => {
                                    if c == is_or { Bit::Constant(c) } else { value }
                                }
                                _ if a == b => a,
                                _ if a == b.inverted() => Bit::Constant(is_or),
                                _ => {
                                    return self
                                        .bitwise_relations(&left, &right, is_or, steps)
                                        .map(Some);
                                }
                            });
                        }
                        return Ok(Some(Binding::Mapped(bits.into())));
                    }
                    ExprKind::BitSlice { value, start, end } => {
                        let Some(base) = self.observed_binding(value, state, steps, depth + 1)?
                        else {
                            return Ok(None);
                        };
                        if let Binding::Bits {
                            width: actual,
                            words,
                        } = base
                        {
                            if start > end || *end > actual || end - start != width {
                                return Ok(None);
                            }
                            spend(steps, width.saturating_add(width.div_ceil(32)))?;
                            let mut sliced = vec![0u32; width.div_ceil(32)];
                            for bit in 0..width {
                                let source = start + bit;
                                if words[source / 32] & (1 << (source % 32)) != 0 {
                                    sliced[bit / 32] |= 1 << (bit % 32);
                                }
                            }
                            return Ok(Some(Binding::Bits {
                                width,
                                words: sliced.into(),
                            }));
                        }
                        return Ok(match base {
                            Binding::Relations(bits)
                                if start <= end && *end <= bits.len() && end - start == width =>
                            {
                                spend(steps, width)?;
                                Some(Binding::Relations(bits[*start..*end].to_vec().into()))
                            }
                            Binding::Mapped(bits)
                                if start <= end && *end <= bits.len() && end - start == width =>
                            {
                                spend(steps, width)?;
                                Some(Binding::Mapped(bits[*start..*end].to_vec().into()))
                            }
                            Binding::View(view)
                                if view.scalar.is_none()
                                    && start <= end
                                    && *end <= view.width
                                    && end - start == width =>
                            {
                                Some(Binding::View(View {
                                    start: view.start.checked_add(*start).ok_or(())?,
                                    width,
                                    scalar: None,
                                }))
                            }
                            _ => None,
                        });
                    }
                    ExprKind::Literal(Literal::BitVec {
                        width: actual,
                        words,
                    }) if *actual == width => {
                        spend(steps, words.len().saturating_add(1))?;
                        if words.len() != width.div_ceil(32)
                            || (width % 32 != 0
                                && words.last().is_some_and(|word| *word >> (width % 32) != 0))
                        {
                            return Ok(None);
                        }
                        return Ok(Some(Binding::Bits {
                            width,
                            words: words.clone().into(),
                        }));
                    }
                    ExprKind::Local(id) => {
                        if let Some(binding @ Binding::Relations(bits)) = state.bindings.get(id) {
                            return if bits.len() == width {
                                Ok(Some(binding.copy(steps)?))
                            } else {
                                Ok(None)
                            };
                        }
                        if let Some(binding @ Binding::Mapped(bits)) = state.bindings.get(id) {
                            return if bits.len() == width {
                                Ok(Some(binding.copy(steps)?))
                            } else {
                                Ok(None)
                            };
                        }
                        if let Some(binding @ Binding::Bits { width: actual, .. }) =
                            state.bindings.get(id)
                        {
                            return if *actual == width {
                                Ok(Some(binding.copy(steps)?))
                            } else {
                                Ok(None)
                            };
                        }
                    }
                    _ => {}
                }
            }
            Ok(self
                .projection(expr, state, steps, depth)?
                .map(Binding::View))
        }
    }

    fn bitwise_relations(
        &mut self,
        left: &Binding,
        right: &Binding,
        is_or: bool,
        steps: &mut usize,
    ) -> Result<Binding, ()> {
        let width = left.width().ok_or(())?;
        if right.width() != Some(width) {
            return Err(());
        }
        spend(steps, width)?;
        let mut bits = Vec::with_capacity(width);
        for index in 0..width {
            // Split on the left result so unioned regions stay disjoint.
            // This is symbolic Boolean composition, not runtime short-circuiting.
            let left_selected = left.regions(index, is_or, steps)?;
            let left_other = left.regions(index, !is_or, steps)?;
            let right_selected = right.regions(index, is_or, steps)?;
            let right_other = right.regions(index, !is_or, steps)?;
            let mut selected = left_selected;
            let extra = intersect_regions(&left_other, &right_selected, steps)?;
            spend(steps, extra.len())?;
            selected.extend(extra);
            let other = intersect_regions(&left_other, &right_other, steps)?;
            let (truth, falsehood) = if is_or {
                (selected, other)
            } else {
                (other, selected)
            };
            bits.push(BitRegions {
                truth: truth.into(),
                falsehood: falsehood.into(),
            });
        }
        Ok(Binding::Relations(bits.into()))
    }

    fn bit_equality(
        &mut self,
        left: &Binding,
        right: &Binding,
        state: &State,
        steps: &mut usize,
    ) -> Result<Option<Vec<Cube>>, ()> {
        let (Some(width), Some(other)) = (left.width(), right.width()) else {
            return Ok(None);
        };
        if width != other {
            return Ok(None);
        }
        spend(steps, state.cube.len().saturating_add(1))?;
        let mut regions = vec![state.cube.clone()];
        if matches!(left, Binding::Relations(_)) || matches!(right, Binding::Relations(_)) {
            for index in 0..width {
                let mut equal = Vec::new();
                for selected in [false, true] {
                    let a = left.regions(index, selected, steps)?;
                    let b = right.regions(index, selected, steps)?;
                    let common = intersect_regions(&a, &b, steps)?;
                    spend(steps, common.len())?;
                    equal.extend(common);
                }
                regions = intersect_regions(&regions, &equal, steps)?;
                if regions.is_empty() {
                    break;
                }
            }
            return Ok(Some(regions));
        }
        for index in 0..width {
            spend(steps, 1)?;
            let a = left.bit(index)?;
            let b = right.bit(index)?;
            if a == b {
                continue;
            }
            let mut next = Vec::new();
            for region in regions {
                for expected in [false, true] {
                    spend(steps, 2)?;
                    let agrees = |bit| match bit {
                        Bit::Constant(actual) => actual == expected,
                        Bit::Position { index, inverted } => region
                            .get(&index)
                            .is_none_or(|actual| (*actual ^ inverted) == expected),
                    };
                    if !agrees(a) || !agrees(b) {
                        continue;
                    }
                    spend(steps, region.len().saturating_add(3))?;
                    let mut selected = region.clone();
                    let mut compatible = true;
                    for bit in [a, b] {
                        if let Bit::Position { index, inverted } = bit {
                            let required = expected ^ inverted;
                            if selected
                                .insert(index, required)
                                .is_some_and(|old| old != required)
                            {
                                compatible = false;
                                break;
                            }
                        }
                    }
                    if compatible {
                        next.push(selected);
                    }
                }
            }
            regions = next;
            if regions.is_empty() {
                break;
            }
        }
        Ok(Some(regions))
    }

    fn predicate_binding(
        &mut self,
        expr: &Expr,
        state: &State,
        steps: &mut usize,
        depth: usize,
    ) -> Result<Option<Binding>, ()> {
        spend(steps, 1)?;
        if depth > 128 {
            return Err(());
        }
        if let ExprKind::Local(id) = expr.kind {
            return match state.bindings.get(&id) {
                Some(binding @ Binding::Predicate { .. }) => Ok(Some(binding.copy(steps)?)),
                _ => Ok(None),
            };
        }
        if let ExprKind::Not(inner) = &expr.kind {
            return Ok(
                match self.predicate_binding(inner, state, steps, depth + 1)? {
                    Some(Binding::Predicate { truth, falsehood }) => Some(Binding::Predicate {
                        truth: falsehood,
                        falsehood: truth,
                    }),
                    _ => None,
                },
            );
        }
        let Some(truth) = self.condition(expr, state, steps, depth + 1)? else {
            return Ok(None);
        };
        let falsehood = outside_regions(state, &truth, steps)?
            .into_iter()
            .map(|s| s.cube)
            .collect::<Vec<_>>();
        spend(steps, truth.len().saturating_add(falsehood.len()))?;
        Ok(Some(Binding::Predicate {
            truth: truth.into(),
            falsehood: falsehood.into(),
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
            ExprKind::BitEqual(left, right) => {
                let Some(left) = self.observed_binding(left, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                let Some(right) = self.observed_binding(right, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                return self.bit_equality(&left, &right, state, steps);
            }
            ExprKind::ScalarEqual(left, right)
                if self.primitive(&left.ty, steps)? == Some(Primitive::Bool)
                    && self.primitive(&right.ty, steps)? == Some(Primitive::Bool) =>
            {
                let Some(left) = self.condition(left, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                let Some(right) = self.condition(right, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                let mut result = intersect_regions(&left, &right, steps)?;
                let no_left = outside_regions(state, &left, steps)?
                    .into_iter()
                    .map(|s| s.cube)
                    .collect::<Vec<_>>();
                let no_right = outside_regions(state, &right, steps)?
                    .into_iter()
                    .map(|s| s.cube)
                    .collect::<Vec<_>>();
                result.extend(intersect_regions(&no_left, &no_right, steps)?);
                return Ok(Some(result));
            }
            ExprKind::ScalarEqual(left, right) => {
                // Machine-integer equality against a literal is an exact bit
                // predicate. Floating equality cannot use this representation rule.
                let (value, literal) = match (&left.kind, &right.kind) {
                    (_, ExprKind::Literal(literal)) => (left, literal),
                    (ExprKind::Literal(literal), _) => (right, literal),
                    _ => return Ok(None),
                };
                let Some(primitive) = self.primitive(&left.ty, steps)? else {
                    return Ok(None);
                };
                if self.primitive(&right.ty, steps)? != Some(primitive)
                    || self
                        .target
                        .map_or_else(
                            || primitive.fixed_integer_layout(),
                            |target| target.integer_layout(primitive),
                        )
                        .is_none()
                    || !literal.has_primitive_type_in(&Term::Primitive(primitive), self.target)
                {
                    return Ok(None);
                }
                let Some(subject) = self.observed_binding(value, state, steps, depth + 1)? else {
                    return Ok(None);
                };
                let mut complete = true;
                let regions = self.pattern_value(
                    &Pattern::Literal(literal.clone()),
                    &subject,
                    copy(state, steps)?,
                    steps,
                    depth + 1,
                    &mut complete,
                )?;
                return Ok(complete.then(|| regions.into_iter().map(|state| state.cube).collect()));
            }
            ExprKind::Block {
                statements,
                result: Some(result),
            } => {
                let mut scoped = copy(state, steps)?;
                let mut owned = std::collections::BTreeSet::new();
                for statement in statements {
                    spend(steps, 1)?;
                    let statement = match &statement.kind {
                        ExprKind::Discard(inner) => inner.as_ref(),
                        _ => statement,
                    };
                    let (binding, value, declaration) = match &statement.kind {
                        ExprKind::Let { binding, value } => (*binding, value, true),
                        ExprKind::Assign { place, value }
                            if place.fields.is_empty() && owned.contains(&place.root) =>
                        {
                            (place.root, value, false)
                        }
                        _ => return Ok(None),
                    };
                    let Some(value) = self.observed_binding(value, &scoped, steps, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    if declaration && scoped.bindings.contains_key(&binding) {
                        return Ok(None);
                    }
                    scoped.bindings.insert(binding, value);
                    if declaration {
                        owned.insert(binding);
                    }
                }
                return self.condition(result, &scoped, steps, depth + 1);
            }
            ExprKind::Local(id) => {
                let Some(Binding::Predicate { truth, .. }) = state.bindings.get(id) else {
                    return Ok(None);
                };
                return Ok(Some(intersect_regions(
                    truth,
                    std::slice::from_ref(&state.cube),
                    steps,
                )?));
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
                if let ExprKind::Local(id) = inner.kind {
                    if let Some(Binding::Predicate { falsehood, .. }) = state.bindings.get(&id) {
                        return Ok(Some(intersect_regions(
                            falsehood,
                            std::slice::from_ref(&state.cube),
                            steps,
                        )?));
                    }
                }
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
            if result {
                spend(steps, state.cube.len())?;
            }
            return Ok(Some(if result {
                vec![state.cube.clone()]
            } else {
                vec![]
            }));
        }
        let ExprKind::Match { value, arms } = &expr.kind else {
            return Ok(None);
        };
        let Some(subject) = self.observed_binding(value, state, steps, depth + 1)? else {
            return Ok(None);
        };
        let mut remaining = vec![copy(state, steps)?];
        let mut truth = Vec::new();
        for arm in arms {
            if remaining.is_empty() {
                break;
            }
            if arm.guard.is_none() && matches!(arm.pattern, Pattern::Wild) {
                // This arm takes every remaining region without creating
                // bindings. In particular, a matches-expression's false arm
                // need not re-match and subtract each prefix of a wide mask.
                for region in &remaining {
                    let Some(regions) = self.condition(&arm.body, region, steps, depth + 1)? else {
                        return Ok(None);
                    };
                    spend(steps, regions.len())?;
                    truth.extend(regions);
                }
                return Ok(Some(truth));
            }
            let mut leftovers = Vec::new();
            for region in remaining {
                let mut complete = true;
                let mut matched = self.pattern_value(
                    &arm.pattern,
                    &subject,
                    copy(&region, steps)?,
                    steps,
                    depth + 1,
                    &mut complete,
                )?;
                if !complete {
                    return Ok(None);
                }
                if let Some(guard) = &arm.guard {
                    let mut accepted = Vec::new();
                    for candidate in matched {
                        let Some(regions) = self.condition(guard, &candidate, steps, depth + 1)?
                        else {
                            return Ok(None);
                        };
                        for cube in regions {
                            let mut selected = copy(&candidate, steps)?;
                            selected.cube = cube;
                            accepted.push(selected);
                        }
                    }
                    matched = accepted;
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
        state: State,
        steps: &mut usize,
        depth: usize,
        complete: &mut bool,
    ) -> Result<Vec<State>, ()> {
        self.pattern_value(pat, &Binding::View(view), state, steps, depth, complete)
    }

    fn pattern_value(
        &mut self,
        pat: &Pattern,
        subject: &Binding,
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
            Pattern::Observe { binding, inner } => {
                let previous = state.bindings.insert(*binding, subject.copy(steps)?);
                let mut matched =
                    self.pattern_value(inner, subject, state, steps, depth + 1, complete)?;
                for state in &mut matched {
                    spend(steps, 1)?;
                    if let Some(previous) = &previous {
                        state.bindings.insert(*binding, previous.copy(steps)?);
                    } else {
                        state.bindings.remove(binding);
                    }
                }
                Ok(matched)
            }
            Pattern::Binding { binding, inner } => {
                let mut matched =
                    self.pattern_value(inner, subject, state, steps, depth + 1, complete)?;
                for state in &mut matched {
                    spend(steps, 1)?;
                    state.bindings.insert(*binding, subject.copy(steps)?);
                }
                Ok(matched)
            }
            Pattern::Remap {
                pattern: inner,
                bindings,
            } => {
                let mut states =
                    self.pattern_value(inner, subject, state, steps, depth + 1, complete)?;
                for state in &mut states {
                    spend(steps, bindings.len().saturating_mul(2))?;
                    let mut remapped = Vec::new();
                    for (from, to) in bindings {
                        if let Some(value) = state.bindings.get(from) {
                            remapped.push((*to, value.copy(steps)?));
                        } else {
                            *complete = false;
                        }
                    }
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
                        let matched = self.pattern_value(
                            alternative,
                            subject,
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
                for state in
                    self.pattern_value(first, subject, state, steps, depth + 1, complete)?
                {
                    if let Some(next_view) =
                        self.observed_binding(value, &state, steps, depth + 1)?
                    {
                        result.extend(self.pattern_value(
                            next,
                            &next_view,
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
                for state in
                    self.pattern_value(inner, subject, state, steps, depth + 1, complete)?
                {
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
            Pattern::Literal(Literal::Bool(selected)) => {
                let Binding::Predicate { truth, falsehood } = subject else {
                    *complete = false;
                    return Ok(vec![]);
                };
                let regions = if *selected { truth } else { falsehood };
                let mut matched = Vec::new();
                for cube in intersect_regions(regions, std::slice::from_ref(&state.cube), steps)? {
                    let mut selected = copy(&state, steps)?;
                    selected.cube = cube;
                    matched.push(selected);
                }
                Ok(matched)
            }
            Pattern::Literal(literal) => {
                if matches!(subject, Binding::Mapped(_) | Binding::Relations(_)) {
                    let Literal::BitVec { width, words } = literal else {
                        *complete = false;
                        return Ok(vec![]);
                    };
                    spend(steps, words.len().saturating_add(1))?;
                    if Some(*width) != subject.width()
                        || words.len() != width.div_ceil(32)
                        || (width % 32 != 0
                            && words.last().is_some_and(|word| *word >> (width % 32) != 0))
                    {
                        return Ok(vec![]);
                    }
                    let expected = Binding::Bits {
                        width: *width,
                        words: words.clone().into(),
                    };
                    let Some(regions) = self.bit_equality(subject, &expected, &state, steps)?
                    else {
                        *complete = false;
                        return Ok(vec![]);
                    };
                    let mut matched = Vec::new();
                    for cube in regions {
                        let mut selected = copy(&state, steps)?;
                        selected.cube = cube;
                        matched.push(selected);
                    }
                    return Ok(matched);
                }
                if let Binding::Bits { width, words } = subject {
                    let Literal::BitVec {
                        width: other,
                        words: expected,
                    } = literal
                    else {
                        *complete = false;
                        return Ok(vec![]);
                    };
                    spend(
                        steps,
                        words.len().saturating_add(expected.len()).saturating_add(1),
                    )?;
                    return Ok(if width == other && words.as_ref() == expected.as_slice() {
                        vec![state]
                    } else {
                        vec![]
                    });
                }
                let Binding::View(view) = subject else {
                    *complete = false;
                    return Ok(vec![]);
                };
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
                            self.target.map_or_else(
                                || p.fixed_integer_layout(),
                                |target| target.integer_layout(p),
                            ) == Some((false, view.width as u32))
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
                            self.target.map_or_else(
                                || p.fixed_integer_layout(),
                                |target| target.integer_layout(p),
                            ) == Some((true, view.width as u32))
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
        let states = self.pattern(
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
        )?;
        Ok(states.into_iter().map(|state| state.cube).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_literal_equality_matches_exhaustive_byte_oracle() {
        let context = Context::default();
        let target = Some(nir::target::TargetData::new(8).unwrap());
        let binding = LocalId { owner: 9, index: 0 };
        let expr = |ty, kind| Expr {
            ty: Term::Primitive(ty),
            kind,
            source: nir::program::SourceRange { start: 0, end: 0 },
        };
        for primitive in [Primitive::Usize, Primitive::Isize] {
            let state = State {
                cube: Cube::new(),
                bindings: BTreeMap::from([(
                    binding,
                    Binding::View(View {
                        start: 0,
                        width: 8,
                        scalar: Some(primitive),
                    }),
                )]),
            };
            for raw in [0u8, 1, 127, 128, 255] {
                let literal = if primitive == Primitive::Isize {
                    Literal::SignedInteger(raw as i8 as i128)
                } else {
                    Literal::UnsignedInteger(raw as u128)
                };
                let condition = expr(
                    Primitive::Bool,
                    ExprKind::ScalarEqual(
                        Box::new(expr(primitive, ExprKind::Local(binding))),
                        Box::new(expr(primitive, ExprKind::Literal(literal))),
                    ),
                );
                let mut analysis = Analysis::new(&context, target);
                assert!(analysis.condition(&condition, &state, &mut 0, 0).is_err());
                let cubes = analysis
                    .condition(&condition, &state, &mut 10000, 0)
                    .unwrap()
                    .unwrap();
                for actual in 0u16..256 {
                    let accepted = cubes.iter().any(|cube| {
                        cube.iter()
                            .all(|(bit, value)| ((actual >> bit) & 1 != 0) == *value)
                    });
                    assert_eq!(accepted, actual == u16::from(raw));
                }
                assert!(
                    Analysis::new(&context, None)
                        .condition(&condition, &state, &mut 10000, 0)
                        .unwrap()
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn mapped_equality_agrees_with_all_two_bit_inputs_and_inversions() {
        let context = Context::default();
        let mut analysis = Analysis::new(&context, None);
        let choices = [
            Bit::Constant(false),
            Bit::Constant(true),
            Bit::Position {
                index: 0,
                inverted: false,
            },
            Bit::Position {
                index: 0,
                inverted: true,
            },
            Bit::Position {
                index: 1,
                inverted: false,
            },
            Bit::Position {
                index: 1,
                inverted: true,
            },
        ];
        let evaluate = |bit: Bit, input: usize| match bit {
            Bit::Constant(value) => value,
            Bit::Position { index, inverted } => (input >> index & 1 != 0) ^ inverted,
        };
        for a in choices {
            for b in choices {
                for c in choices {
                    for d in choices {
                        let left = Binding::Mapped(vec![a, b].into());
                        let right = Binding::Mapped(vec![c, d].into());
                        for is_or in [false, true] {
                            assert!(
                                analysis
                                    .bitwise_relations(&left, &right, is_or, &mut 1)
                                    .is_err()
                            );
                            let composed = analysis
                                .bitwise_relations(&left, &right, is_or, &mut 100_000)
                                .unwrap();
                            for (index, (lhs, rhs)) in [(a, c), (b, d)].into_iter().enumerate() {
                                for selected in [false, true] {
                                    let regions =
                                        composed.regions(index, selected, &mut 100_000).unwrap();
                                    for input in 0..4 {
                                        let expected = if is_or {
                                            evaluate(lhs, input) || evaluate(rhs, input)
                                        } else {
                                            evaluate(lhs, input) && evaluate(rhs, input)
                                        };
                                        let count = regions
                                            .iter()
                                            .filter(|cube| {
                                                cube.iter().all(|(index, value)| {
                                                    (input >> index & 1 != 0) == *value
                                                })
                                            })
                                            .count();
                                        assert_eq!(count, usize::from(expected == selected));
                                    }
                                }
                            }
                            // Equality consumes the complete composed relation,
                            // including correlations between its result bits.
                            let equal = analysis
                                .bit_equality(&composed, &left, &State::default(), &mut 100_000)
                                .unwrap()
                                .unwrap();
                            for input in 0..4 {
                                let expected = [(a, c), (b, d)].into_iter().all(|(l, r)| {
                                    let l = evaluate(l, input);
                                    let r = evaluate(r, input);
                                    (if is_or { l || r } else { l && r }) == l
                                });
                                let count = equal
                                    .iter()
                                    .filter(|cube| {
                                        cube.iter().all(|(index, value)| {
                                            (input >> index & 1 != 0) == *value
                                        })
                                    })
                                    .count();
                                assert_eq!(count, usize::from(expected));
                            }
                        }
                        let regions = analysis
                            .bit_equality(&left, &right, &State::default(), &mut 100_000)
                            .unwrap()
                            .unwrap();
                        for input in 0..4 {
                            let count = regions
                                .iter()
                                .filter(|cube| {
                                    cube.iter()
                                        .all(|(index, value)| (input >> index & 1 != 0) == *value)
                                })
                                .count();
                            let equal = evaluate(a, input) == evaluate(c, input)
                                && evaluate(b, input) == evaluate(d, input);
                            assert_eq!(count, usize::from(equal));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn constant_slices_preserve_bits_across_word_boundaries_and_bound_work() {
        let context = Context::default();
        let mut analysis = Analysis::new(&context, None);
        let source = nir::program::SourceRange { start: 0, end: 0 };
        let words = vec![0xa579e31c, 0x8c124eb7, 1];
        let value = Expr {
            source,
            ty: Term::Primitive(Primitive::BitVec(65)),
            kind: ExprKind::Literal(Literal::BitVec {
                width: 65,
                words: words.clone(),
            }),
        };
        for start in 0..=65 {
            for end in start..=65 {
                let expr = Expr {
                    source,
                    ty: Term::Primitive(Primitive::BitVec(end - start)),
                    kind: ExprKind::BitSlice {
                        value: Box::new(value.clone()),
                        start,
                        end,
                    },
                };
                let Some(Binding::Bits {
                    width,
                    words: sliced,
                }) = analysis
                    .observed_binding(&expr, &State::default(), &mut 100_000, 0)
                    .unwrap()
                else {
                    panic!("constant slice lost its value")
                };
                assert_eq!(width, end - start);
                assert_eq!(sliced.len(), width.div_ceil(32));
                for bit in 0..sliced.len() * 32 {
                    let expected =
                        bit < width && words[(start + bit) / 32] & (1 << ((start + bit) % 32)) != 0;
                    assert_eq!(sliced[bit / 32] & (1 << (bit % 32)) != 0, expected);
                }
                assert!(
                    analysis
                        .observed_binding(&expr, &State::default(), &mut 1, 0)
                        .is_err()
                );
                assert!(
                    analysis
                        .observed_binding(&expr, &State::default(), &mut 100_000, 129)
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn equality_regions_agree_with_small_bit_vectors_and_overlapping_slices() {
        let context = Context::default();
        let mut analysis = Analysis::new(&context, None);
        for width in 0..=4 {
            for left in 0..=4 - width {
                for right in 0..=4 - width {
                    let a = Binding::View(View {
                        start: left,
                        width,
                        scalar: None,
                    });
                    let b = Binding::View(View {
                        start: right,
                        width,
                        scalar: None,
                    });
                    for fixed in [None, Some(false), Some(true)] {
                        let state = State {
                            cube: fixed
                                .map(|bit| BTreeMap::from([(1, bit)]))
                                .unwrap_or_default(),
                            bindings: BTreeMap::new(),
                        };
                        let regions = analysis
                            .bit_equality(&a, &b, &state, &mut 100_000)
                            .unwrap()
                            .unwrap();
                        for value in 0..16usize {
                            let matches = regions
                                .iter()
                                .filter(|cube| {
                                    cube.iter()
                                        .all(|(bit, set)| (value >> bit & 1 != 0) == *set)
                                })
                                .count();
                            let mask = (1 << width) - 1;
                            let expected = (value >> left & mask) == (value >> right & mask)
                                && fixed.is_none_or(|bit| (value & 2 != 0) == bit);
                            assert_eq!(
                                matches,
                                usize::from(expected),
                                "width={width}, left={left}, right={right}, value={value}, fixed={fixed:?}"
                            );
                        }
                    }
                }
            }
        }
        let a = Binding::View(View {
            start: 0,
            width: 2,
            scalar: None,
        });
        for constant in 0..4u32 {
            let b = Binding::Bits {
                width: 2,
                words: vec![constant].into(),
            };
            assert!(
                analysis
                    .bit_equality(&a, &b, &State::default(), &mut 1)
                    .is_err()
            );
            let regions = analysis
                .bit_equality(&a, &b, &State::default(), &mut 1000)
                .unwrap()
                .unwrap();
            for value in 0..4u32 {
                assert_eq!(
                    regions.iter().any(|cube| cube
                        .iter()
                        .all(|(bit, set)| (value >> bit & 1 != 0) == *set)),
                    value == constant
                );
            }
        }
        let wide = Binding::View(View {
            start: 0,
            width: 64,
            scalar: None,
        });
        let independent = Binding::View(View {
            start: 64,
            width: 64,
            scalar: None,
        });
        let conjunction = analysis
            .bitwise_relations(&wide, &independent, false, &mut 100_000)
            .unwrap();
        let zero = Binding::Bits {
            width: 64,
            words: vec![0, 0].into(),
        };
        // Each zero result allows three input pairs. Bounded failure must
        // precede enumerating their exponentially many combinations.
        assert!(
            analysis
                .bit_equality(&conjunction, &zero, &State::default(), &mut 1000)
                .is_err()
        );
        assert!(
            analysis
                .bit_equality(&wide, &independent, &State::default(), &mut 1000)
                .is_err()
        );
        assert_eq!(
            analysis
                .bit_equality(&wide, &wide, &State::default(), &mut 1000)
                .unwrap(),
            Some(vec![Cube::new()])
        );
    }

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
                Binding::View(View {
                    start: 0,
                    width: 2,
                    scalar: None,
                }),
            )]),
        };
        let empty = Context::default();
        assert!(
            Analysis::new(&empty, None)
                .projection(&expr, &state, &mut 100_000, 0)
                .unwrap()
                .is_none()
        );
        let mut context = Context::default();
        context.assume(crate::ptc::Formula::equal(
            ty.clone(),
            Term::Primitive(Primitive::BitVec(2)),
        ));
        let mut analysis = Analysis::new(&context, None);
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
            Analysis::new(&empty, None)
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
            Analysis::new(&wrong_width, None)
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
                Binding::View(View {
                    start: 0,
                    width: 2,
                    scalar: None,
                }),
            )]),
        };
        for left in 0u8..16 {
            for right in 0u8..16 {
                let bound = LocalId { owner: 7, index: 5 };
                for (kind, expected) in [
                    (
                        ExprKind::Match {
                            value: Box::new(predicate(left)),
                            arms: vec![
                                MatchArm {
                                    pattern: Pattern::Binding {
                                        binding: bound,
                                        inner: Box::new(Pattern::Wild),
                                    },
                                    guard: Some(predicate(right)),
                                    body: expr(Primitive::Bool, ExprKind::Local(bound)),
                                },
                                MatchArm {
                                    pattern: Pattern::Wild,
                                    guard: None,
                                    body: expr(
                                        Primitive::Bool,
                                        ExprKind::Not(Box::new(predicate(left))),
                                    ),
                                },
                            ],
                        },
                        ((left & right) | (!left & !right)) & 15,
                    ),
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
                        ExprKind::ScalarEqual(
                            Box::new(predicate(left)),
                            Box::new(predicate(right)),
                        ),
                        !(left ^ right) & 15,
                    ),
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
                    let temporary = LocalId { owner: 7, index: 3 };
                    let local = || expr(Primitive::Bool, ExprKind::Local(temporary));
                    let wrapped = expr(
                        Primitive::Bool,
                        ExprKind::Block {
                            statements: vec![
                                expr(
                                    Primitive::Void,
                                    ExprKind::Let {
                                        binding: temporary,
                                        value: Box::new(check.clone()),
                                    },
                                ),
                                expr(
                                    Primitive::Void,
                                    ExprKind::Assign {
                                        place: nir::program::Place {
                                            root: temporary,
                                            fields: vec![],
                                        },
                                        value: Box::new(expr(
                                            Primitive::Bool,
                                            ExprKind::Not(Box::new(local())),
                                        )),
                                    },
                                ),
                            ],
                            result: Some(Box::new(expr(
                                Primitive::Bool,
                                ExprKind::Not(Box::new(local())),
                            ))),
                        },
                    );
                    for check in [check, wrapped] {
                        assert!(
                            Analysis::new(&Context::default(), None)
                                .condition(&check, &state, &mut 1, 0)
                                .is_err()
                        );
                        let cubes = Analysis::new(&Context::default(), None)
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
            bindings: BTreeMap::from([(subject, Binding::View(view))]),
        };
        assert!(
            Analysis::new(&Context::default(), None)
                .pattern(&check, view, state.clone(), &mut 1, 0, &mut true)
                .is_err()
        );
        let mut complete = true;
        let states = Analysis::new(&Context::default(), None)
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
