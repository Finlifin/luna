//! Closed local pointer aliases used only for loads and stores. Any escape or
//! pointer-cell mutation excludes its pointee from finite-domain propagation.
use super::*;

pub(super) type Aliases = BTreeMap<n::LocalId, n::LocalId>;

pub(super) fn target(pointer: &n::Expr, aliases: &Aliases) -> Option<n::LocalId> {
    match &pointer.kind {
        K::AddressOf(place) if staged(&place.ty) => match place.kind {
            K::Local(binding) => Some(binding),
            _ => None,
        },
        K::Local(binding) => aliases.get(binding).copied(),
        _ => None,
    }
}

pub(super) fn analyze(
    expression: &n::Expr,
    unavailable: &mut BTreeSet<n::LocalId>,
    fuel: &mut usize,
) -> Result<Aliases, String> {
    let mut declarations = Vec::new();
    let mut aliases = BTreeMap::new();
    iir::visit::expression(expression, fuel, |expr, fuel| {
        if let K::Let { binding, value } = &expr.kind {
            if matches!(&value.ty, Term::Pointer(inner) if staged(inner)) {
                declarations.push((*binding, copied(value, fuel)?));
            }
        }
        Ok(())
    })?;
    loop {
        let mut changed = false;
        for (binding, value) in &declarations {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("template pointer alias budget exhausted")?;
            if aliases.contains_key(binding) {
                continue;
            }
            if let Some(pointee) = target(value, &aliases) {
                aliases.insert(*binding, pointee);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut roots = vec![expression];
    while let Some(root) = roots.pop() {
        iir::visit::expression_pruned(root, fuel, |expr, fuel| {
            match &expr.kind {
                K::Let { binding, value }
                    if aliases.contains_key(binding)
                        && target(value, &aliases) == aliases.get(binding).copied() =>
                {
                    return Ok(false);
                }
                K::ReadPointer(pointer) if target(pointer, &aliases).is_some() => return Ok(false),
                K::WritePointer { pointer, value } if target(pointer, &aliases).is_some() => {
                    roots.push(value);
                    return Ok(false);
                }
                K::Local(binding) => {
                    if let Some(pointee) = aliases.get(binding) {
                        unavailable.insert(*pointee);
                    }
                }
                K::Assign { place, .. } => {
                    if let Some(pointee) = aliases.get(&place.root) {
                        unavailable.insert(*pointee);
                    }
                }
                K::Closure { captures, .. } => {
                    for capture in captures {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("template pointer capture budget exhausted")?;
                        if capture.value.is_none() {
                            if let Some(pointee) = aliases.get(&capture.binding) {
                                unavailable.insert(*pointee);
                            }
                        }
                    }
                }
                K::AddressOf(_) => unavailable.extend(nir::free_bindings::collect(expr, fuel)?),
                _ => {}
            }
            Ok(true)
        })?;
    }
    Ok(aliases)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local(index: u32) -> n::LocalId {
        n::LocalId { owner: 1, index }
    }
    fn ty() -> Term {
        Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::ComptimeValue,
                ty: Term::Primitive(nir::Primitive::Bool),
            }],
            result: Box::new(Term::Primitive(nir::Primitive::Bool)),
        }
    }
    fn expr(ty: Term, kind: K) -> n::Expr {
        n::Expr {
            source: n::SourceRange { start: 0, end: 1 },
            ty,
            kind,
        }
    }
    fn pointer(index: u32) -> n::Expr {
        expr(Term::Pointer(Box::new(ty())), K::Local(local(index)))
    }
    fn address(index: u32) -> n::Expr {
        expr(
            Term::Pointer(Box::new(ty())),
            K::AddressOf(Box::new(expr(ty(), K::Local(local(index))))),
        )
    }
    fn block(last: n::Expr) -> n::Expr {
        let void = Term::Primitive(nir::Primitive::Void);
        expr(
            void.clone(),
            K::Block {
                statements: vec![
                    expr(
                        void.clone(),
                        K::Let {
                            binding: local(2),
                            value: Box::new(address(0)),
                        },
                    ),
                    expr(
                        void,
                        K::Let {
                            binding: local(3),
                            value: Box::new(pointer(2)),
                        },
                    ),
                    last,
                ],
                result: None,
            },
        )
    }
    #[test]
    fn closed_aliases_are_distinct_from_returned_captured_and_retargeted_pointers() {
        let read = expr(ty(), K::ReadPointer(Box::new(pointer(3))));
        let mut unavailable = BTreeSet::new();
        let aliases = analyze(&block(read), &mut unavailable, &mut 10_000).unwrap();
        assert_eq!(aliases.get(&local(3)), Some(&local(0)));
        assert!(unavailable.is_empty());
        for escape in [
            expr(
                Term::Primitive(nir::Primitive::Never),
                K::Return(Some(Box::new(pointer(3)))),
            ),
            expr(
                Term::Primitive(nir::Primitive::Void),
                K::Assign {
                    place: n::Place {
                        root: local(2),
                        fields: vec![],
                    },
                    value: Box::new(address(1)),
                },
            ),
            expr(
                ty(),
                K::Closure {
                    parameters: vec![],
                    captures: vec![n::Capture {
                        binding: local(3),
                        ty: pointer(3).ty,
                        value: None,
                    }],
                    body: std::sync::Arc::new(expr(Term::Primitive(nir::Primitive::Void), K::Unit)),
                },
            ),
        ] {
            let mut unavailable = BTreeSet::new();
            analyze(&block(escape), &mut unavailable, &mut 10_000).unwrap();
            assert!(unavailable.contains(&local(0)));
        }
        assert!(analyze(&block(pointer(3)), &mut BTreeSet::new(), &mut 0).is_err());
    }
}
