//! Borrowed leaves retain logical paths, physical slots and payload guards.
//! Layout does not establish backing-storage validity or access permission.
use super::*;
use std::borrow::Cow;
use std::collections::HashMap;

/// One pointer input with distinct symbolic and physical paths. Guards are
/// ordered outermost first, so an inactive payload is never projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerInputLeaf {
    pub input_fields: Vec<usize>,
    pub storage_fields: Vec<usize>,
    pub guards: Vec<(Vec<usize>, usize)>,
    pub pointee: Term,
}

impl fmt::Display for PointerInputLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(pointer-input (input-fields")?;
        display_owned_path(&self.input_fields, f)?;
        f.write_str(") (storage-fields")?;
        for field in &self.storage_fields {
            write!(f, " {field}")?;
        }
        f.write_str(") (guards")?;
        for (path, tag) in &self.guards {
            f.write_str(" (guard (path")?;
            for field in path {
                write!(f, " {field}")?;
            }
            write!(f, ") (tag {tag}))")?;
        }
        write!(f, ") (pointee {}))", self.pointee)
    }
}

/// An immutable text view's logical identity and physical value slot. Neither
/// path identifies the backing character region: that requires origin evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInputLeaf {
    pub input_fields: Vec<usize>,
    pub storage_fields: Vec<usize>,
    pub guards: Vec<(Vec<usize>, usize)>,
}

impl fmt::Display for TextInputLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(text-input (input-fields")?;
        display_owned_path(&self.input_fields, f)?;
        f.write_str(") (storage-fields")?;
        for field in &self.storage_fields {
            write!(f, " {field}")?;
        }
        f.write_str(") (guards")?;
        for (path, tag) in &self.guards {
            f.write_str(" (guard (path")?;
            for field in path {
                write!(f, " {field}")?;
            }
            write!(f, ") (tag {tag}))")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BorrowedInputLeaf {
    Pointer(PointerInputLeaf),
    Text(TextInputLeaf),
}

impl fmt::Display for BorrowedInputLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pointer(leaf) => leaf.fmt(f),
            Self::Text(leaf) => leaf.fmt(f),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextProvenance {
    #[default]
    Unchecked,
    VerifiedByRegionAnalysis,
}

/// Existing pointer-only consumers must reject text leaves explicitly until
/// they can transport and validate backing-region evidence at call boundaries.
pub fn pointer_input_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Vec<PointerInputLeaf>, String> {
    borrowed_input_layout(package, ty, fuel)?.into_iter().map(|leaf| match leaf {
        BorrowedInputLeaf::Pointer(leaf) => Ok(leaf),
        BorrowedInputLeaf::Text(leaf) => Err(format!(
            "string-view memory provenance is not implemented; str requires its backing character storage to remain valid; {leaf}"
        )),
    }).collect()
}

/// Separate text views from pointer inputs only after backing-region
/// provenance has been verified for the compilation graph.
pub fn pointer_input_layout_for(
    package: Option<&Package>,
    ty: &Term,
    provenance: TextProvenance,
    fuel: &mut usize,
) -> Result<Vec<PointerInputLeaf>, String> {
    match provenance {
        TextProvenance::Unchecked => pointer_input_layout(package, ty, fuel),
        TextProvenance::VerifiedByRegionAnalysis => Ok(borrowed_input_layout(package, ty, fuel)?
            .into_iter()
            .filter_map(|leaf| match leaf {
                BorrowedInputLeaf::Pointer(leaf) => Some(leaf),
                BorrowedInputLeaf::Text(_) => None,
            })
            .collect()),
    }
}

// Borrow original closure environments and move already instantiated fields.
// Retaining a cursor visits siblings in order without copying the path into
// every pending child or recursively retaining a traversal stack frame.
enum Children<'a> {
    Borrowed(std::iter::Enumerate<std::slice::Iter<'a, Term>>),
    Owned(std::iter::Enumerate<std::vec::IntoIter<Term>>),
}

impl<'a> Children<'a> {
    fn next(&mut self) -> Option<(usize, Cow<'a, Term>)> {
        match self {
            Self::Borrowed(fields) => fields.next().map(|(i, field)| (i, Cow::Borrowed(field))),
            Self::Owned(fields) => fields.next().map(|(i, field)| (i, Cow::Owned(field))),
        }
    }
}

enum Frame<'a> {
    Visit(Cow<'a, Term>),
    Fields {
        children: Children<'a>,
        variant: Option<usize>,
    },
    Variants(std::iter::Enumerate<std::vec::IntoIter<Vec<Term>>>),
    Restore {
        input: usize,
        storage: usize,
        guards: usize,
    },
}

/// Enumerate borrowed inputs without conflating a text region with a pointer
/// slot. Layout evidence does not establish any region's origin or lifetime.
/// An explicit depth-first cursor preserves path/guard order and charges the
/// same node, field-formation and copied-path work before exposing any result.
pub fn borrowed_input_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Vec<BorrowedInputLeaf>, String> {
    let mut input = Vec::new();
    let mut storage = Vec::new();
    let mut guards: Vec<(Vec<usize>, usize)> = Vec::new();
    let mut output = Vec::new();
    let mut frames = vec![Frame::Visit(Cow::Borrowed(ty))];
    while let Some(frame) = frames.pop() {
        let ty = match frame {
            Frame::Restore {
                input: input_len,
                storage: storage_len,
                guards: guards_len,
            } => {
                input.truncate(input_len);
                storage.truncate(storage_len);
                guards.truncate(guards_len);
                continue;
            }
            Frame::Fields {
                mut children,
                variant,
            } => {
                if let Some((index, field)) = children.next() {
                    frames.push(Frame::Fields { children, variant });
                    frames.push(Frame::Restore {
                        input: input.len(),
                        storage: storage.len(),
                        guards: guards.len(),
                    });
                    if let Some(tag) = variant {
                        input.extend([VARIANT_FIELD, tag, index]);
                    } else {
                        input.push(index);
                    }
                    storage.push(index);
                    frames.push(Frame::Visit(field));
                }
                continue;
            }
            Frame::Variants(mut variants) => {
                if let Some((tag, fields)) = variants.next() {
                    *fuel = fuel
                        .checked_sub(storage.len() + 1)
                        .ok_or("input pointer guard budget exhausted")?;
                    frames.push(Frame::Variants(variants));
                    frames.push(Frame::Restore {
                        input: input.len(),
                        storage: storage.len(),
                        guards: guards.len(),
                    });
                    guards.push((storage.clone(), tag));
                    frames.push(Frame::Fields {
                        children: Children::Owned(fields.into_iter().enumerate()),
                        variant: Some(tag),
                    });
                }
                continue;
            }
            Frame::Visit(ty) => ty,
        };
        if input.len() >= 128 {
            return Err("input pointer layout depth exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("input pointer layout budget exhausted")?;
        if *ty == Term::Primitive(Primitive::Str) {
            let cost = guards
                .iter()
                .try_fold(
                    input.len().saturating_add(storage.len()).saturating_add(1),
                    |cost, (path, _)| cost.checked_add(path.len().saturating_add(1)),
                )
                .ok_or("text view layout budget exhausted")?;
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("text view layout budget exhausted")?;
            output.push(BorrowedInputLeaf::Text(TextInputLeaf {
                input_fields: input.clone(),
                storage_fields: storage.clone(),
                guards: guards.clone(),
            }));
            continue;
        }
        if let Some((wrappers, pointee)) = pointer_representation(package, &ty, fuel)? {
            let guard_cost = guards
                .iter()
                .try_fold(0usize, |cost, (path, _)| cost.checked_add(path.len() + 1))
                .ok_or("input pointer path budget exhausted")?;
            let cost = input
                .len()
                .saturating_add(storage.len())
                .saturating_add(wrappers)
                .saturating_add(guard_cost)
                .saturating_add(1);
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("input pointer path budget exhausted")?;
            let mut physical = storage.clone();
            physical.extend(std::iter::repeat_n(0, wrappers));
            output.push(BorrowedInputLeaf::Pointer(PointerInputLeaf {
                input_fields: input.clone(),
                storage_fields: physical,
                guards: guards.clone(),
                pointee,
            }));
            continue;
        }
        if matches!(ty.as_ref(), Term::TraitObject { .. }) {
            let cost = guards
                .iter()
                .try_fold(
                    input.len().saturating_add(storage.len()).saturating_add(1),
                    |cost, (path, _)| cost.checked_add(path.len().saturating_add(1)),
                )
                .ok_or("dyn trait pointer layout budget exhausted")?;
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("dyn trait pointer layout budget exhausted")?;
            output.push(BorrowedInputLeaf::Pointer(PointerInputLeaf {
                input_fields: input.clone(),
                storage_fields: storage.clone(),
                guards: guards.clone(),
                pointee: Term::Primitive(Primitive::Void),
            }));
            continue;
        }
        if let Term::Closure { signature, .. } = ty.as_ref()
            && matches!(signature.as_ref(), Term::TraitObject { .. })
        {
            let cost = guards
                .iter()
                .try_fold(
                    input.len().saturating_add(storage.len()).saturating_add(1),
                    |cost, (path, _)| cost.checked_add(path.len().saturating_add(1)),
                )
                .ok_or("dyn trait closure layout budget exhausted")?;
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("dyn trait closure layout budget exhausted")?;
            output.push(BorrowedInputLeaf::Pointer(PointerInputLeaf {
                input_fields: input.clone(),
                storage_fields: storage.clone(),
                guards: guards.clone(),
                pointee: Term::Primitive(Primitive::Void),
            }));
        }
        if intrinsic_pointer_value(&ty)
            || matches!(
                ty.as_ref(),
                Term::Function { .. }
                    | Term::UnsafeFunction(_)
                    | Term::PureFunction(_)
                    | Term::ClosureContextType
                    | Term::EffectRowType
                    | Term::ErrorRowType
                    | Term::ReachabilityType
                    | Term::AnyType
                    | Term::ClosureContextValue(_)
                    | Term::Primitive(
                        Primitive::Void | Primitive::Never | Primitive::Type | Primitive::Unit,
                    )
            )
        {
            continue;
        }
        let children = match ty {
            Cow::Borrowed(Term::Closure { environment, .. }) => Frame::Fields {
                children: Children::Borrowed(environment.iter().enumerate()),
                variant: None,
            },
            Cow::Owned(Term::Closure { environment, .. }) => Frame::Fields {
                children: Children::Owned(environment.into_iter().enumerate()),
                variant: None,
            },
            ty if matches!(ty.as_ref(), Term::ErrorResult { .. } | Term::Optional(_))
                || matches!(ty.as_ref(), Term::Nominal { declaration, .. }
                    if package.is_some_and(|p| p.enums.iter().any(|e| e.declaration == *declaration))) =>
            {
                Frame::Variants(
                    owned_variant_fields(package, &ty, fuel)?
                        .into_iter()
                        .enumerate(),
                )
            }
            ty => Frame::Fields {
                children: Children::Owned(
                    owned_fields(package, &ty, fuel)?.into_iter().enumerate(),
                ),
                variant: None,
            },
        };
        frames.push(children);
    }
    Ok(output)
}

/// Package-scoped cache for structural memory layout queries over closed types.
/// Failed or budget-exhausted lookups are never cached; cache hits continue
/// deducting bounded work from `fuel` so exhausted budgets still fail closed.
#[derive(Default)]
pub struct TypeLayoutCache {
    borrowed_layouts: HashMap<Term, Vec<BorrowedInputLeaf>>,
    pointer_layouts_unchecked: HashMap<Term, Vec<PointerInputLeaf>>,
    pointer_layouts_verified: HashMap<Term, Vec<PointerInputLeaf>>,
    pointer_representations: HashMap<Term, Option<(usize, Term)>>,
    readable_unchecked: HashMap<Term, bool>,
    readable_verified: HashMap<Term, bool>,
    copy_readable_unchecked: HashMap<Term, bool>,
    copy_readable_verified: HashMap<Term, bool>,
    owned_variants: HashMap<Term, Vec<Vec<Term>>>,
    callable_paths_named: HashMap<Term, Vec<Vec<usize>>>,
    callable_paths_all: HashMap<Term, Vec<Vec<usize>>>,
    may_hold_text_or_callable: HashMap<Term, bool>,
}

impl TypeLayoutCache {
    pub fn borrowed_input_layout(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Vec<BorrowedInputLeaf>, String> {
        if let Some(cached) = self.borrowed_layouts.get(ty) {
            *fuel = fuel
                .checked_sub(cached.len().saturating_add(1))
                .ok_or("input pointer layout budget exhausted")?;
            return Ok(cached.clone());
        }
        let result = borrowed_input_layout(package, ty, fuel)?;
        self.borrowed_layouts.insert(ty.clone(), result.clone());
        Ok(result)
    }

    pub fn pointer_input_layout_for(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        provenance: TextProvenance,
        fuel: &mut usize,
    ) -> Result<Vec<PointerInputLeaf>, String> {
        let cache = match provenance {
            TextProvenance::Unchecked => &self.pointer_layouts_unchecked,
            TextProvenance::VerifiedByRegionAnalysis => &self.pointer_layouts_verified,
        };
        if let Some(cached) = cache.get(ty) {
            *fuel = fuel
                .checked_sub(cached.len().saturating_add(1))
                .ok_or("input pointer layout budget exhausted")?;
            return Ok(cached.clone());
        }
        let leaves = self.borrowed_input_layout(package, ty, fuel)?;
        let result = match provenance {
            TextProvenance::Unchecked => leaves
                .into_iter()
                .map(|leaf| match leaf {
                    BorrowedInputLeaf::Pointer(leaf) => Ok(leaf),
                    BorrowedInputLeaf::Text(leaf) => Err(format!(
                        "string-view memory provenance is not implemented; str requires its backing character storage to remain valid; {leaf}"
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?,
            TextProvenance::VerifiedByRegionAnalysis => leaves
                .into_iter()
                .filter_map(|leaf| match leaf {
                    BorrowedInputLeaf::Pointer(leaf) => Some(leaf),
                    BorrowedInputLeaf::Text(_) => None,
                })
                .collect(),
        };
        let cache = match provenance {
            TextProvenance::Unchecked => &mut self.pointer_layouts_unchecked,
            TextProvenance::VerifiedByRegionAnalysis => &mut self.pointer_layouts_verified,
        };
        cache.insert(ty.clone(), result.clone());
        Ok(result)
    }

    pub fn pointer_representation(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<(usize, Term)>, String> {
        if let Some(cached) = self.pointer_representations.get(ty) {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("pointer representation budget exhausted")?;
            return Ok(cached.clone());
        }
        let result = pointer_representation(package, ty, fuel)?;
        self.pointer_representations
            .insert(ty.clone(), result.clone());
        Ok(result)
    }

    fn cached_bool_query(
        unchecked: &mut HashMap<Term, bool>,
        verified: &mut HashMap<Term, bool>,
        package: Option<&Package>,
        ty: &Term,
        provenance: TextProvenance,
        fuel: &mut usize,
        compute: fn(Option<&Package>, &Term, TextProvenance, &mut usize) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let cache = match provenance {
            TextProvenance::Unchecked => &mut *unchecked,
            TextProvenance::VerifiedByRegionAnalysis => &mut *verified,
        };
        if let Some(&cached) = cache.get(ty) {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("read representation budget exhausted")?;
            return Ok(cached);
        }
        let result = compute(package, ty, provenance, fuel)?;
        let cache = match provenance {
            TextProvenance::Unchecked => unchecked,
            TextProvenance::VerifiedByRegionAnalysis => verified,
        };
        cache.insert(ty.clone(), result);
        Ok(result)
    }

    pub(crate) fn readable_representation_for(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        provenance: TextProvenance,
        fuel: &mut usize,
    ) -> Result<bool, String> {
        Self::cached_bool_query(
            &mut self.readable_unchecked,
            &mut self.readable_verified,
            package,
            ty,
            provenance,
            fuel,
            readable_representation_for,
        )
    }

    pub(crate) fn copy_readable_representation_for(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        provenance: TextProvenance,
        fuel: &mut usize,
    ) -> Result<bool, String> {
        Self::cached_bool_query(
            &mut self.copy_readable_unchecked,
            &mut self.copy_readable_verified,
            package,
            ty,
            provenance,
            fuel,
            copy_readable_representation_for,
        )
    }

    pub(crate) fn owned_variant_fields(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Vec<Vec<Term>>, String> {
        if let Some(cached) = self.owned_variants.get(ty) {
            *fuel = fuel
                .checked_sub(cached.len().saturating_add(1))
                .ok_or("memory variant lookup budget exhausted")?;
            return Ok(cached.clone());
        }
        let result = owned_variant_fields(package, ty, fuel)?;
        self.owned_variants.insert(ty.clone(), result.clone());
        Ok(result)
    }

    pub(crate) fn owned_callable_paths(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        include_closures: bool,
        fuel: &mut usize,
    ) -> Result<Vec<Vec<usize>>, String> {
        let cache = if include_closures {
            &self.callable_paths_all
        } else {
            &self.callable_paths_named
        };
        if let Some(cached) = cache.get(ty) {
            *fuel = fuel
                .checked_sub(cached.len().saturating_add(1))
                .ok_or("memory callable result layout budget exhausted")?;
            return Ok(cached.clone());
        }
        let paths: Vec<Vec<usize>> =
            super::owned_callable_bottom(package, ty, fuel, include_closures)?
                .into_keys()
                .collect();
        let cache = if include_closures {
            &mut self.callable_paths_all
        } else {
            &mut self.callable_paths_named
        };
        cache.insert(ty.clone(), paths.clone());
        Ok(paths)
    }

    pub(crate) fn owned_callable_bottom(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        include_closures: bool,
        fuel: &mut usize,
    ) -> Result<BTreeMap<Vec<usize>, super::CallableResult>, String> {
        Ok(self
            .owned_callable_paths(package, ty, include_closures, fuel)?
            .into_iter()
            .map(|path| (path, super::CallableResult::NoReturn))
            .collect())
    }

    pub(crate) fn result_callable_bottom(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<BTreeMap<Vec<usize>, super::CallableResult>, String> {
        let ty = match ty {
            Term::Computation { result, .. } => result,
            ty => ty,
        };
        self.owned_callable_bottom(package, ty, false, fuel)
    }

    pub(crate) fn code_result_bottom(
        &mut self,
        package: Option<&Package>,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<super::CodeFields, String> {
        let ty = match ty {
            Term::Computation { result, .. } => result,
            ty => ty,
        };
        Ok(self
            .owned_callable_paths(package, ty, true, fuel)?
            .into_iter()
            .map(|path| (path, super::CodeResult::NoReturn))
            .collect())
    }

    pub(crate) fn type_may_hold_text_or_callable(
        &mut self,
        package: &Package,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<bool, String> {
        if let Some(&cached) = self.may_hold_text_or_callable.get(ty) {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("text region inference budget exhausted")?;
            return Ok(cached);
        }
        let result = super::text_regions::type_may_hold_text_or_callable(package, ty, fuel)?;
        self.may_hold_text_or_callable.insert(ty.clone(), result);
        Ok(result)
    }
}

pub(crate) enum TypeLayoutCacheStorage<'a> {
    Owned(TypeLayoutCache),
    Borrowed(&'a mut TypeLayoutCache),
}

impl Default for TypeLayoutCacheStorage<'_> {
    fn default() -> Self {
        Self::Owned(TypeLayoutCache::default())
    }
}

impl TypeLayoutCacheStorage<'_> {
    pub(crate) fn as_mut(&mut self) -> &mut TypeLayoutCache {
        match self {
            Self::Owned(cache) => cache,
            Self::Borrowed(cache) => cache,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nominal_chain(depth: usize) -> (Package, Term) {
        let identity = |index: usize| DeclarationId {
            package: 91,
            local: index as u32,
        };
        let nominal = |index| Term::Nominal {
            declaration: identity(index),
            arguments: vec![],
        };
        let mut package = Package::default();
        for index in 0..depth {
            package.structs.push(nir::program::Struct {
                declaration: identity(index),
                name: format!("Layer{index}"),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![(
                    "inner".into(),
                    if index + 1 == depth {
                        Term::Primitive(Primitive::Str)
                    } else {
                        nominal(index + 1)
                    },
                )],
                packed: nir::program::PackedMode::None,
            });
        }
        (package, nominal(0))
    }

    #[test]
    fn nominal_layout_depth_is_bounded_without_recursive_traversal_frames() {
        std::thread::Builder::new()
            .stack_size(96 * 1024)
            .spawn(|| {
                let (package, ty) = nominal_chain(127);
                let leaves = borrowed_input_layout(Some(&package), &ty, &mut 1_000_000).unwrap();
                assert_eq!(
                    leaves,
                    vec![BorrowedInputLeaf::Text(TextInputLeaf {
                        input_fields: vec![0; 127],
                        storage_fields: vec![0; 127],
                        guards: vec![],
                    })]
                );
                let (package, ty) = nominal_chain(128);
                assert_eq!(
                    borrowed_input_layout(Some(&package), &ty, &mut 1_000_000).unwrap_err(),
                    "input pointer layout depth exhausted"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn interrupted_mixed_layouts_preserve_guards_and_independent_retries() {
        let pointer = Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
        let text = Term::Primitive(Primitive::Str);
        let ty = Term::Tuple(vec![
            Term::Optional(Box::new(Term::Tuple(vec![text.clone(), pointer.clone()]))),
            text.clone(),
            pointer.clone(),
        ]);
        let mut remaining = 100_000;
        let leaves = borrowed_input_layout(None, &ty, &mut remaining).unwrap();
        let work = 100_000 - remaining;
        assert_eq!(leaves.len(), 4);
        for (index, leaf) in leaves.iter().enumerate() {
            let (logical, physical, guards) = match leaf {
                BorrowedInputLeaf::Pointer(leaf) => {
                    assert_eq!(leaf.pointee, Term::Primitive(Primitive::Bool));
                    (&leaf.input_fields, &leaf.storage_fields, &leaf.guards)
                }
                BorrowedInputLeaf::Text(leaf) => {
                    (&leaf.input_fields, &leaf.storage_fields, &leaf.guards)
                }
            };
            let projection = project_owned_storage(None, &ty, logical, &mut 10_000).unwrap();
            assert_eq!(&projection.guards, guards);
            assert_eq!(
                projection.ty,
                if index % 2 == 0 {
                    text.clone()
                } else {
                    pointer.clone()
                }
            );
            if index < 2 {
                assert_eq!(logical, &[0, VARIANT_FIELD, 1, 0, index]);
                assert_eq!(physical, &[0, 0, index]);
                assert_eq!(guards, &[(vec![0], 1)]);
            } else {
                assert_eq!(logical, &[index - 1]);
                assert_eq!(physical, logical);
                assert!(guards.is_empty());
            }
        }
        for allowance in 0..work {
            let mut fuel = allowance;
            assert!(
                borrowed_input_layout(None, &ty, &mut fuel).is_err(),
                "{allowance}"
            );
            assert!(fuel <= allowance);
        }
        let mut retry = work;
        assert_eq!(
            borrowed_input_layout(None, &ty, &mut retry).unwrap(),
            leaves
        );
        assert_eq!(retry, 0);
    }

    #[test]
    fn text_views_remain_distinct_from_pointer_slots_and_guarded_payloads() {
        let text = Term::Primitive(Primitive::Str);
        let ty = Term::Tuple(vec![
            Term::Pointer(Box::new(text.clone())),
            text.clone(),
            Term::Optional(Box::new(text.clone())),
            Term::ErrorResult {
                errors: vec![text.clone()],
                result: Box::new(text),
            },
        ]);
        let leaves = borrowed_input_layout(None, &ty, &mut 10_000).unwrap();
        assert_eq!(leaves.len(), 5);
        let BorrowedInputLeaf::Pointer(pointer) = &leaves[0] else {
            panic!("pointer to a view is its own slot")
        };
        assert_eq!(pointer.input_fields, [0]);
        assert_eq!(pointer.pointee, Term::Primitive(Primitive::Str));
        for (leaf, logical, physical, guards) in [
            (&leaves[1], vec![1], vec![1], vec![]),
            (
                &leaves[2],
                vec![2, VARIANT_FIELD, 1, 0],
                vec![2, 0],
                vec![(vec![2], 1)],
            ),
            (
                &leaves[3],
                vec![3, VARIANT_FIELD, 0, 0],
                vec![3, 0],
                vec![(vec![3], 0)],
            ),
            (
                &leaves[4],
                vec![3, VARIANT_FIELD, 1, 0],
                vec![3, 0],
                vec![(vec![3], 1)],
            ),
        ] {
            let BorrowedInputLeaf::Text(leaf) = leaf else {
                panic!("expected text dependency")
            };
            assert_eq!(leaf.input_fields, logical);
            assert_eq!(leaf.storage_fields, physical);
            assert_eq!(leaf.guards, guards);
        }
        assert_eq!(
            leaves[2].to_string(),
            "(text-input (input-fields 2 (variant 1) 0) (storage-fields 2 0) (guards (guard (path 2) (tag 1))))"
        );
        assert!(
            pointer_input_layout(None, &ty, &mut 10_000)
                .unwrap_err()
                .contains("string-view memory provenance")
        );
        let verified = pointer_input_layout_for(
            None,
            &ty,
            TextProvenance::VerifiedByRegionAnalysis,
            &mut 10_000,
        )
        .unwrap();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].input_fields, vec![0]);
    }

    #[test]
    fn text_layouts_handle_nested_records_and_closures_without_unfolding_heads() {
        let ty = Term::Closure {
            environment: vec![Term::Record(vec![(
                "name".into(),
                Term::Primitive(Primitive::Str),
            )])],
            signature: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Void)),
            }),
        };
        let expected = vec![BorrowedInputLeaf::Text(TextInputLeaf {
            input_fields: vec![0, 0],
            storage_fields: vec![0, 0],
            guards: vec![],
        })];
        assert_eq!(
            borrowed_input_layout(None, &ty, &mut 1000).unwrap(),
            expected
        );
        assert!(borrowed_input_layout(None, &ty, &mut 1).is_err());
        assert_eq!(
            borrowed_input_layout(None, &ty, &mut 1000).unwrap(),
            expected
        );
        let opaque = Term::NormalForm {
            sort: nir::Sort::Type,
            head: DeclarationId {
                package: 1,
                local: 1,
            },
            arguments: vec![Term::Primitive(Primitive::Str)],
        };
        assert!(borrowed_input_layout(None, &opaque, &mut 1000).is_err());
        let mut deep = Term::Primitive(Primitive::Str);
        for _ in 0..130 {
            deep = Term::Optional(Box::new(deep));
        }
        assert!(borrowed_input_layout(None, &deep, &mut 100_000).is_err());
    }

    #[test]
    fn erased_and_closure_context_terms_have_no_borrowed_leaves_and_unknowns_fail_closed() {
        let context_value = Term::ClosureContextValue(vec![
            Term::Primitive(Primitive::Str),
            Term::Pointer(Box::new(Term::Primitive(Primitive::U8))),
        ]);
        for ty in [
            Term::Primitive(Primitive::Type),
            Term::ClosureContextType,
            context_value.clone(),
            Term::Tuple(vec![Term::Primitive(Primitive::Type), context_value]),
        ] {
            assert_eq!(borrowed_input_layout(None, &ty, &mut 1000).unwrap(), vec![]);
        }
        let unknown_rigid = Term::Rigid {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            sort: nir::Sort::Type,
        };
        assert!(borrowed_input_layout(None, &unknown_rigid, &mut 1000).is_err());
    }

    #[test]
    fn type_layout_cache_reuses_closed_results_under_tight_budget_and_never_caches_errors() {
        let ptr_bool = Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
        let wide_struct_id = DeclarationId {
            package: 92,
            local: 1,
        };
        let wide_enum_id = DeclarationId {
            package: 92,
            local: 2,
        };
        let wide_newtype_id = DeclarationId {
            package: 92,
            local: 3,
        };
        let param = DeclarationId {
            package: 92,
            local: 4,
        };
        let mut package = Package::default();
        package.structs.push(nir::program::Struct {
            declaration: wide_struct_id,
            name: "WideStruct".into(),
            type_parameters: vec![param],
            parameter_types: vec![Term::Primitive(Primitive::Type)],
            requirements: vec![],
            capabilities: vec![],
            fields: (0..40)
                .map(|i| {
                    (
                        format!("f{i}"),
                        if i == 39 {
                            Term::Primitive(Primitive::Str)
                        } else {
                            Term::Rigid {
                                declaration: param,
                                sort: nir::Sort::Type,
                            }
                        },
                    )
                })
                .collect(),
            packed: nir::program::PackedMode::None,
        });
        package.enums.push(nir::program::Enum {
            declaration: wide_enum_id,
            name: "WideEnum".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            is_intrinsic: false,
            variants: vec![nir::program::Variant {
                name: "a".into(),
                shape: nir::program::VariantShape::Unit,
                fields: vec![],
            }],
        });
        package.newtypes.push(nir::newtypes::Newtype {
            declaration: wide_newtype_id,
            name: "WidePtr".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            capture_count: 0,
            requirements: vec![],
            capabilities: vec![],
            representation: ptr_bool.clone(),
        });

        let ty = Term::Nominal {
            declaration: wide_struct_id,
            arguments: vec![ptr_bool.clone()],
        };
        let newtype_ty = Term::Nominal {
            declaration: wide_newtype_id,
            arguments: vec![],
        };
        let err_ty = Term::ErrorResult {
            errors: (0..12)
                .map(|_| Term::Nominal {
                    declaration: wide_enum_id,
                    arguments: vec![],
                })
                .collect(),
            result: Box::new(Term::Primitive(Primitive::U32)),
        };

        // Cold failure under tiny budget must not be cached.
        let mut cache = TypeLayoutCache::default();
        let err = cache
            .pointer_input_layout_for(
                Some(&package),
                &ty,
                TextProvenance::VerifiedByRegionAnalysis,
                &mut 5,
            )
            .unwrap_err();
        assert!(err.contains("budget"), "{err}");

        // Invalid nominal argument error must not be cached either.
        let bad_ty = Term::Nominal {
            declaration: wide_struct_id,
            arguments: vec![Term::Bool(true)],
        };
        assert!(
            cache
                .borrowed_input_layout(Some(&package), &bad_ty, &mut 100_000)
                .is_err()
        );
        assert!(
            cache
                .borrowed_input_layout(Some(&package), &bad_ty, &mut 100_000)
                .is_err()
        );

        // Warm all layout tables on valid types.
        let mut warm = 100_000;
        let cold_before = warm;
        let verified_leaves = cache
            .pointer_input_layout_for(
                Some(&package),
                &ty,
                TextProvenance::VerifiedByRegionAnalysis,
                &mut warm,
            )
            .unwrap();
        let cold_cost = cold_before - warm;
        assert_eq!(verified_leaves.len(), 39);
        assert!(cold_cost > verified_leaves.len() + 1);

        // Unchecked provenance must still reject `str` even after borrowed_layouts was warmed.
        assert!(
            cache
                .pointer_input_layout_for(Some(&package), &ty, TextProvenance::Unchecked, &mut 500)
                .unwrap_err()
                .contains("string-view memory provenance")
        );

        assert_eq!(
            cache
                .pointer_representation(Some(&package), &newtype_ty, &mut 500)
                .unwrap(),
            Some((1, Term::Primitive(Primitive::Bool)))
        );
        assert!(
            !cache
                .readable_representation_for(
                    Some(&package),
                    &ty,
                    TextProvenance::VerifiedByRegionAnalysis,
                    &mut 5_000,
                )
                .unwrap()
        );
        assert!(
            cache
                .copy_readable_representation_for(
                    Some(&package),
                    &ty,
                    TextProvenance::VerifiedByRegionAnalysis,
                    &mut 5_000,
                )
                .unwrap()
        );
        assert_eq!(
            cache
                .owned_variant_fields(Some(&package), &err_ty, &mut 500)
                .unwrap()
                .len(),
            13
        );

        // Repeated queries succeed well below cold cost (40 < 100 each) and still
        // fail closed when `fuel == 0`.
        let mut tight = 256;
        for _ in 0..4 {
            assert_eq!(
                cache
                    .pointer_input_layout_for(
                        Some(&package),
                        &ty,
                        TextProvenance::VerifiedByRegionAnalysis,
                        &mut tight,
                    )
                    .unwrap(),
                verified_leaves
            );
        }
        assert!(
            cache
                .pointer_input_layout_for(
                    Some(&package),
                    &ty,
                    TextProvenance::VerifiedByRegionAnalysis,
                    &mut 0,
                )
                .is_err()
        );
        assert!(
            cache
                .pointer_representation(Some(&package), &newtype_ty, &mut 0)
                .is_err()
        );
        assert!(
            cache
                .readable_representation_for(
                    Some(&package),
                    &ty,
                    TextProvenance::VerifiedByRegionAnalysis,
                    &mut 0,
                )
                .is_err()
        );
        assert!(
            cache
                .copy_readable_representation_for(
                    Some(&package),
                    &ty,
                    TextProvenance::VerifiedByRegionAnalysis,
                    &mut 0,
                )
                .is_err()
        );
        assert!(
            cache
                .owned_variant_fields(Some(&package), &err_ty, &mut 0)
                .is_err()
        );
    }

    #[test]
    fn type_layout_cache_callable_paths_and_text_or_callable_reuse_and_fail_closed() {
        let str_ty = Term::Primitive(Primitive::Str);
        let u8_ty = Term::Primitive(Primitive::U8);
        let fn_ty = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Void)),
        };
        let deep_clean_id = DeclarationId {
            package: 98,
            local: 1,
        };
        let wide_cb_id = DeclarationId {
            package: 98,
            local: 2,
        };
        let mut deep_clean = u8_ty;
        for _ in 0..8 {
            deep_clean = Term::Tuple(vec![deep_clean.clone(), deep_clean]);
        }
        let wide_cb = Term::Tuple(vec![
            fn_ty.clone(),
            Term::Closure {
                environment: vec![str_ty.clone(), fn_ty],
                signature: Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Void)),
                }),
            },
            Term::Optional(Box::new(str_ty)),
        ]);
        let mut package = Package::default();
        package.structs.push(nir::program::Struct {
            declaration: deep_clean_id,
            name: "DeepClean".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            fields: vec![("inner".into(), deep_clean)],
            packed: nir::program::PackedMode::None,
        });
        package.structs.push(nir::program::Struct {
            declaration: wide_cb_id,
            name: "WideCallbacks".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            fields: vec![("items".into(), wide_cb)],
            packed: nir::program::PackedMode::None,
        });
        let bad_arity_id = DeclarationId {
            package: 98,
            local: 99,
        };
        package.structs.push(nir::program::Struct {
            declaration: bad_arity_id,
            name: "BadArity".into(),
            type_parameters: vec![DeclarationId {
                package: 98,
                local: 999,
            }],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            fields: vec![],
            packed: nir::program::PackedMode::None,
        });
        let deep_nominal = Term::Nominal {
            declaration: deep_clean_id,
            arguments: vec![],
        };
        let wide_nominal = Term::Nominal {
            declaration: wide_cb_id,
            arguments: vec![],
        };
        let bad_arity_nominal = Term::Nominal {
            declaration: bad_arity_id,
            arguments: vec![],
        };

        let mut cache = TypeLayoutCache::default();
        // Errors (structural mismatch / budget exhaustion) are never cached;
        // type_may_hold_text_or_callable fails closed on non-budget errors.
        assert!(
            cache
                .owned_callable_paths(Some(&package), &bad_arity_nominal, true, &mut 1_000)
                .is_err()
        );
        assert!(
            cache
                .owned_callable_paths(Some(&package), &bad_arity_nominal, true, &mut 1_000)
                .is_err()
        );
        assert!(
            cache
                .type_may_hold_text_or_callable(&package, &bad_arity_nominal, &mut 1_000)
                .unwrap()
        );
        assert!(
            cache
                .owned_callable_paths(Some(&package), &wide_nominal, true, &mut 3)
                .is_err()
        );
        assert!(
            cache
                .type_may_hold_text_or_callable(&package, &deep_nominal, &mut 5)
                .is_err()
        );

        let mut cold_fuel = 50_000;
        let cold_before = cold_fuel;
        let paths = cache
            .owned_callable_paths(Some(&package), &wide_nominal, true, &mut cold_fuel)
            .unwrap();
        let cold_cost = cold_before - cold_fuel;
        assert_eq!(paths.len(), 2);
        assert!(cold_cost > paths.len() + 1);

        let mut tight = 10;
        assert_eq!(
            cache
                .owned_callable_paths(Some(&package), &wide_nominal, true, &mut tight)
                .unwrap(),
            paths
        );
        assert_eq!(tight, 10 - (paths.len() + 1));
        assert!(
            cache
                .owned_callable_paths(Some(&package), &wide_nominal, true, &mut 0)
                .is_err()
        );

        let mut text_cold = 50_000;
        assert!(
            !cache
                .type_may_hold_text_or_callable(&package, &deep_nominal, &mut text_cold)
                .unwrap()
        );
        assert!(50_000 - text_cold > 20);
        assert!(
            cache
                .type_may_hold_text_or_callable(&package, &wide_nominal, &mut 50_000)
                .unwrap()
        );

        let mut text_tight = 2;
        assert!(
            !cache
                .type_may_hold_text_or_callable(&package, &deep_nominal, &mut text_tight)
                .unwrap()
        );
        assert!(
            cache
                .type_may_hold_text_or_callable(&package, &wide_nominal, &mut text_tight)
                .unwrap()
        );
        assert_eq!(text_tight, 0);
        assert!(
            cache
                .type_may_hold_text_or_callable(&package, &deep_nominal, &mut 0)
                .is_err()
        );
    }
}
