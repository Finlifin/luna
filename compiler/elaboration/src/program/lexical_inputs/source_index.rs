//! Immutable source topology, shared only within one checked HIR view.
use super::*;
use std::cell::OnceCell;

/// This is a source index, not a selected implementation or dependency result.
/// Retaining the package borrow makes the indexed view explicit; expansion or
/// another package must construct its own index even when owner IDs coincide.
pub(in crate::program) struct SourceProtocolIndex<'a, 'h> {
    package: &'a hir::Package<'h>,
    parents: OnceCell<BTreeMap<OwnerId, BTreeSet<OwnerId>>>,
}

impl<'a, 'h> SourceProtocolIndex<'a, 'h> {
    pub(in crate::program) fn new(package: &'a hir::Package<'h>) -> Self {
        Self {
            package,
            parents: OnceCell::new(),
        }
    }

    pub(in crate::program) fn for_parent(
        &self,
        parent: OwnerId,
        fuel: &mut usize,
        span: Span,
    ) -> Result<Option<&BTreeSet<OwnerId>>, Error> {
        fn charge(fuel: &mut usize, span: Span) -> Result<(), Error> {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "lexical protocol index budget exhausted"))?;
            Ok(())
        }

        if self.parents.get().is_none() {
            let mut parents = BTreeMap::<_, BTreeSet<_>>::new();
            for (&child, &owner) in &self.package.lexical_parents {
                charge(fuel, span)?;
                let item = self.package.item(child).ok_or_else(|| {
                    error(span, "lexical protocol index awaits a source declaration")
                })?;
                if matches!(item.kind, ItemKind::Impl(_) | ItemKind::Derive(_)) {
                    parents.entry(owner).or_default().insert(child);
                }
            }
            // Only a complete topology is reusable. An exhausted scan leaves
            // no partial index, and its consumed work is not rolled back.
            self.parents
                .set(parents)
                .map_err(|_| error(span, "lexical protocol index was already initialized"))?;
        }
        charge(fuel, span)?;
        Ok(self.parents.get().unwrap().get(&parent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hir::{HirArena, HirId, Ident, Item, OwnerInfo, OwnerNode, OwnerNodes, Symbol};
    use rustc_span::DUMMY_SP;

    fn add<'h>(package: &mut hir::Package<'h>, arena: &'h HirArena, kind: ItemKind<'h>) -> OwnerId {
        let owner = package.alloc_owner_id();
        let item = arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("item"), DUMMY_SP),
            span: DUMMY_SP,
            kind,
        });
        package.insert_owner(
            owner,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
        owner
    }

    fn fixture(
        arena: &HirArena,
        unrelated: usize,
    ) -> (hir::Package<'_>, OwnerId, OwnerId, BTreeSet<OwnerId>) {
        let mut package = hir::Package::new();
        let declaration = || {
            ItemKind::FnDeclaration(hir::FnSig {
                params: &[],
                return_ty: None,
                return_bind: None,
                return_binding: None,
                modifiers: Default::default(),
                clause_params: &[],
                clause_constraints: &[],
                span: DUMMY_SP,
            })
        };
        let parent = add(&mut package, arena, declaration());
        let other = add(&mut package, arena, declaration());
        let ty = arena.alloc_expr(Expr {
            hir_id: HirId::make_owner(parent),
            kind: H::TyType,
            span: DUMMY_SP,
        });
        let implementation = || {
            ItemKind::Impl(hir::item::ImplDef {
                generation_trait: None,
                phase: hir::item::ImplementationPhase::Runtime,
                self_ty: ty,
                trait_ref: None,
                clause_params: &[],
                clause_constraints: &[],
                items: vec![],
            })
        };
        let first = add(&mut package, arena, implementation());
        let second = add(
            &mut package,
            arena,
            ItemKind::Derive(hir::item::DeriveRequest {
                phase: hir::item::ImplementationPhase::Runtime,
                trait_ref: ty,
                self_ty: ty,
                strategy: None,
                clause_params: &[],
                clause_constraints: &[],
            }),
        );
        package.lexical_parents.insert(first, parent);
        package.lexical_parents.insert(second, parent);
        let elsewhere = add(&mut package, arena, implementation());
        package.lexical_parents.insert(elsewhere, other);
        // A global impl is not a contribution to either local scope.
        add(&mut package, arena, implementation());
        for _ in 0..unrelated {
            let child = add(&mut package, arena, declaration());
            package.lexical_parents.insert(child, other);
        }
        (package, parent, other, BTreeSet::from([first, second]))
    }

    #[test]
    fn warm_lookup_does_not_rescan_unrelated_declarations() {
        for unrelated in [0, 4096] {
            let arena = HirArena::new();
            let (package, parent, other, expected) = fixture(&arena, unrelated);
            let index = SourceProtocolIndex::new(&package);
            assert_eq!(
                index.for_parent(parent, &mut 10_000, DUMMY_SP).unwrap(),
                Some(&expected)
            );
            // The warm operation has a fixed budget regardless of package size.
            let mut fuel = 1;
            assert_eq!(
                index.for_parent(parent, &mut fuel, DUMMY_SP).unwrap(),
                Some(&expected)
            );
            assert_eq!(fuel, 0);
            assert_eq!(
                index
                    .for_parent(other, &mut 1, DUMMY_SP)
                    .unwrap()
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                index
                    .for_parent(OwnerId::INVALID, &mut 1, DUMMY_SP)
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn exhausted_construction_publishes_no_partial_topology_and_can_retry() {
        let arena = HirArena::new();
        let (package, parent, _, expected) = fixture(&arena, 64);
        let index = SourceProtocolIndex::new(&package);
        let mut fuel = 10;
        assert!(
            index
                .for_parent(parent, &mut fuel, DUMMY_SP)
                .unwrap_err()
                .message
                .contains("lexical protocol index budget exhausted")
        );
        assert_eq!(fuel, 0);
        assert!(index.parents.get().is_none());
        assert!(index.for_parent(parent, &mut 1, DUMMY_SP).is_err());
        assert!(index.parents.get().is_none());
        assert_eq!(
            index.for_parent(parent, &mut 1000, DUMMY_SP).unwrap(),
            Some(&expected)
        );
        assert!(index.for_parent(parent, &mut 0, DUMMY_SP).is_err());
        assert_eq!(
            index.for_parent(parent, &mut 1, DUMMY_SP).unwrap(),
            Some(&expected)
        );
    }

    #[test]
    fn identical_owner_numbers_in_a_new_view_have_independent_indices() {
        let arena = HirArena::new();
        let (package, parent, _, expected) = fixture(&arena, 0);
        let original = SourceProtocolIndex::new(&package);
        assert_eq!(
            original.for_parent(parent, &mut 100, DUMMY_SP).unwrap(),
            Some(&expected)
        );
        let mut changed = package.clone();
        for child in &expected {
            changed.lexical_parents.remove(child);
        }
        let successor = SourceProtocolIndex::new(&changed);
        assert!(
            successor
                .for_parent(parent, &mut 100, DUMMY_SP)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            original.for_parent(parent, &mut 1, DUMMY_SP).unwrap(),
            Some(&expected)
        );
    }

    #[test]
    fn missing_source_declarations_do_not_become_absence_evidence() {
        let arena = HirArena::new();
        let (mut package, parent, _, expected) = fixture(&arena, 0);
        let pending = package.alloc_owner_id();
        package.lexical_parents.insert(pending, parent);
        let index = SourceProtocolIndex::new(&package);
        assert!(
            index
                .for_parent(parent, &mut 100, DUMMY_SP)
                .unwrap_err()
                .message
                .contains("awaits a source declaration")
        );
        assert!(index.parents.get().is_none());
        let mut repaired = package.clone();
        let item = arena.alloc_item(Item {
            owner_id: pending,
            ident: Ident::new(Symbol::intern("pending"), DUMMY_SP),
            span: DUMMY_SP,
            kind: package
                .item(*expected.first().unwrap())
                .unwrap()
                .kind
                .clone(),
        });
        repaired.insert_owner(
            pending,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
        let repaired_index = SourceProtocolIndex::new(&repaired);
        let mut completed = expected;
        completed.insert(pending);
        assert_eq!(
            repaired_index
                .for_parent(parent, &mut 100, DUMMY_SP)
                .unwrap(),
            Some(&completed)
        );
        assert!(index.for_parent(parent, &mut 100, DUMMY_SP).is_err());
    }
}
