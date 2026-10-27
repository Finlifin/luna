//! Transactional publication of generated HIR definitions.
//!
//! All ItemKind payloads use the ordinary HIR and subsequent elaboration. This
//! is a registration boundary, not type checking, name resolution, or a source
//! language builder API. Unfinished drafts never enter the package tables.
use crate::{Body, BodyId, ItemKind, LocalDefId, OwnerId, OwnerInfo, Package};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use symbol::DefId;
mod copy_budget;
mod resolution;
pub use resolution::ResolutionEvidence;
pub(crate) mod members;
pub use members::clauses as declared_clause_parameters;
mod nodes;
pub use nodes::DefinitionNodes;
mod structs;
pub use structs::{
    TypedField, assemble_struct, assemble_struct_definition, assemble_struct_with_defaults,
};
mod implementations;
mod interfaces;
pub use implementations::assemble_inherent_impl;
pub use interfaces::{assemble_impl, assemble_trait};
mod enums;
pub use enums::{assemble_enum, assemble_enum_definition};
mod associated;
mod containers;
mod effects;
mod functions;
mod parameters;
mod types;
pub use associated::assemble_associated;
pub use containers::{assemble_constant, assemble_module};
pub use effects::{assemble_effect, assemble_effect_group};
pub use functions::{AssembledFunction, assemble_function, assemble_function_declaration};
pub use types::{TypeDeclaration, assemble_newtype, assemble_type_alias};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: &'static str,
    pub owner: Option<OwnerId>,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(definition-error (kind {:?}) (owner ", self.kind)?;
        match self.owner {
            Some(owner) => write!(f, "{owner}"),
            None => f.write_str("none"),
        }?;
        f.write_str("))")
    }
}
impl std::error::Error for Error {}
fn error(kind: &'static str, owner: OwnerId) -> Error {
    Error {
        kind,
        owner: Some(owner),
    }
}
fn charge(fuel: &mut usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(1).ok_or(Error {
        kind: "budget exhausted",
        owner: None,
    })?;
    Ok(())
}

/// Fork package tables while sharing immutable arena nodes. Preflight the copy
/// budget before allocation; edits to the result cannot mutate cached source
/// tables. The caller must still check generated definitions before execution.
pub fn fork_package<'hir>(
    package: &Package<'hir>,
    fuel: &mut usize,
) -> Result<Package<'hir>, Error> {
    copy_budget::charge_package(package, fuel)?;
    Ok(package.clone())
}

#[derive(Clone)]
struct Definition<'hir> {
    info: OwnerInfo<'hir>,
    identity: Option<DefId>,
    lexical_parent: Option<OwnerId>,
}

/// Structural rejection and semantic rejection retain their original diagnostics.
#[derive(Debug)]
pub enum PublicationError<E> {
    Structure(Error),
    Check(E),
}
impl<E: fmt::Display> fmt::Display for PublicationError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Structure(error) => write!(f, "(definition-publication (structure {error}))"),
            Self::Check(error) => write!(
                f,
                "(definition-publication (check {:?}))",
                error.to_string()
            ),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for PublicationError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Structure(error) => error,
            Self::Check(error) => error,
        })
    }
}

/// Exclusive access prevents a competing publication from stealing reserved
/// identities. Dropping the batch, or a failed publish, changes no package table.
/// Arena allocations remain caller-owned and are not executable definitions.
pub struct DefinitionBatch<'package, 'hir> {
    package: &'package mut Package<'hir>,
    base: usize,
    identity_next: u32,
    definitions: Vec<Option<Definition<'hir>>>,
    bodies: Vec<(BodyId, Body<'hir>)>,
    body_slots: HashMap<BodyId, usize>,
    attributes: Vec<crate::DeferredDefinitionAttribute<'hir>>,
    resolution: Vec<(crate::HirId, ResolutionEvidence)>,
}
/// An immutable checked candidate holding exclusive access to its original draft.
/// Dropping it abandons preparation without changing published tables or drafts.
/// Commit performs no further validation, allocation or user callback.
pub struct PreparedPublication<'batch, 'package, 'hir, T> {
    batch: &'batch mut DefinitionBatch<'package, 'hir>,
    candidate: Package<'hir>,
    owners: Vec<OwnerId>,
    checked: T,
}
impl<'batch, 'package, 'hir, T> PreparedPublication<'batch, 'package, 'hir, T> {
    pub fn published(&self) -> &Package<'hir> {
        self.batch.published()
    }
    pub fn candidate(&self) -> &Package<'hir> {
        &self.candidate
    }
    pub fn owners(&self) -> &[OwnerId] {
        &self.owners
    }
    pub fn checked(&self) -> &T {
        &self.checked
    }

    /// Transfer the checked output into caller-owned stable storage without
    /// changing the candidate or rerunning semantic checking.
    pub fn map_checked<U>(
        self,
        map: impl FnOnce(T) -> U,
    ) -> PreparedPublication<'batch, 'package, 'hir, U> {
        PreparedPublication {
            batch: self.batch,
            candidate: self.candidate,
            owners: self.owners,
            checked: map(self.checked),
        }
    }

    pub fn commit(self) -> (Vec<OwnerId>, T) {
        *self.batch.package = self.candidate;
        self.batch.definitions.clear();
        self.batch.bodies.clear();
        self.batch.body_slots.clear();
        self.batch.attributes.clear();
        self.batch.resolution.clear();
        self.batch.base = self.batch.package.num_defs();
        (self.owners, self.checked)
    }
}
impl<T> fmt::Display for PreparedPublication<'_, '_, '_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(prepared-publication (owners")?;
        for owner in &self.owners {
            write!(f, " {owner}")?;
        }
        write!(
            f,
            ") (candidate-definitions {}) (candidate-bodies {}))",
            self.candidate.num_defs(),
            self.candidate.num_bodies()
        )
    }
}

impl<'package, 'hir> DefinitionBatch<'package, 'hir> {
    pub fn new(package: &'package mut Package<'hir>) -> Self {
        Self {
            base: package.num_defs(),
            identity_next: package.next_definition_index(),
            package,
            definitions: Vec::new(),
            bodies: Vec::new(),
            body_slots: HashMap::new(),
            attributes: Vec::new(),
            resolution: Vec::new(),
        }
    }
    /// Read-only access to previously published definitions, never draft entries.
    pub fn published(&self) -> &Package<'hir> {
        self.package
    }
    pub fn reserve(&mut self) -> Result<OwnerId, Error> {
        let raw = self
            .base
            .checked_add(self.definitions.len())
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n != u32::MAX)
            .ok_or(Error {
                kind: "owner identity exhausted",
                owner: None,
            })?;
        self.definitions.push(None);
        Ok(OwnerId::new(LocalDefId::new(raw)))
    }
    /// Reserve a resolver identity for a generated owner or clause. The source
    /// scan prefix is preserved even for declarations without an HIR owner.
    /// Failed publication retains reservations; dropping an unpublished batch
    /// discards them. Never expose draft identities as published definitions.
    pub fn reserve_identity(&mut self, package: u32, fuel: &mut usize) -> Result<DefId, Error> {
        charge(fuel)?;
        let mut allocator = symbol::DefIdGen::from_next_index(package, self.identity_next);
        let identity = allocator.try_next().ok_or(Error {
            kind: "definition identity exhausted",
            owner: None,
        })?;
        self.identity_next = allocator.count();
        Ok(identity)
    }
    fn slot(&self, owner: OwnerId) -> Option<usize> {
        owner
            .def_id
            .index()
            .checked_sub(self.base)
            .filter(|i| *i < self.definitions.len())
    }
    /// Replace an unfinished description, for example after correcting a failed
    /// validation. Already-published owners cannot be changed through this API.
    /// `lexical_parent` describes a block-local declaration's enclosing scope;
    /// membership in structs/traits/impls is represented by their item lists.
    pub fn define(
        &mut self,
        info: OwnerInfo<'hir>,
        identity: Option<DefId>,
        lexical_parent: Option<OwnerId>,
    ) -> Result<(), Error> {
        let owner = info.node.expect_item().owner_id;
        let slot = self
            .slot(owner)
            .ok_or_else(|| error("owner not reserved in this batch", owner))?;
        if let Some(identity) = identity {
            self.identity_next = self.identity_next.max(identity.index.saturating_add(1));
        }
        for parameter in members::clauses(info.node.expect_item()) {
            if let Some(identity) = parameter.definition {
                self.identity_next = self.identity_next.max(identity.index.saturating_add(1));
            }
        }
        self.definitions[slot] = Some(Definition {
            info,
            identity,
            lexical_parent,
        });
        Ok(())
    }
    pub fn body(&mut self, id: BodyId, body: Body<'hir>) -> Result<(), Error> {
        self.slot(id.hir_id.owner)
            .ok_or_else(|| error("body owner not reserved in this batch", id.hir_id.owner))?;
        if let Some(index) = self.body_slots.get(&id) {
            self.bodies[*index].1 = body;
        } else {
            self.body_slots.insert(id, self.bodies.len());
            self.bodies.push((id, body));
        }
        Ok(())
    }
    /// Replace the draft's complete attribute sequence, preserving application
    /// order and lexical provenance. Targets must belong to this batch; lexical
    /// contexts may refer to previously published definitions.
    pub fn set_attributes(&mut self, attributes: Vec<crate::DeferredDefinitionAttribute<'hir>>) {
        self.attributes = attributes;
    }
    fn exists(&self, owner: OwnerId) -> bool {
        self.slot(owner)
            .is_some_and(|i| self.definitions[i].is_some())
            || self.package.owner(owner).is_some()
    }
    /// Check an isolated candidate containing both published and draft owners.
    /// Only a successful check installs the candidate and consumes the drafts.
    /// Errors and unwinding leave the target and drafts unchanged. Side effects
    /// outside the candidate are not rolled back.
    ///
    /// The callback sees immutable HIR and must create fresh semantic state;
    /// this method cannot roll back a caller's existing inference context.
    /// Fuel covers candidate table copying and structural validation, not the
    /// callback. Copy preflight completes before allocating the candidate.
    /// This initial implementation copies package tables; arena nodes are shared.
    pub fn publish_checked<T, E>(
        &mut self,
        fuel: &mut usize,
        check: impl FnOnce(&Package<'hir>) -> Result<T, E>,
    ) -> Result<(Vec<OwnerId>, T), PublicationError<E>> {
        Ok(self.prepare_checked(fuel, check)?.commit())
    }

    /// Check once without publishing. The returned preparation prevents edits
    /// to this batch until committed or dropped. External admission may fail
    /// after checking; dropping the preparation retains the repairable draft.
    pub fn prepare_checked<T, E>(
        &mut self,
        fuel: &mut usize,
        check: impl FnOnce(&Package<'hir>) -> Result<T, E>,
    ) -> Result<PreparedPublication<'_, 'package, 'hir, T>, PublicationError<E>> {
        self.charge_candidate_copy(fuel)
            .map_err(PublicationError::Structure)?;
        let mut candidate = self.package.clone();
        let mut batch = DefinitionBatch {
            package: &mut candidate,
            base: self.base,
            identity_next: self.identity_next,
            definitions: self.definitions.clone(),
            bodies: self.bodies.clone(),
            body_slots: self.body_slots.clone(),
            attributes: self.attributes.clone(),
            resolution: self.resolution.clone(),
        };
        let owners = batch.publish(fuel).map_err(PublicationError::Structure)?;
        let result = check(&candidate).map_err(PublicationError::Check)?;
        Ok(PreparedPublication {
            batch: self,
            candidate,
            owners,
            checked: result,
        })
    }
    /// Check every entry before making any mutation. Budget exhaustion is also
    /// transactional; a caller can supply fresh fuel and retry the same draft.
    pub fn publish(&mut self, fuel: &mut usize) -> Result<Vec<OwnerId>, Error> {
        charge(fuel)?;
        self.validate_resolution(fuel)?;
        let mut identities = BTreeSet::new();
        let mut checked_containers = BTreeSet::new();
        let mut checked_lexical_parents = BTreeSet::new();
        for attribute in &self.attributes {
            charge(fuel)?;
            let owner = match attribute.target {
                crate::AttributeTarget::Definition(owner) => owner,
                crate::AttributeTarget::Member(id) => id.owner,
            };
            let definition = self
                .slot(owner)
                .and_then(|slot| self.definitions[slot].as_ref())
                .ok_or_else(|| error("attribute target not defined in this batch", owner))?;
            if !self.exists(attribute.lexical_owner)
                || attribute.value.hir_id.owner != attribute.lexical_owner
            {
                return Err(error("invalid attribute lexical context", owner));
            }
            if let crate::AttributeTarget::Member(id) = attribute.target {
                if id.local_id.raw() == 0 || definition.info.nodes.get(id.local_id).is_none() {
                    return Err(error("missing attribute member target", owner));
                }
            }
        }
        for (id, body) in &self.bodies {
            charge(fuel)?;
            let owner = id.hir_id.owner;
            if !self.exists(owner) || self.package.body(*id).is_some() {
                return Err(error("missing or duplicate body owner", owner));
            }
            if body.value.hir_id.owner != owner {
                return Err(error("body belongs to a different owner", owner));
            }
            for parameter in body.params {
                charge(fuel)?;
                if parameter.hir_id.owner != owner {
                    return Err(error("body parameter belongs to a different owner", owner));
                }
            }
        }
        for (index, definition) in self.definitions.iter().enumerate() {
            charge(fuel)?;
            let owner = OwnerId::new(LocalDefId::new((self.base + index) as u32));
            let definition = definition
                .as_ref()
                .ok_or_else(|| error("unfinished definition", owner))?;
            if self.package.owner_definition(owner).is_some()
                || self.package.lexical_parents.contains_key(&owner)
            {
                return Err(error("owner already has published metadata", owner));
            }
            let mut checked_node_parents = BTreeSet::new();
            for (local, entry) in definition.info.nodes.nodes.iter_enumerated() {
                charge(fuel)?;
                let Some(entry) = entry else { continue };
                if entry.node.hir_id() != crate::HirId::new(owner, local) {
                    return Err(error("node index does not match its identity", owner));
                }
                if local.raw() == 0 && entry.parent.raw() != 0 {
                    return Err(error("root node has a non-root parent", owner));
                }
                if local.raw() == 0
                    && !matches!(&entry.node, crate::Node::Item(item) if std::ptr::eq(*item, definition.info.node.expect_item()))
                {
                    return Err(error("root index does not refer to its definition", owner));
                }
                let mut parent = entry.parent;
                let mut seen = BTreeSet::from([local]);
                while parent.raw() != 0 {
                    charge(fuel)?;
                    if checked_node_parents.contains(&parent) {
                        break;
                    }
                    if !seen.insert(parent) {
                        return Err(error("cyclic node parents", owner));
                    }
                    parent = definition
                        .info
                        .nodes
                        .get(parent)
                        .ok_or_else(|| error("missing node parent", owner))?
                        .parent;
                }
                checked_node_parents.extend(seen);
            }
            if let Some(identity) = definition.identity {
                if !identity.is_valid()
                    || self.package.definition_owner(identity).is_some()
                    || self.package.clause_definitions.contains_key(&identity)
                    || !identities.insert(identity)
                {
                    return Err(error("invalid or duplicate definition identity", owner));
                }
            }
            if let Some(parent) = definition.lexical_parent {
                if parent == owner || !self.exists(parent) {
                    return Err(error("invalid lexical parent", owner));
                }
                let mut ancestor = Some(parent);
                let mut seen = BTreeSet::from([owner]);
                while let Some(id) = ancestor {
                    charge(fuel)?;
                    if checked_lexical_parents.contains(&id) {
                        break;
                    }
                    if !seen.insert(id) {
                        return Err(error("cyclic lexical parents", owner));
                    }
                    if !self.exists(id) {
                        return Err(error("missing lexical ancestor", owner));
                    }
                    ancestor = match self.slot(id) {
                        Some(i) => self.definitions[i].as_ref().and_then(|d| d.lexical_parent),
                        None => self.package.lexical_parents.get(&id).copied(),
                    };
                }
                checked_lexical_parents.extend(seen);
            }
            let item = definition.info.node.expect_item();
            members::validate(item, fuel)?;
            for parameter in members::clauses(item) {
                charge(fuel)?;
                if let Some(identity) = parameter.definition {
                    if self.package.definition_owner(identity).is_some()
                        || self.package.clause_definitions.contains_key(&identity)
                        || !identities.insert(identity)
                    {
                        return Err(error(
                            "clause definition identity is already declared",
                            owner,
                        ));
                    }
                }
            }
            let body = match &item.kind {
                ItemKind::Fn(_, id)
                | ItemKind::FunctionTemplateInstance { body: id, .. }
                | ItemKind::TestTemplateInstance { body: id, .. } => Some(*id),
                ItemKind::Deriver(definition) => Some(definition.body),
                ItemKind::Invalid => return Err(error("invalid definition payload", owner)),
                _ => None,
            };
            if let Some(id) = body {
                if id.hir_id.owner != owner || !self.body_slots.contains_key(&id) {
                    return Err(error(
                        "definition body is missing or has a different owner",
                        owner,
                    ));
                }
            }
            // Container edges may refer forward to another completed draft.
            // Detect cycles using an explicit worklist, not the host stack.
            let mut pending = vec![(owner, false)];
            let mut active = BTreeSet::new();
            while let Some((id, leaving)) = pending.pop() {
                charge(fuel)?;
                if leaving {
                    active.remove(&id);
                    checked_containers.insert(id);
                    continue;
                }
                if checked_containers.contains(&id) {
                    continue;
                }
                if !active.insert(id) {
                    return Err(error("cyclic definition containers", owner));
                }
                let nested = match self.slot(id) {
                    Some(i) => self.definitions[i]
                        .as_ref()
                        .map(|d| d.info.node.expect_item()),
                    None => self.package.item(id),
                }
                .ok_or_else(|| error("missing contained definition", id))?;
                pending.push((id, true));
                for child in children(&nested.kind).iter().rev() {
                    charge(fuel)?;
                    if !self.exists(*child) {
                        return Err(error("missing contained definition", *child));
                    }
                    pending.push((*child, false));
                }
            }
        }
        let mut owners = Vec::with_capacity(self.definitions.len());
        // No fallible checks, callbacks, or user code beyond this point.
        for definition in self.definitions.drain(..) {
            let definition = definition.unwrap();
            let owner = definition.info.node.expect_item().owner_id;
            self.package.insert_owner(owner, definition.info);
            if let Some(identity) = definition.identity {
                self.package.bind_definition(identity, owner);
            }
            if let Some(parent) = definition.lexical_parent {
                self.package.lexical_parents.insert(owner, parent);
            }
            owners.push(owner);
        }
        for (id, body) in self.bodies.drain(..) {
            self.package.insert_body(id, body);
        }
        self.package
            .definition_attributes
            .append(&mut self.attributes);
        self.body_slots.clear();
        self.publish_resolution();
        self.package.reserve_definition_prefix(self.identity_next);
        self.base = self.package.num_defs();
        Ok(owners)
    }
}
fn children<'a>(kind: &'a ItemKind<'_>) -> &'a [OwnerId] {
    match kind {
        ItemKind::Mod(definition) => &definition.items,
        ItemKind::Impl(definition) => &definition.items,
        ItemKind::Trait(definition) => &definition.items,
        ItemKind::Struct(definition) => &definition.nested_items,
        ItemKind::Enum(definition) => &definition.nested_items,
        ItemKind::EffectGroup(definition) => &definition.operations,
        _ => &[],
    }
}
impl fmt::Display for DefinitionBatch<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-batch (base {}) (next-definition {}) (definitions",
            self.base, self.identity_next
        )?;
        for (index, definition) in self.definitions.iter().enumerate() {
            write!(f, " (definition (owner {})", self.base + index)?;
            match definition {
                None => f.write_str(" (state unfinished)")?,
                Some(definition) => {
                    f.write_str(" (identity ")?;
                    match definition.identity {
                        Some(id) => write!(f, "(def {} {})", id.pkg, id.index)?,
                        None => f.write_str("none")?,
                    }
                    f.write_str(") (lexical-parent ")?;
                    match definition.lexical_parent {
                        Some(id) => write!(f, "{id}")?,
                        None => f.write_str("none")?,
                    }
                    write!(
                        f,
                        ") (item {}) (node-index",
                        definition.info.node.expect_item()
                    )?;
                    for (local, entry) in definition.info.nodes.nodes.iter_enumerated() {
                        if let Some(entry) = entry {
                            write!(
                                f,
                                " (node-ref (local {local}) (parent {}) (kind {:?}))",
                                entry.parent,
                                entry.node.kind_name()
                            )?;
                        }
                    }
                    f.write_str("))")?;
                    continue;
                }
            }
            f.write_str(")")?;
        }
        f.write_str(") (bodies")?;
        let mut bodies: Vec<_> = self.bodies.iter().collect();
        bodies.sort_by_key(|(id, _)| (id.hir_id.owner, id.hir_id.local_id));
        for (id, body) in bodies {
            write!(f, " (body (id {id}) {body})")?;
        }
        f.write_str(") (attributes")?;
        for attribute in &self.attributes {
            write!(f, " {attribute}")?;
        }
        f.write_str(") (resolution")?;
        let mut evidence: Vec<_> = self.resolution.iter().collect();
        evidence.sort_by_key(|(id, _)| (id.owner, id.local_id));
        for (id, value) in evidence {
            write!(f, " (use-site (id {id}) {value})")?;
        }
        f.write_str("))")
    }
}
