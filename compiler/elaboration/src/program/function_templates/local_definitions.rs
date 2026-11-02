//! Fresh declarations for copied template code, including their lexical world.
//! The enclosing expansion owns an unpublished package fork until checking.
use super::*;
use hir::common::DefId;

#[derive(Default)]
pub(super) struct Locals<'h> {
    owners: HashMap<OwnerId, OwnerId>,
    parents: HashMap<OwnerId, OwnerId>,
    completed: BTreeSet<OwnerId>,
    items: Vec<(OwnerId, DefId, hir::OwnerInfo<'h>)>,
    attributes: Vec<hir::DeferredDefinitionAttribute<'h>>,
    staged: Vec<(HirId, DefId)>,
}

impl<'h> Rebind<'_, 'h> {
    pub(super) fn prepare_local_definitions(
        &mut self,
        package: &mut Package<'h>,
        template: &Expr<'h>,
        instance: OwnerId,
    ) -> Result<(), Error> {
        let head = if let E::NFApplication(head, _) = template.kind {
            head
        } else {
            template
        };
        let E::ResolvedDef { def_id, .. } = head.kind else {
            return Ok(());
        };
        let Some(root) = self.source.definition_owner(def_id) else {
            return Ok(());
        };
        let mut children: std::collections::BTreeMap<OwnerId, Vec<OwnerId>> = Default::default();
        for (&child, &parent) in &self.source.lexical_parents {
            self.charge(template.span, 0)?;
            children.entry(parent).or_default().push(child);
        }
        for entries in children.values_mut() {
            entries.sort();
        }
        let mut staged: HashMap<OwnerId, Vec<DefId>> = HashMap::new();
        for (&parameter, &identity) in &self.source.staged_parameter_definitions {
            self.charge(template.span, 0)?;
            staged.entry(parameter.owner).or_default().push(identity);
        }
        let mut pending = vec![(root, instance)];
        let mut next = package.next_definition_index();
        while let Some((old_parent, new_parent)) = pending.pop() {
            for &old in children.get(&old_parent).into_iter().flatten() {
                self.charge(template.span, 0)?;
                if self.locals.owners.contains_key(&old) {
                    return Err(error(template.span, "cyclic template lexical definitions"));
                }
                let new = package.alloc_owner_id();
                self.locals.owners.insert(old, new);
                self.locals.parents.insert(new, new_parent);
                let item = self
                    .source
                    .item(old)
                    .ok_or_else(|| error(template.span, "missing template local definition"))?;
                let definition = self
                    .source
                    .owner_definition(old)
                    .ok_or_else(|| error(item.span, "template local definition has no identity"))?;
                let mut identities = vec![definition];
                identities.extend(
                    hir::definitions::declared_clause_parameters(item)
                        .iter()
                        .filter_map(|p| p.definition),
                );
                identities.extend(staged.get(&old).into_iter().flatten().copied());
                identities.sort_by_key(|id| (id.pkg, id.index));
                identities.dedup();
                for identity in identities {
                    self.charge(item.span, 0)?;
                    let fresh = DefId {
                        pkg: identity.pkg,
                        index: next,
                    };
                    next = next
                        .checked_add(1)
                        .ok_or_else(|| error(item.span, "template local identity exhausted"))?;
                    self.definitions.insert(identity, fresh);
                }
                pending.push((old, new));
            }
        }
        package.reserve_definition_prefix(next);
        Ok(())
    }

    pub(super) fn local_definition(
        &mut self,
        old: OwnerId,
        depth: usize,
    ) -> Result<OwnerId, Error> {
        let item = self
            .source
            .item(old)
            .ok_or_else(|| error(rustc_span::DUMMY_SP, "missing template local definition"))?;
        self.charge(item.span, depth)?;
        let new = self.locals.owners.get(&old).copied().ok_or_else(|| {
            error(
                item.span,
                "local definition cloning requires a template lexical scope",
            )
        })?;
        if !self.locals.completed.insert(old) {
            return Ok(new);
        }
        let previous = std::mem::replace(&mut self.namespace, new);
        let result = self.clone_local_item(item, new, depth + 1);
        self.namespace = previous;
        result?;
        Ok(new)
    }

    fn clone_local_item(
        &mut self,
        item: &hir::Item<'h>,
        owner: OwnerId,
        depth: usize,
    ) -> Result<(), Error> {
        let old = item.owner_id;
        let mut cloned = item.clone();
        cloned.owner_id = owner;
        let mut nodes = hir::OwnerNodes::new();
        cloned.kind = match &item.kind {
            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                if !sig.modifiers.is_function_template =>
            {
                let mut sig = sig.clone();
                sig.clause_params = self.clauses(sig.clause_params)?;
                let mut params = Vec::new();
                for param in sig.params {
                    self.charge(param.span, depth)?;
                    let cloned = self.parameter(param, depth)?;
                    if let Some(id) = self.source.staged_parameter_definitions.get(&param.hir_id) {
                        let identity = self.definitions.get(id).copied().ok_or_else(|| {
                            error(param.span, "missing cloned staged parameter identity")
                        })?;
                        self.locals.staged.push((cloned.hir_id, identity));
                    }
                    let node = self
                        .arena
                        .alloc_param(hir::body::BodyParam::from_fn_param(&cloned));
                    nodes.insert(
                        cloned.hir_id.local_id,
                        hir::ItemLocalId::new(0),
                        hir::Node::Param(node),
                    );
                    params.push(cloned);
                }
                sig.params = self.arena.alloc_fn_param_slice(params);
                sig.return_ty = sig
                    .return_ty
                    .map(|value| self.expr(value, depth))
                    .transpose()?;
                sig.return_binding = sig.return_binding.map(|id| self.id(id));
                sig.clause_constraints = self.constraints(sig.clause_constraints)?;
                for parameter in sig.clause_params {
                    nodes.insert(
                        parameter.hir_id.local_id,
                        hir::ItemLocalId::new(0),
                        hir::Node::ClauseParam(parameter),
                    );
                }
                match item.kind {
                    ItemKind::Fn(_, body) => ItemKind::Fn(sig, self.body(body, item.span, depth)?),
                    _ => ItemKind::FnDeclaration(sig),
                }
            }
            ItemKind::Const(ty, value) => {
                ItemKind::Const(self.expr(ty, depth)?, self.expr(value, depth)?)
            }
            ItemKind::TypeAlias(value, parameters, constraints)
            | ItemKind::Newtype(value, parameters, constraints) => {
                let parameters = self.clauses(parameters)?;
                let constraints = self.constraints(constraints)?;
                let value = self.expr(value, depth)?;
                for parameter in parameters {
                    nodes.insert(
                        parameter.hir_id.local_id,
                        hir::ItemLocalId::new(0),
                        hir::Node::ClauseParam(parameter),
                    );
                }
                if matches!(item.kind, ItemKind::TypeAlias(..)) {
                    ItemKind::TypeAlias(value, parameters, constraints)
                } else {
                    ItemKind::Newtype(value, parameters, constraints)
                }
            }
            _ => {
                return Err(error(
                    item.span,
                    "this local declaration kind in copied template code is not yet implemented",
                ));
            }
        };
        // Configurations and prefixes inside copied lexical scopes must follow
        // the same bindings; outer package prefixes retain their source scope.
        for attribute in &self.source.definition_attributes {
            self.charge(attribute.span, depth)?;
            let target = match attribute.target {
                hir::AttributeTarget::Definition(id) if id == old => {
                    hir::AttributeTarget::Definition(owner)
                }
                hir::AttributeTarget::Member(id) if id.owner == old => {
                    hir::AttributeTarget::Member(self.bindings.get(&id).copied().ok_or_else(
                        || {
                            error(
                                attribute.span,
                                "template local attribute target was not cloned",
                            )
                        },
                    )?)
                }
                _ => continue,
            };
            let lexical_owner = self
                .locals
                .owners
                .get(&attribute.lexical_owner)
                .copied()
                .or_else(|| {
                    self.source
                        .lexical_parents
                        .get(&old)
                        .filter(|parent| **parent == attribute.lexical_owner)
                        .and_then(|_| self.locals.parents.get(&owner).copied())
                });
            let bytes = attribute
                .expression
                .len()
                .checked_add(attribute.configuration_key.as_ref().map_or(0, String::len))
                .ok_or_else(|| error(attribute.span, "template local attribute size overflow"))?;
            self.budget = self.budget.checked_sub(bytes).ok_or_else(|| {
                error(attribute.span, "template local attribute budget exhausted")
            })?;
            let mut cloned = attribute.clone();
            cloned.target = target;
            if let Some(lexical_owner) = lexical_owner {
                cloned.lexical_owner = lexical_owner;
                cloned.value = self.expr(attribute.value, depth)?;
            }
            self.locals.attributes.push(cloned);
        }
        let identity = self
            .source
            .owner_definition(old)
            .and_then(|id| self.definitions.get(&id).copied())
            .ok_or_else(|| error(item.span, "missing cloned local definition identity"))?;
        let item = self.arena.alloc_item(cloned);
        nodes.insert(
            hir::ItemLocalId::new(0),
            hir::ItemLocalId::new(0),
            hir::Node::Item(item),
        );
        self.locals.items.push((
            owner,
            identity,
            hir::OwnerInfo {
                node: hir::OwnerNode::Item(item),
                nodes,
            },
        ));
        Ok(())
    }

    pub(super) fn publish_local_definitions(&mut self, package: &mut Package<'h>) {
        for (owner, identity, info) in self.locals.items.drain(..) {
            package.bind_definition(identity, owner);
            package
                .lexical_parents
                .insert(owner, self.locals.parents[&owner]);
            package.insert_owner(owner, info);
        }
        package
            .definition_attributes
            .append(&mut self.locals.attributes);
        package
            .staged_parameter_definitions
            .extend(self.locals.staged.drain(..));
    }
}
