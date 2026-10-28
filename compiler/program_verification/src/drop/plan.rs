//! Acquire destructor choices, then plan against ownership and variant evidence.
//! Steps require normal completion of prior steps; this is not cleanup execution.
use super::*;
use crate::{
    moves::{VARIANT_PATH, VariantSelection},
    ownership::{Availability, CleanupTree, Place, Shape, State},
};
use std::collections::{BTreeMap, BTreeSet};

mod reads;
pub use reads::{CleanupReadFootprint, CleanupReader};

#[derive(Clone, Debug)]
enum Layout<T> {
    Atomic,
    Fields(Vec<T>),
    Variants(Vec<Vec<T>>),
}
#[derive(Clone, Debug)]
pub struct AcquiredCleanup {
    ty: Term,
    resolution: Resolution,
    layout: Layout<AcquiredCleanup>,
    assumptions: Vec<TraitRequirement>,
}
impl AcquiredCleanup {
    pub fn ty(&self) -> &Term {
        &self.ty
    }
    /// Visit every represented component, requiring no custom destructor.
    /// Callers can additionally require Copy capability for each component.
    /// A pointer is atomic: ownership of its target is not implied.
    pub(crate) fn check_trivial_components(
        &self,
        fuel: &mut usize,
        check: &mut impl FnMut(&Term, &mut usize) -> Result<(), String>,
    ) -> Result<(), String> {
        self.visit_components(fuel, &mut |ty, resolution, fuel| {
            if !matches!(resolution, Resolution::NoCustomDrop(_)) {
                return Err(
                    "custom or unresolved Drop responsibility prevents trivial cleanup".into(),
                );
            }
            check(ty, fuel)
        })
    }

    /// Inspect only responsibilities retained by a checked ownership tree.
    /// This is behavior evidence, not permission to execute cleanup or access
    /// storage. Unrefined values still inspect every possible component.
    pub fn visit_remaining_components(
        &self,
        tree: &CleanupTree,
        variants: &[VariantSelection],
        fuel: &mut usize,
        check: &mut impl FnMut(&Term, &Resolution, &mut usize) -> Result<(), String>,
    ) -> Result<(), String> {
        fn visit(
            binding: &AcquiredCleanup,
            tree: &CleanupTree,
            base: &Place,
            variants: &[VariantSelection],
            fuel: &mut usize,
            depth: usize,
            check: &mut impl FnMut(&Term, &Resolution, &mut usize) -> Result<(), String>,
        ) -> Result<(), String> {
            step(fuel, depth)?;
            let Some(children) = &tree.children else {
                return if tree.availability == Availability::Empty {
                    Ok(())
                } else {
                    binding.visit_components(fuel, check)
                };
            };
            if tree.availability != Availability::Empty {
                check(&binding.ty, &binding.resolution, fuel)?;
            }
            if matches!(binding.layout, Layout::Atomic) {
                return Err("residual cleanup shape disagrees with atomic layout".into());
            }
            // An empty list is evidence that every payload field has moved;
            // the enclosing destructor (if any) was still checked above.
            if children.is_empty() {
                return Ok(());
            }
            let fields = match &binding.layout {
                Layout::Fields(fields) => fields,
                Layout::Variants(layouts) => {
                    let relative = tree
                        .place
                        .fields
                        .strip_prefix(base.fields.as_slice())
                        .ok_or("residual cleanup lies outside receiver")?;
                    let mut selected = None;
                    for variant in variants {
                        *fuel = fuel
                            .checked_sub(variant.fields.len() + 1)
                            .ok_or("residual cleanup variant budget exhausted")?;
                        if variant.fields == relative {
                            if selected.replace(variant.index).is_some() {
                                return Err("duplicate residual variant premise".into());
                            }
                        }
                    }
                    layouts
                        .get(selected.ok_or("decomposed cleanup requires a variant premise")?)
                        .ok_or("residual variant out of bounds")?
                }
                Layout::Atomic => unreachable!(),
            };
            for child in children {
                *fuel = fuel
                    .checked_sub(child.place.fields.len() + 1)
                    .ok_or("residual cleanup path budget exhausted")?;
                let index = *child
                    .place
                    .fields
                    .last()
                    .ok_or("residual child has no field")?;
                if child.place.root != tree.place.root
                    || child.place.fields.len() != tree.place.fields.len() + 1
                    || !child.place.fields.starts_with(&tree.place.fields)
                {
                    return Err("residual child is outside parent".into());
                }
                let field = fields
                    .get(index)
                    .ok_or("residual field exceeds acquired layout")?;
                visit(field, child, base, variants, fuel, depth + 1, check)?;
            }
            Ok(())
        }
        visit(self, tree, &tree.place, variants, fuel, 0, check)
    }

    /// Inspect the frozen selection for every possible cleanup component.
    /// This does not authorize memory access or acquire runtime environments.
    pub fn visit_components(
        &self,
        fuel: &mut usize,
        check: &mut impl FnMut(&Term, &Resolution, &mut usize) -> Result<(), String>,
    ) -> Result<(), String> {
        step(fuel, 0)?;
        check(&self.ty, &self.resolution, fuel)?;
        match &self.layout {
            Layout::Atomic => {}
            Layout::Fields(fields) => {
                for field in fields {
                    field.visit_components(fuel, check)?;
                }
            }
            Layout::Variants(variants) => {
                for fields in variants {
                    step(fuel, 0)?;
                    for field in fields {
                        field.visit_components(fuel, check)?;
                    }
                }
            }
        }
        Ok(())
    }
}
impl fmt::Display for AcquiredCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(acquired-cleanup (type {}) {}",
            self.ty, self.resolution
        )?;
        match &self.layout {
            Layout::Atomic => f.write_str(" (atomic)")?,
            Layout::Fields(fields) => {
                f.write_str(" (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str(")")?;
            }
            Layout::Variants(variants) => {
                f.write_str(" (variants")?;
                for (index, fields) in variants.iter().enumerate() {
                    write!(f, " (variant (index {index}) (fields")?;
                    for field in fields {
                        write!(f, " {field}")?;
                    }
                    f.write_str("))")?;
                }
                f.write_str(")")?;
            }
        }
        f.write_str(" (assumptions")?;
        for assumption in &self.assumptions {
            write!(f, " {assumption}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug)]
pub struct VariantCleanup {
    pub index: usize,
    pub steps: Vec<CleanupStep>,
}
impl fmt::Display for VariantCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(variant-cleanup (index {}) (steps", self.index)?;
        for step in &self.steps {
            write!(f, " {step}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug)]
pub enum CleanupStep {
    /// Mutually exclusive residual layouts, each verified with its own exact
    /// variant premises. Select before running any user destructor.
    SelectVariantPlan { plans: Vec<CleanupPlan> },
    /// Test this position's construction responsibility, not its value bits or
    /// whether every child is present. Children carry independent guards.
    IfConstructed {
        place: Place,
        steps: Vec<CleanupStep>,
    },
    Invoke {
        selected: SelectedDrop,
        access: AccessCheck,
    },
    /// Execute exactly one branch selected by the actual value's variant tag.
    SwitchVariant {
        place: Place,
        branches: Vec<VariantCleanup>,
    },
    /// Complete ownership responsibility; this does not free backing allocation.
    Release { place: Place, ty: Term },
}
impl fmt::Display for CleanupStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelectVariantPlan { plans } => {
                f.write_str("(select-variant-plan")?;
                for plan in plans {
                    write!(f, " {plan}")?;
                }
                f.write_str(")")
            }
            Self::IfConstructed { place, steps } => {
                write!(f, "(if-constructed {place} (steps")?;
                for step in steps {
                    write!(f, " {step}")?;
                }
                f.write_str("))")
            }
            Self::Invoke { selected, access } => write!(f, "(invoke-drop {selected} {access})"),
            Self::Release { place, ty } => write!(f, "(release-position {place} (type {ty}))"),
            Self::SwitchVariant { place, branches } => {
                write!(f, "(switch-variant {place} (branches")?;
                for branch in branches {
                    write!(f, " {branch}")?;
                }
                f.write_str("))")
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct CleanupPlan {
    pub receiver: Place,
    pub initial: Option<CleanupTree>,
    pub variants: Vec<VariantSelection>,
    pub steps: Vec<CleanupStep>,
}
impl fmt::Display for CleanupPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(cleanup-plan (receiver {}) (initial", self.receiver)?;
        if let Some(tree) = &self.initial {
            write!(f, " {tree}")?;
        }
        f.write_str(") (variants")?;
        for variant in &self.variants {
            write!(f, " {variant}")?;
        }
        f.write_str(") (steps")?;
        for step in &self.steps {
            write!(f, " {step}")?;
        }
        f.write_str("))")
    }
}
fn step(fuel: &mut usize, depth: usize) -> Result<(), String> {
    if depth > 128 {
        return Err("cleanup layout depth exhausted".into());
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or("cleanup planning budget exhausted")?;
    Ok(())
}
fn layout(package: &Package, ty: &Term, fuel: &mut usize) -> Result<Layout<Term>, String> {
    crate::memory::bound_type(ty, fuel)?;
    if let Some(fields) = nir::normal_forms::tuple_fields(Some(package), ty) {
        return Ok(Layout::Fields(fields.to_vec()));
    }
    match ty {
        // Optional cleanup uses a private absent/present switch. Only the
        // present branch owns a payload, including any nested optional value.
        Term::Optional(payload) => Ok(Layout::Variants(vec![vec![], vec![(**payload).clone()]])),
        // Cleanup switch tags are local representation positions: success first,
        // then the retained error-row order. This is not error type identity.
        Term::ErrorResult { errors, result } => Ok(Layout::Variants(
            std::iter::once(vec![(**result).clone()])
                .chain(errors.iter().map(|ty| vec![ty.clone()]))
                .collect(),
        )),
        Term::Nominal {
            declaration,
            arguments,
        } => {
            if let Some(definition) = package
                .structs
                .iter()
                .find(|s| s.declaration == *declaration)
            {
                for (_, field) in &definition.fields {
                    crate::memory::bound_type(field, fuel)?;
                }
                Ok(Layout::Fields(
                    definition
                        .instantiate_fields(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|(_, ty)| ty)
                        .collect(),
                ))
            } else if let Some(definition) =
                package.enums.iter().find(|e| e.declaration == *declaration)
            {
                for variant in &definition.variants {
                    step(fuel, 0)?;
                    for field in &variant.fields {
                        crate::memory::bound_type(field, fuel)?;
                    }
                }
                Ok(Layout::Variants(
                    definition
                        .instantiate_variants(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|v| v.fields)
                        .collect(),
                ))
            } else if let Some(definition) = package
                .newtypes
                .iter()
                .find(|n| n.declaration == *declaration)
            {
                // Private representation responsibility; this does not expose a
                // source field or inherit the representation's Drop impl.
                Ok(Layout::Fields(vec![
                    definition
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|e| e.to_string())?,
                ]))
            } else {
                Err("cleanup nominal layout is missing".into())
            }
        }
        Term::Record(fields) => Ok(Layout::Fields(
            fields.iter().map(|(_, ty)| ty.clone()).collect(),
        )),
        Term::Tuple(fields)
        | Term::Closure {
            environment: fields,
            ..
        } => Ok(Layout::Fields(fields.clone())),
        Term::Primitive(_)
        | Term::Pointer(_)
        | Term::Function { .. }
        | Term::UnsafeFunction(_)
        | Term::ComptimeFunction(_) => Ok(Layout::Atomic),
        _ => Err("cleanup layout is not established; opaque heads are not unfolded".into()),
    }
}
fn map_layout<T, U>(
    layout: Layout<T>,
    mut f: impl FnMut(T) -> Result<U, String>,
) -> Result<Layout<U>, String> {
    Ok(match layout {
        Layout::Atomic => Layout::Atomic,
        Layout::Fields(fields) => {
            Layout::Fields(fields.into_iter().map(&mut f).collect::<Result<_, _>>()?)
        }
        Layout::Variants(variants) => Layout::Variants(
            variants
                .into_iter()
                .map(|fields| fields.into_iter().map(&mut f).collect::<Result<_, _>>())
                .collect::<Result<_, _>>()?,
        ),
    })
}
fn same_layout(actual: &Layout<Term>, bound: &Layout<AcquiredCleanup>) -> bool {
    let fields = |a: &[Term], b: &[AcquiredCleanup]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a == &b.ty)
    };
    match (actual, bound) {
        (Layout::Atomic, Layout::Atomic) => true,
        (Layout::Fields(a), Layout::Fields(b)) => fields(a, b),
        (Layout::Variants(a), Layout::Variants(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| fields(a, b))
        }
        _ => false,
    }
}
/// Capture before destruction. Moves retain this artifact. Runtime environment
/// acquisition is not implemented and environment-bearing selections fail.
pub fn acquire_cleanup(
    package: &Package,
    world: &World<'_>,
    protocol: DeclarationId,
    ty: &Term,
    budget: Budget,
    fuel: &mut usize,
) -> Result<AcquiredCleanup, String> {
    fn acquire(
        package: &Package,
        world: &World<'_>,
        protocol: DeclarationId,
        ty: &Term,
        budget: Budget,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<AcquiredCleanup, String> {
        step(fuel, depth)?;
        crate::memory::bound_type(ty, fuel)?;
        let resolution = select(package, world, protocol, ty, budget)?;
        match &resolution {
            Resolution::Pending(reason) => {
                return Err(format!("cleanup binding is pending: {reason}"));
            }
            Resolution::Assumed { .. } => {
                return Err("cleanup binding requires concrete implementation evidence".into());
            }
            Resolution::Selected(s) if !s.capabilities.is_empty() => {
                return Err("cleanup binding requires runtime environment acquisition".into());
            }
            _ => (),
        }
        let layout = map_layout(layout(package, ty, fuel)?, |field| {
            acquire(package, world, protocol, &field, budget, fuel, depth + 1)
        })?;
        Ok(AcquiredCleanup {
            ty: ty.clone(),
            resolution,
            layout,
            assumptions: world.assumptions.to_vec(),
        })
    }
    acquire(package, world, protocol, ty, budget, fuel, 0)
}
struct Planner<'a, 'w> {
    package: &'a Package,
    world: &'a World<'w>,
    base: &'a Place,
    variants: &'a [VariantSelection],
    domains: &'a BTreeMap<Vec<usize>, BTreeSet<usize>>,
    qualified_path: Vec<usize>,
    fuel: &'a mut usize,
    pointers: Option<PointerMemory<'a>>,
    retired: Vec<Place>,
    variant_layouts: std::collections::BTreeMap<Place, super::access::VariantLayout>,
}
impl Planner<'_, '_> {
    fn shape(&mut self, binding: &AcquiredCleanup, depth: usize) -> Result<Shape, String> {
        step(self.fuel, depth)?;
        match &binding.layout {
            Layout::Fields(fields) => Ok(Shape::Fields(
                fields
                    .iter()
                    .map(|f| self.shape(f, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            _ => Ok(Shape::Value),
        }
    }
    fn children(
        &mut self,
        fields: &[AcquiredCleanup],
        state: &State,
        tree: &CleanupTree,
        depth: usize,
        steps: &mut Vec<CleanupStep>,
    ) -> Result<(), String> {
        if state
            .field_count(&tree.place)
            .map_err(|e| format!("cleanup shape: {e:?}"))?
            != Some(fields.len())
        {
            return Err("cleanup ownership shape disagrees with acquired layout".into());
        }
        let children = tree
            .children
            .as_ref()
            .ok_or("cleanup ownership shape disagrees with acquired layout")?;
        for child in children {
            let index = *child
                .place
                .fields
                .last()
                .ok_or("cleanup child lacks a field position")?;
            let mut expected = tree.place.clone();
            expected.fields.push(index);
            if child.place != expected {
                return Err("cleanup child is outside its parent".into());
            }
            let field = fields
                .get(index)
                .ok_or("cleanup child index exceeds acquired layout")?;
            self.qualified_path.push(index);
            let result = self.visit(field, state, child, depth + 1, steps);
            self.qualified_path.pop();
            result?;
        }
        Ok(())
    }
    fn variant_children(
        &mut self,
        index: usize,
        fields: &[AcquiredCleanup],
        state: &State,
        tree: &CleanupTree,
        depth: usize,
        steps: &mut Vec<CleanupStep>,
    ) -> Result<(), String> {
        let mut layout = vec![];
        for field in fields {
            crate::memory::bound_type(&field.ty, self.fuel)?;
            layout.push(field.ty.clone());
        }
        step(self.fuel, tree.place.fields.len())?;
        let previous = self.variant_layouts.insert(
            tree.place.clone(),
            super::access::VariantLayout {
                index,
                fields: layout,
            },
        );
        self.qualified_path.extend([VARIANT_PATH, index]);
        let result = self.variant_children_with_layout(fields, state, tree, depth, steps);
        self.qualified_path.truncate(self.qualified_path.len() - 2);
        if let Some(previous) = previous {
            self.variant_layouts.insert(tree.place.clone(), previous);
        } else {
            self.variant_layouts.remove(&tree.place);
        }
        result
    }
    fn variant_children_with_layout(
        &mut self,
        fields: &[AcquiredCleanup],
        state: &State,
        tree: &CleanupTree,
        depth: usize,
        steps: &mut Vec<CleanupStep>,
    ) -> Result<(), String> {
        if tree.children.is_some() {
            return self.children(fields, state, tree, depth, steps);
        }
        if tree.availability != Availability::Present {
            return Err("opaque enum payload is not fully present".into());
        }
        let shape = Shape::Fields(
            fields
                .iter()
                .map(|f| self.shape(f, depth + 1))
                .collect::<Result<_, _>>()?,
        );
        let mut refined = state
            .join(state, self.fuel)
            .map_err(|e| format!("variant state: {e:?}"))?;
        refined
            .refine_present_payload(&tree.place, &shape, self.fuel)
            .map_err(|e| format!("variant payload: {e:?}"))?;
        let payload = refined
            .cleanup_tree(&tree.place, self.fuel)
            .map_err(|e| format!("variant cleanup tree: {e:?}"))?
            .ok_or("missing variant payload")?;
        self.children(fields, &refined, &payload, depth, steps)
    }
    fn variant_payload(
        &mut self,
        variants: &[Vec<AcquiredCleanup>],
        state: &State,
        tree: &CleanupTree,
        depth: usize,
        steps: &mut Vec<CleanupStep>,
    ) -> Result<(), String> {
        if variants.is_empty() {
            return Err("enum cleanup has no constructible variant".into());
        }
        let relative = tree
            .place
            .fields
            .strip_prefix(self.base.fields.as_slice())
            .ok_or("variant outside cleanup root")?;
        if let Some(evidence) = self.variants.iter().find(|v| v.fields == relative) {
            let fields = variants
                .get(evidence.index)
                .ok_or("cleanup variant index is out of bounds")?;
            self.variant_children(evidence.index, fields, state, tree, depth, steps)?;
        } else {
            if tree.children.is_some() {
                return Err("decomposed enum cleanup requires active variant evidence".into());
            }
            let mut branches = vec![];
            for (index, fields) in variants.iter().enumerate() {
                step(self.fuel, 0)?;
                if self
                    .domains
                    .get(&self.qualified_path)
                    .or_else(|| self.domains.get(relative))
                    .is_some_and(|tags| !tags.contains(&index))
                {
                    continue;
                }
                let mut branch = vec![];
                let retired_before = self.retired.len();
                self.variant_children(index, fields, state, tree, depth, &mut branch)?;
                self.retired.truncate(retired_before);
                branches.push(VariantCleanup {
                    index,
                    steps: branch,
                });
            }
            steps.push(CleanupStep::SwitchVariant {
                place: tree.place.clone(),
                branches,
            });
        }
        Ok(())
    }
    fn visit(
        &mut self,
        binding: &AcquiredCleanup,
        state: &State,
        tree: &CleanupTree,
        depth: usize,
        steps: &mut Vec<CleanupStep>,
    ) -> Result<(), String> {
        step(self.fuel, depth)?;
        if binding.assumptions != self.world.assumptions {
            return Err("cleanup binding assumptions changed".into());
        }
        if !same_layout(
            &layout(self.package, &binding.ty, self.fuel)?,
            &binding.layout,
        ) {
            return Err("acquired cleanup field layout changed".into());
        }
        let evidence = match &binding.resolution {
            Resolution::NoCustomDrop(e) => e,
            Resolution::Selected(s) => &s.selection,
            _ => return Err("cleanup binding has no concrete resolution".into()),
        };
        if !self.world.index.type_resolution_is_current(evidence) {
            return Err("cleanup binding belongs to a different implementation world".into());
        }
        if matches!(binding.layout, Layout::Atomic) && tree.children.is_some() {
            return Err("cleanup ownership shape disagrees with acquired layout".into());
        }
        let conditional = tree.availability == Availability::Conditional;
        if tree.availability != Availability::Empty {
            if let Resolution::Selected(selected) = &binding.resolution {
                if !selected.program.preconditions.is_empty()
                    || !selected.program.postconditions.is_empty()
                    || !selected.contract.program.preconditions.is_empty()
                    || !selected.contract.program.postconditions.is_empty()
                {
                    return Err(
                        "cleanup program specification obligations are not discharged".into(),
                    );
                }
                let guarded;
                let access_state = if conditional {
                    guarded = state
                        .under_construction_guard(&tree.place, true, self.fuel)
                        .map_err(|e| format!("cleanup construction guard: {e:?}"))?;
                    &guarded
                } else {
                    state
                };
                let access = super::access::check_access_with_layout(
                    self.package,
                    self.world,
                    selected,
                    access_state,
                    &tree.place,
                    self.pointers,
                    &self.variant_layouts,
                    self.fuel,
                )
                .map_err(|e| {
                    if conditional {
                        format!(
                            "construction guard does not establish destructor field access: {e}"
                        )
                    } else {
                        e
                    }
                })?;
                for (access, targets) in access.footprint.accesses.iter().zip(&access.targets) {
                    if access.read_value {
                        for target in targets {
                            super::access::check_retired(&self.retired, target, self.fuel)?;
                        }
                    }
                }
                let invoke = CleanupStep::Invoke {
                    selected: selected.clone(),
                    access,
                };
                steps.push(if conditional {
                    CleanupStep::IfConstructed {
                        place: tree.place.clone(),
                        steps: vec![invoke],
                    }
                } else {
                    invoke
                });
            }
        }
        match &binding.layout {
            Layout::Atomic => (),
            Layout::Fields(fields) => self.children(fields, state, tree, depth, steps)?,
            Layout::Variants(variants) => {
                if conditional && tree.children.is_none() {
                    // Opaque variants have one token for the entire payload;
                    // unlike decomposed fields their tag cannot be read until
                    // the token is known live. Refinement below must not invent
                    // evidence for an already decomposed conditional payload.
                    let guarded = state
                        .under_construction_guard(&tree.place, true, self.fuel)
                        .map_err(|e| format!("enum construction guard: {e:?}"))?;
                    let mut present = tree.clone();
                    present.availability = Availability::Present;
                    // Plan only the payload here: the custom destructor has
                    // already been emitted, and release follows below.
                    let mut payload = vec![];
                    self.variant_payload(variants, &guarded, &present, depth, &mut payload)?;
                    steps.push(CleanupStep::IfConstructed {
                        place: tree.place.clone(),
                        steps: payload,
                    });
                } else {
                    // A selected decomposed payload has independent field
                    // tokens. Even an unconstructed parent may own initialized
                    // children, so do not put these under its parent guard.
                    self.variant_payload(variants, state, tree, depth, steps)?;
                }
            }
        }
        if tree.availability != Availability::Empty {
            let release = CleanupStep::Release {
                place: tree.place.clone(),
                ty: binding.ty.clone(),
            };
            steps.push(if conditional {
                CleanupStep::IfConstructed {
                    place: tree.place.clone(),
                    steps: vec![release],
                }
            } else {
                release
            });
        }
        self.retired.push(tree.place.clone());
        Ok(())
    }
}
pub fn plan_cleanup(
    package: &Package,
    world: &World<'_>,
    acquired: &AcquiredCleanup,
    state: &State,
    receiver: &Place,
    fuel: &mut usize,
) -> Result<CleanupPlan, String> {
    plan_cleanup_with_variants(package, world, acquired, state, receiver, &[], fuel)
}
/// Variant selections are relative to receiver and must be supplied by the
/// runtime tag branch or checked pattern decomposition, never guessed from types.
pub fn plan_cleanup_with_variants(
    package: &Package,
    world: &World<'_>,
    acquired: &AcquiredCleanup,
    state: &State,
    receiver: &Place,
    variants: &[VariantSelection],
    fuel: &mut usize,
) -> Result<CleanupPlan, String> {
    plan_cleanup_with_memory(
        package, world, acquired, state, receiver, variants, None, fuel,
    )
}

pub fn plan_cleanup_with_memory(
    package: &Package,
    world: &World<'_>,
    acquired: &AcquiredCleanup,
    state: &State,
    receiver: &Place,
    variants: &[VariantSelection],
    pointers: Option<PointerMemory<'_>>,
    fuel: &mut usize,
) -> Result<CleanupPlan, String> {
    plan_cleanup_with_domains(
        package,
        world,
        acquired,
        state,
        receiver,
        variants,
        &BTreeMap::new(),
        pointers,
        fuel,
    )
}

fn plan_cleanup_with_domains(
    package: &Package,
    world: &World<'_>,
    acquired: &AcquiredCleanup,
    state: &State,
    receiver: &Place,
    variants: &[VariantSelection],
    domains: &BTreeMap<Vec<usize>, BTreeSet<usize>>,
    pointers: Option<PointerMemory<'_>>,
    fuel: &mut usize,
) -> Result<CleanupPlan, String> {
    step(fuel, 0)?;
    let mut seen = BTreeSet::new();
    for variant in variants {
        step(fuel, 0)?;
        *fuel = fuel
            .checked_sub(variant.fields.len())
            .ok_or("cleanup variant budget exhausted")?;
        if !seen.insert(&variant.fields) {
            return Err("duplicate cleanup variant evidence".into());
        }
    }
    // A field path through an enum is meaningful only under its selected tag.
    // Validate even evidence for moved positions; it must never leak into all
    // alternatives of a dynamic parent switch.
    let mut validation = vec![];
    for (fields, tags) in domains {
        step(fuel, fields.len().saturating_add(tags.len()))?;
        if tags.is_empty() || variants.iter().any(|v| &v.fields == fields) {
            return Err("invalid cleanup variant domain".into());
        }
        if fields.contains(&VARIANT_PATH) {
            let mut binding = acquired;
            let mut position = 0;
            let mut physical = vec![];
            while position < fields.len() {
                step(fuel, position)?;
                let children = match &binding.layout {
                    Layout::Fields(children) => children,
                    Layout::Variants(alternatives) => {
                        if fields[position] != VARIANT_PATH {
                            return Err(
                                "conditional cleanup domain requires a variant qualifier".into()
                            );
                        }
                        let tag = *fields
                            .get(position + 1)
                            .ok_or("incomplete cleanup variant qualifier")?;
                        if domains
                            .get(&fields[..position])
                            .is_some_and(|tags| !tags.contains(&tag))
                            || variants
                                .iter()
                                .any(|v| v.fields == physical && v.index != tag)
                        {
                            return Err("cleanup domain traverses an excluded variant".into());
                        }
                        position += 2;
                        alternatives
                            .get(tag)
                            .ok_or("cleanup variant index is out of bounds")?
                    }
                    Layout::Atomic => return Err("cleanup domain traverses an atomic value".into()),
                };
                let field = *fields
                    .get(position)
                    .ok_or("cleanup variant qualifier lacks a payload field")?;
                binding = children
                    .get(field)
                    .ok_or("cleanup variant domain field is out of bounds")?;
                physical.push(field);
                position += 1;
            }
            match &binding.layout {
                Layout::Variants(alternatives)
                    if tags.iter().all(|tag| *tag < alternatives.len()) =>
                {
                    ()
                }
                Layout::Variants(_) => return Err("cleanup variant index is out of bounds".into()),
                _ => return Err("cleanup variant domain does not identify an enum".into()),
            }
            continue;
        }
        for index in tags {
            step(fuel, fields.len())?;
            validation.push(VariantSelection {
                fields: fields.clone(),
                index: *index,
            });
        }
    }
    for variant in variants.iter().chain(&validation) {
        let mut binding = acquired;
        for (depth, index) in variant.fields.iter().enumerate() {
            step(fuel, depth)?;
            let fields = match &binding.layout {
                Layout::Fields(fields) => fields,
                Layout::Variants(alternatives) => {
                    let parent = variants
                        .iter()
                        .find(|v| v.fields == variant.fields[..depth])
                        .ok_or("nested cleanup variant evidence requires its enclosing variant")?;
                    alternatives
                        .get(parent.index)
                        .ok_or("cleanup variant index is out of bounds")?
                }
                Layout::Atomic => {
                    return Err("cleanup variant evidence traverses an atomic value".into());
                }
            };
            binding = fields
                .get(*index)
                .ok_or("cleanup variant evidence field is out of bounds")?;
        }
        match &binding.layout {
            Layout::Variants(alternatives) if variant.index < alternatives.len() => (),
            Layout::Variants(_) => return Err("cleanup variant index is out of bounds".into()),
            _ => return Err("cleanup variant evidence does not identify an enum".into()),
        }
    }
    let initial = state
        .cleanup_tree(receiver, fuel)
        .map_err(|e| format!("cleanup receiver: {e:?}"))?;
    let mut steps = vec![];
    if let Some(tree) = &initial {
        Planner {
            package,
            world,
            base: receiver,
            variants,
            domains,
            qualified_path: vec![],
            fuel,
            pointers,
            retired: vec![],
            variant_layouts: Default::default(),
        }
        .visit(acquired, state, tree, 0, &mut steps)?;
    }
    Ok(CleanupPlan {
        receiver: receiver.clone(),
        initial,
        variants: variants.to_vec(),
        steps,
    })
}

/// Captured choices for one immutable program/implementation environment.
/// Field moves reuse their parent's captured subtree. This arena contains no
/// runtime environment values and cannot cross implementation worlds.
pub struct CleanupSession<'a, 'w> {
    package: &'a Package,
    world: &'a World<'w>,
    protocol: DeclarationId,
    budget: Budget,
    bindings: Vec<AcquiredCleanup>,
    // Paths refer only to immutable acquired trees, never runtime positions.
    binding_paths: std::collections::HashMap<Term, (usize, Vec<usize>)>,
    // Completed yes/no answers about immutable acquired layouts. A custom
    // destructor is a stable negative result, unlike a failed query.
    trivial_layouts: std::collections::HashMap<Term, bool>,
    rejection_stack: Vec<Term>,
    memory_inert_stack: Vec<Term>,
    read_footprint_stack: Vec<Term>,
    // Completed positive proofs only, in this immutable package/Drop world.
    // Budget exhaustion and unresolved recursion must never become cache facts.
    read_footprints: std::collections::HashMap<Term, CleanupReadFootprint>,
    memory_inert_types: std::collections::HashSet<Term>,
    nonrejecting_types: std::collections::HashSet<Term>,
}
impl<'a, 'w> CleanupSession<'a, 'w> {
    pub fn new(
        package: &'a Package,
        world: &'a World<'w>,
        protocol: DeclarationId,
        budget: Budget,
    ) -> Self {
        Self {
            package,
            world,
            protocol,
            budget,
            bindings: vec![],
            binding_paths: Default::default(),
            trivial_layouts: Default::default(),
            rejection_stack: vec![],
            memory_inert_stack: vec![],
            read_footprint_stack: vec![],
            read_footprints: Default::default(),
            memory_inert_types: Default::default(),
            nonrejecting_types: Default::default(),
        }
    }
    fn find<'b>(
        &'b self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<&'b AcquiredCleanup>, String> {
        crate::memory::bound_type(ty, fuel)?;
        step(fuel, 0)?;
        let Some((root, path)) = self.binding_paths.get(ty) else {
            return Ok(None);
        };
        let mut binding = &self.bindings[*root];
        let mut indices = path.iter();
        while let Some(index) = indices.next() {
            step(fuel, 0)?;
            binding = match &binding.layout {
                Layout::Fields(fields) => &fields[*index],
                Layout::Variants(variants) => {
                    let field = indices
                        .next()
                        .ok_or("invalid acquired cleanup variant path")?;
                    &variants[*index][*field]
                }
                Layout::Atomic => return Err("invalid acquired cleanup atomic path".into()),
            };
        }
        Ok(Some(binding))
    }
}
impl crate::moves::CleanupPolicy for CleanupSession<'_, '_> {
    fn is_trivial(&mut self, ty: &Term, fuel: &mut usize) -> Result<bool, String> {
        crate::memory::bound_type(ty, fuel)?;
        if let Some(trivial) = self.trivial_layouts.get(ty) {
            return Ok(*trivial);
        }
        self.acquire(ty, fuel)?;
        let binding = self.find(ty, fuel)?.ok_or("missing acquired cleanup")?;
        let mut trivial = true;
        binding.visit_components(fuel, &mut |_, resolution, _| {
            trivial &= matches!(resolution, Resolution::NoCustomDrop(_));
            Ok(())
        })?;
        self.trivial_layouts.insert(ty.clone(), trivial);
        Ok(trivial)
    }
    fn is_memory_inert(&mut self, ty: &Term, fuel: &mut usize) -> Result<bool, String> {
        crate::memory::bound_type(ty, fuel)?;
        if self.memory_inert_types.contains(ty) {
            return Ok(true);
        }
        for active in &self.memory_inert_stack {
            crate::memory::bound_type(active, fuel)?;
            if active == ty {
                return Ok(false);
            }
        }
        if self.memory_inert_stack.len() >= 64 {
            return Err("cleanup memory proof nesting budget exhausted".into());
        }
        self.memory_inert_stack.push(ty.clone());
        let result = (|| {
            let Some(footprint) = self.read_footprint(ty, fuel)? else {
                return Ok(false);
            };
            for access in footprint.root_accesses(self.package, fuel)? {
                step(fuel, 0)?;
                // Only receiver-local reads can omit external caller mapping.
                // Actual cleanup still checks field availability and Copy.
                if !access.target.through.is_empty() {
                    return Ok(false);
                }
            }
            Ok(true)
        })();
        self.memory_inert_stack.pop();
        if matches!(result, Ok(true)) {
            crate::memory::bound_type(ty, fuel)?;
            self.memory_inert_types.insert(ty.clone());
        }
        result
    }
    fn cleanup_reads(
        &mut self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<Vec<crate::memory::Access>>, String> {
        self.read_footprint(ty, fuel)?
            .map(|footprint| footprint.root_accesses(self.package, fuel))
            .transpose()
    }
    fn check_error_embedding(
        &mut self,
        source: &Term,
        target: &Term,
        fuel: &mut usize,
    ) -> Result<(), String> {
        crate::memory::bound_type(source, fuel)?;
        crate::memory::bound_type(target, fuel)?;
        if source == target {
            return Ok(());
        }
        self.acquire(source, fuel)?;
        let binding = self.find(source, fuel)?.ok_or("missing acquired cleanup")?;
        // Payload owners survive embedding unchanged. Only the enclosing
        // result's responsibility would otherwise disappear.
        if matches!(binding.resolution, Resolution::NoCustomDrop(_)) {
            Ok(())
        } else {
            Err("error embedding requires explicit Drop responsibility transfer for the source result".into())
        }
    }
    fn may_reject(&mut self, ty: &Term, fuel: &mut usize) -> Result<bool, String> {
        crate::memory::bound_type(ty, fuel)?;
        if self.nonrejecting_types.contains(ty) {
            return Ok(false);
        }
        for active in &self.rejection_stack {
            crate::memory::bound_type(active, fuel)?;
            if active == ty {
                return Ok(true);
            }
        }
        if self.rejection_stack.len() >= 64 {
            return Err("cleanup rejection nesting budget exhausted".into());
        }
        self.acquire(ty, fuel)?;
        let binding = self.find(ty, fuel)?.ok_or("missing acquired cleanup")?;
        let mut methods = vec![];
        let mut unresolved = false;
        binding.visit_components(fuel, &mut |_, resolution, fuel| {
            match resolution {
                Resolution::NoCustomDrop(_) => {}
                Resolution::Selected(selected) => {
                    for argument in &selected.method.arguments {
                        crate::memory::bound_type(argument, fuel)?;
                    }
                    methods.push((selected.method.body, selected.method.arguments.clone()));
                }
                _ => unresolved = true,
            }
            Ok(())
        })?;
        if unresolved {
            return Ok(true);
        }
        self.rejection_stack.push(ty.clone());
        let result = (|| {
            let package = self.package;
            // Keep the contextual memory summary: it can resolve code identities
            // which the structural rejection scan intentionally leaves unknown.
            // If retirement is its only uncertainty, inspect nested cleanup
            // against this same captured implementation world.
            let mut rejection = crate::rejection::Cache::new(Some(package));
            for (method, arguments) in methods {
                let summary = crate::memory::infer_in_package(package, method, &arguments, fuel)?;
                if summary.may_reject
                    && rejection
                        .function_may_reject_with_cleanup(
                            method,
                            &arguments,
                            fuel,
                            &mut |ty, fuel| self.may_reject(ty, fuel),
                        )
                        .map_err(str::to_owned)?
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        self.rejection_stack.pop();
        if matches!(result, Ok(false)) {
            crate::memory::bound_type(ty, fuel)?;
            self.nonrejecting_types.insert(ty.clone());
        }
        result
    }
    fn acquire(&mut self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        if self.find(ty, fuel)?.is_none() {
            let binding = acquire_cleanup(
                self.package,
                self.world,
                self.protocol,
                ty,
                self.budget,
                fuel,
            )?;
            // Build the index transactionally: an exhausted indexing budget
            // must not publish a partially registered acquisition.
            fn index(
                binding: &AcquiredCleanup,
                path: &mut Vec<usize>,
                output: &mut Vec<(Term, Vec<usize>)>,
                fuel: &mut usize,
                depth: usize,
            ) -> Result<(), String> {
                step(fuel, depth)?;
                crate::memory::bound_type(&binding.ty, fuel)?;
                *fuel = fuel
                    .checked_sub(path.len())
                    .ok_or("cleanup index budget exhausted")?;
                output.push((binding.ty.clone(), path.clone()));
                match &binding.layout {
                    Layout::Atomic => (),
                    Layout::Fields(fields) => {
                        for (field, child) in fields.iter().enumerate() {
                            path.push(field);
                            index(child, path, output, fuel, depth + 1)?;
                            path.pop();
                        }
                    }
                    Layout::Variants(variants) => {
                        for (variant, fields) in variants.iter().enumerate() {
                            step(fuel, depth)?;
                            path.push(variant);
                            for (field, child) in fields.iter().enumerate() {
                                path.push(field);
                                index(child, path, output, fuel, depth + 1)?;
                                path.pop();
                            }
                            path.pop();
                        }
                    }
                }
                Ok(())
            }
            let mut paths = vec![];
            index(&binding, &mut vec![], &mut paths, fuel, 0)?;
            let root = self.bindings.len();
            self.bindings.push(binding);
            for (ty, path) in paths {
                self.binding_paths.entry(ty).or_insert((root, path));
            }
        }
        Ok(())
    }
    fn plan(
        &mut self,
        ty: &Term,
        state: &State,
        place: &Place,
        variants: &[VariantSelection],
        fuel: &mut usize,
    ) -> Result<CleanupPlan, String> {
        self.plan_with_memory(ty, state, place, variants, None, fuel)
    }
    fn plan_with_domains(
        &mut self,
        ty: &Term,
        state: &State,
        place: &Place,
        variants: &[VariantSelection],
        domains: &BTreeMap<Vec<usize>, BTreeSet<usize>>,
        pointers: Option<PointerMemory<'_>>,
        fuel: &mut usize,
    ) -> Result<CleanupPlan, String> {
        let binding = self
            .find(ty, fuel)?
            .ok_or("cleanup responsibility has no acquired binding")?;
        plan_cleanup_with_domains(
            self.package,
            self.world,
            binding,
            state,
            place,
            variants,
            domains,
            pointers,
            fuel,
        )
    }
    fn plan_with_memory(
        &mut self,
        ty: &Term,
        state: &State,
        place: &Place,
        variants: &[VariantSelection],
        pointers: Option<PointerMemory<'_>>,
        fuel: &mut usize,
    ) -> Result<CleanupPlan, String> {
        let binding = self
            .find(ty, fuel)?
            .ok_or("cleanup responsibility has no acquired binding")?;
        plan_cleanup_with_memory(
            self.package,
            self.world,
            binding,
            state,
            place,
            variants,
            pointers,
            fuel,
        )
    }
}

#[cfg(test)]
mod tuple_layout_tests {
    use super::*;
    use nir::normal_forms::{Head, Role};

    #[test]
    fn cleanup_tuple_layout_uses_trusted_identity_and_preserves_fields() {
        let id = nir::DeclarationId {
            package: 9,
            local: 1,
        };
        let fields = vec![
            Term::Primitive(nir::Primitive::Bool),
            Term::Primitive(nir::Primitive::U8),
        ];
        let ty = Term::NormalForm {
            head: id,
            sort: nir::Sort::Type,
            arguments: vec![Term::TypeListValue(fields.clone())],
        };
        let mut package = Package {
            normal_forms: vec![Head {
                declaration: id,
                role: Role::Tuple,
                parameters: vec![nir::normal_forms::Parameter {
                    declaration: nir::DeclarationId {
                        package: 9,
                        local: 3,
                    },
                    binding: nir::program::LocalId { owner: 1, index: 0 },
                    kind: nir::normal_forms::Kind::TypeList,
                    ty: Term::Nominal {
                        declaration: nir::DeclarationId {
                            package: 9,
                            local: 2,
                        },
                        arguments: vec![Term::Primitive(nir::Primitive::Type)],
                    },
                }],
                requirements: vec![],
            }],
            ..Default::default()
        };
        nir::normal_forms::validate(&package).unwrap();
        let Layout::Fields(actual) = layout(&package, &ty, &mut 1000).unwrap() else {
            panic!("tuple layout");
        };
        assert_eq!(actual, fields);
        assert!(layout(&package, &ty, &mut 0).is_err());
        package.normal_forms[0].role = Role::Opaque;
        assert!(layout(&package, &ty, &mut 1000).is_err());
    }
}
