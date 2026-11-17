//! Borrowed leaves retain logical paths, physical slots and payload guards.
//! Layout does not establish backing-storage validity or access permission.
use super::*;
use std::borrow::Cow;

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
        if intrinsic_pointer_value(&ty)
            || matches!(
                ty.as_ref(),
                Term::Function { .. }
                    | Term::UnsafeFunction(_)
                    | Term::Primitive(Primitive::Void | Primitive::Never)
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
}
