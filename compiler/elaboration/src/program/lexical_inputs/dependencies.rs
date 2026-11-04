//! Declaration-rooted lexical reads. OwnerNodes is an index, not a complete
//! syntax tree (in particular after template rebinding). Walk the actual roots,
//! close referenced local declarations, and retain inherited proof premises.
use super::dependency_closure::{ScopeReads, close};
use super::*;
use hir::clause::{ClauseConstraintKind as Constraint, ClauseParamKind as Clause};
use hir::expr::ObjectEntry;
use hir::pattern::PatternArm;
use hir::pattern::{BitSegment, PatternKind as Pattern};

// Every edge is charged before enqueueing, so a wide source node cannot allocate
// an unbounded work list before discovering that its observation budget ran out.
enum Node<'a, 'h> {
    Expr(&'a Expr<'h>),
    Pattern(&'a hir::Pattern<'h>),
    Block(&'a hir::Block<'h>),
    Body(hir::BodyId),
    Owner(OwnerId),
    Signature(&'a hir::FnSig<'h>),
    Parameter(&'a TyParamKind<'h>),
    Clause(&'a Clause<'h>),
    Variant(&'a hir::item::Variant<'h>),
}

/// The expression being observed is not its enclosing declaration. In
/// particular, a recursive reference must not bind the expression's inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SourceScope {
    Expression,
    Declaration(OwnerId),
}

/// One query purpose selects the result and delivered-scope requirements.
/// Declaration formation observes its own telescope; expression queries retain
/// that telescope as external inputs instead of binding it away.
#[derive(Clone, Copy)]
enum Query<'a> {
    Declaration,
    FreeLocals,
    FreeDeclarations,
    MayUse(&'a BTreeSet<n::LocalId>),
}

/// Only immutable source and formed declaration context are observable here.
/// Keeping VM slots and the rest of Checker out prevents dependency discovery
/// from accidentally executing a supplier or inspecting active call state.
#[derive(Clone, Copy)]
struct Source<'a, 'h> {
    package: &'a hir::Package<'h>,
    identities: &'a BTreeMap<u32, u64>,
    self_types: &'a BTreeMap<OwnerId, Term>,
    containers: &'a BTreeMap<OwnerId, OwnerId>,
    protocols: &'a SourceProtocolIndex<'a, 'h>,
}

impl<'a, 'h> Source<'a, 'h> {
    fn from_checker(checker: &'a Checker<'_, 'h>) -> Self {
        Self {
            package: checker.package,
            identities: &checker.package_identities,
            self_types: &checker.self_types,
            containers: &checker.self_containers,
            protocols: &checker.lexical_source_protocols,
        }
    }
}

struct Reads<'a, 'h> {
    source: Source<'a, 'h>,
    pending: Vec<(OwnerId, SourceScope, Node<'a, 'h>)>,
    current: OwnerId,
    scope: SourceScope,
    expressions: BTreeSet<(SourceScope, OwnerId, n::LocalId)>,
    patterns: BTreeSet<(SourceScope, OwnerId, n::LocalId)>,
    owners: BTreeSet<OwnerId>,
    graph: BTreeMap<SourceScope, ScopeReads<SourceScope, DeclarationId>>,
    query: Query<'a>,
    reads_local_input: bool,
    local_graph: BTreeMap<SourceScope, ScopeReads<SourceScope, n::LocalId>>,
    staged_locals: BTreeMap<DeclarationId, n::LocalId>,
    fuel: usize,
    span: Span,
}

pub(super) fn collect(
    checker: &Checker<'_, '_>,
    owner: OwnerId,
    fuel: &mut usize,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    let mut reads = Reads::new(Source::from_checker(checker), owner, fuel, span);
    let result = (|| {
        reads.push(Node::Owner(owner))?;
        reads.drain()?;
        close(
            &reads.graph,
            SourceScope::Declaration(owner),
            &mut reads.fuel,
            span,
        )
    })();
    *fuel = reads.fuel;
    result
}

/// Observe free declaration inputs of an expression. This shares declaration
/// closure with formation, but must not bind the enclosing function's inputs.
pub(super) fn expression_declaration_dependencies(
    checker: &Checker<'_, '_>,
    owner: OwnerId,
    value: &Expr<'_>,
    fuel: &mut usize,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    let mut reads = Reads::new(Source::from_checker(checker), owner, fuel, span);
    reads.query = Query::FreeDeclarations;
    reads.graph.entry(SourceScope::Expression).or_default();
    let result = (|| {
        reads.inherited(owner)?;
        reads.protocols(owner)?;
        reads.expression(value)?;
        reads.drain()?;
        close(&reads.graph, SourceScope::Expression, &mut reads.fuel, span)
    })();
    *fuel = reads.fuel;
    result
}

/// Observe the complete expression and referenced local declarations before
/// claiming that a pending input domain is irrelevant. This query never invokes
/// a producer or stores the caller's active context as declaration metadata.
pub(super) fn expression_may_use_inputs(
    checker: &Checker<'_, '_>,
    owner: OwnerId,
    value: &Expr<'_>,
    inputs: &BTreeSet<n::LocalId>,
    fuel: &mut usize,
    span: Span,
) -> Result<bool, Error> {
    let mut reads = Reads::new(Source::from_checker(checker), owner, fuel, span);
    reads.query = Query::MayUse(inputs);
    let result = (|| {
        reads.inherited(owner)?;
        reads.protocols(owner)?;
        reads.expression(value)?;
        reads.drain()?;
        Ok(reads.reads_local_input)
    })();
    *fuel = reads.fuel;
    result
}

/// Collect free source-frame inputs for invocation preparation. Scoped delivered
/// terms need an explicit binder mapping; they cannot be treated as no reads.
pub(super) fn expression_local_dependencies(
    checker: &Checker<'_, '_>,
    owner: OwnerId,
    value: &Expr<'_>,
    fuel: &mut usize,
    span: Span,
) -> Result<BTreeSet<n::LocalId>, Error> {
    let mut reads = Reads::new(Source::from_checker(checker), owner, fuel, span);
    reads.query = Query::FreeLocals;
    reads
        .local_graph
        .entry(SourceScope::Expression)
        .or_default();
    let result = (|| {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            checker.package.item(owner).map(|item| &item.kind)
        else {
            return Err(error(
                span,
                "parameter dependencies require a function owner",
            ));
        };
        // Only this source telescope can contribute invocation slots. Do not
        // rebuild a reverse index for every staged parameter in the package.
        for parameter in sig.params {
            reads.charge()?;
            if let Some(declaration) = checker
                .package
                .staged_parameter_definitions
                .get(&parameter.hir_id)
            {
                reads
                    .staged_locals
                    .insert(reads.identity(*declaration), local(parameter.hir_id));
            }
        }
        reads.inherited(owner)?;
        reads.protocols(owner)?;
        reads.expression(value)?;
        reads.drain()?;
        close(
            &reads.local_graph,
            SourceScope::Expression,
            &mut reads.fuel,
            span,
        )
    })();
    *fuel = reads.fuel;
    result
}

impl<'a, 'h> Reads<'a, 'h> {
    fn new(source: Source<'a, 'h>, owner: OwnerId, fuel: &usize, span: Span) -> Self {
        Self {
            source,
            pending: Vec::new(),
            owners: BTreeSet::new(),
            current: owner,
            scope: SourceScope::Expression,
            expressions: BTreeSet::new(),
            patterns: BTreeSet::new(),
            graph: BTreeMap::new(),
            query: Query::Declaration,
            reads_local_input: false,
            local_graph: BTreeMap::new(),
            staged_locals: BTreeMap::new(),
            fuel: *fuel,
            span,
        }
    }
    fn drain(&mut self) -> Result<(), Error> {
        while let Some((context, scope, node)) = self.pending.pop() {
            self.scope = match &node {
                Node::Owner(owner) => SourceScope::Declaration(*owner),
                _ => scope,
            };
            self.current = match &node {
                Node::Owner(owner) => *owner,
                _ => context,
            };
            self.node(node)?;
        }
        Ok(())
    }
    fn charge(&mut self) -> Result<(), Error> {
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(self.span, "lexical dependency observation budget exhausted"))?;
        Ok(())
    }
    fn push(&mut self, node: Node<'a, 'h>) -> Result<(), Error> {
        self.charge()?;
        self.pending.push((self.current, self.scope, node));
        Ok(())
    }
    fn expression(&mut self, expr: &'a Expr<'h>) -> Result<(), Error> {
        self.push(Node::Expr(expr))
    }
    fn optional(&mut self, expr: Option<&'a Expr<'h>>) -> Result<(), Error> {
        if let Some(expr) = expr {
            self.expression(expr)?;
        }
        Ok(())
    }
    fn input(&mut self, id: DeclarationId) {
        if let Some(binding) = self.staged_locals.get(&id) {
            self.read_local(*binding);
        }
        self.graph.entry(self.scope).or_default().direct.insert(id);
    }
    fn term(&mut self, term: &Term) -> Result<(), Error> {
        if !matches!(self.query, Query::Declaration) {
            // A delivered term may carry a scope that is not the source local
            // frame. Without its mapping, absence of a Local read is no proof
            // that the pending inputs are irrelevant.
            nir::type_computations::charge_term(term, &mut self.fuel)
                .map_err(|failure| error(self.span, failure.to_string()))?;
            if !term
                .substitution_parameters()
                .map_err(|failure| error(self.span, failure.to_string()))?
                .is_empty()
            {
                if matches!(self.query, Query::FreeLocals | Query::FreeDeclarations) {
                    return Err(error(
                        self.span,
                        "source dependency requires a delivered scope mapping",
                    ));
                }
                self.reads_local_input = true;
            }
        }
        let mut symbolic = BTreeSet::new();
        let direct = if !matches!(self.query, Query::Declaration) {
            &mut symbolic
        } else {
            &mut self.graph.entry(self.scope).or_default().direct
        };
        nir::type_computations::rigid_declarations_bounded(term, &mut self.fuel, direct)
            .map_err(|e| error(self.span, e.to_string()))?;
        if !symbolic.is_empty() {
            // Rigid declarations also require a declaration-to-local mapping.
            // Until that evidence is available, retain the pending context.
            self.reads_local_input = true;
            for declaration in symbolic {
                self.input(declaration);
            }
        }
        Ok(())
    }
    fn reference(&mut self, owner: OwnerId) -> Result<(), Error> {
        if matches!(self.query, Query::FreeLocals) {
            self.local_graph
                .entry(self.scope)
                .or_default()
                .callees
                .insert(SourceScope::Declaration(owner));
        }
        self.graph
            .entry(self.scope)
            .or_default()
            .callees
            .insert(SourceScope::Declaration(owner));
        self.push(Node::Owner(owner))
    }
    fn read_local(&mut self, binding: n::LocalId) {
        if matches!(self.query, Query::FreeLocals) {
            self.local_graph
                .entry(self.scope)
                .or_default()
                .direct
                .insert(binding);
        }
    }
    fn bind_local(&mut self, binding: hir::HirId) {
        if matches!(self.query, Query::FreeLocals) {
            self.local_graph
                .entry(self.scope)
                .or_default()
                .bindings
                .insert(local(binding));
        }
    }
    fn identity(&self, id: hir::common::DefId) -> DeclarationId {
        DeclarationId {
            package: semantic_package(self.source.identities, id.pkg),
            local: id.index,
        }
    }
    fn definition(&mut self, id: hir::common::DefId) -> Result<(), Error> {
        self.charge()?;
        // Global declarations cannot close over this lexical scope. Their
        // explicit applications are walked at the use site instead.
        let Some(owner) = self.source.package.definition_owner(id) else {
            return Ok(());
        };
        let mut cursor = owner;
        let mut seen = BTreeSet::new();
        loop {
            self.charge()?;
            if !seen.insert(cursor) {
                return Err(error(self.span, "cyclic lexical declaration ownership"));
            }
            if self.source.package.lexical_parents.contains_key(&cursor) {
                return self.reference(owner);
            }
            let Some(parent) = self.source.containers.get(&cursor).copied() else {
                break;
            };
            cursor = parent;
        }
        Ok(())
    }
    fn arguments(&mut self, args: &'a [Arg<'h>]) -> Result<(), Error> {
        for arg in args {
            let (Arg::Positional(value)
            | Arg::Named(_, value)
            | Arg::Expand(value)
            | Arg::Implicit(value)
            | Arg::PostLambda(value)) = arg;
            self.expression(value)?;
        }
        Ok(())
    }
    fn clauses(
        &mut self,
        parameters: &'a [hir::ClauseParam<'h>],
        constraints: &'a [hir::ClauseConstraint<'h>],
    ) -> Result<(), Error> {
        for parameter in parameters {
            self.push(Node::Clause(&parameter.kind))?;
        }
        self.constraints(parameters, constraints)
    }
    fn constraints(
        &mut self,
        parameters: &'a [hir::ClauseParam<'h>],
        constraints: &'a [hir::ClauseConstraint<'h>],
    ) -> Result<(), Error> {
        for parameter in parameters {
            self.charge()?;
            let mut kind = &parameter.kind;
            while let Clause::Quote(inner) = kind {
                self.charge()?;
                kind = inner;
            }
            if let Clause::TraitBound(_, bound) = kind {
                if let Some(id) = parameter.definition {
                    self.input(self.identity(id));
                }
                self.expression(bound)?;
            }
        }
        for constraint in constraints {
            match constraint.kind {
                Constraint::Requires(value)
                | Constraint::Ensures(value)
                | Constraint::Decreases(value) => self.expression(value)?,
                Constraint::Outcome => {
                    return Err(error(
                        constraint.span,
                        "lexical dependency has an unformed outcome contract",
                    ));
                }
            }
        }
        Ok(())
    }
    fn inherited(&mut self, owner: OwnerId) -> Result<(), Error> {
        let mut cursor = owner;
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.source.package.lexical_parents.get(&cursor).copied() {
            self.charge()?;
            if !seen.insert(parent) {
                return Err(error(self.span, "cyclic lexical proof environment"));
            }
            self.protocols(parent)?;
            let item = self
                .source
                .package
                .item(parent)
                .ok_or_else(|| error(self.span, "missing lexical parent declaration"))?;
            match &item.kind {
                ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => {
                    self.constraints(sig.clause_params, sig.clause_constraints)?
                }
                _ => {
                    return Err(error(
                        item.span,
                        "lexical dependency requires a formed parent interface",
                    ));
                }
            }
            cursor = parent;
        }
        Ok(())
    }
    fn fields(&mut self, fields: &'a [hir::FieldDef<'h>]) -> Result<(), Error> {
        for field in fields {
            self.expression(field.ty)?;
            self.optional(field.default)?;
        }
        Ok(())
    }
    fn members(&mut self, members: &[OwnerId]) -> Result<(), Error> {
        for member in members {
            self.reference(*member)?;
        }
        Ok(())
    }
    fn arm(&mut self, arm: &'a PatternArm<'h>) -> Result<(), Error> {
        self.push(Node::Pattern(&arm.pat))?;
        self.optional(arm.guard)?;
        self.expression(arm.body)
    }
    fn node(&mut self, node: Node<'a, 'h>) -> Result<(), Error> {
        match node {
            Node::Owner(owner) => self.owner(owner)?,
            Node::Body(id) => {
                let body = self
                    .source
                    .package
                    .body(id)
                    .ok_or_else(|| error(self.span, "missing lexical dependency body"))?;
                for parameter in body.params {
                    self.bind_local(parameter.hir_id);
                    self.push(Node::Parameter(&parameter.ty.kind))?;
                }
                self.expression(body.value)?;
            }
            Node::Signature(sig) => {
                if let Some(binding) = sig.return_binding {
                    self.bind_local(binding);
                }
                for parameter in sig.params {
                    self.bind_local(parameter.hir_id);
                    self.push(Node::Parameter(&parameter.ty.kind))?;
                }
                self.optional(sig.return_ty)?;
                self.clauses(sig.clause_params, sig.clause_constraints)?;
            }
            Node::Parameter(kind) => match kind {
                TyParamKind::Positional(value)
                | TyParamKind::PositionalDependencyCatched(_, value)
                | TyParamKind::Varadic(_, value) => self.expression(value)?,
                TyParamKind::Optional(_, ty, default) => {
                    self.expression(ty)?;
                    self.expression(default)?;
                }
                TyParamKind::Itself { .. } => {}
            },
            Node::Clause(kind) => match kind {
                Clause::Type(_) => {}
                Clause::Positional(_, ty) | Clause::Varadic(_, ty) | Clause::TraitBound(_, ty) => {
                    self.expression(ty)?
                }
                Clause::Optional(_, ty, default) => {
                    self.expression(ty)?;
                    self.expression(default)?;
                }
                Clause::Quote(inner) => self.push(Node::Clause(inner))?,
            },
            Node::Block(block) => {
                for expr in block.stmts {
                    self.expression(expr)?;
                }
                self.optional(block.expr)?;
            }
            Node::Expr(expr) => self.expr(expr)?,
            Node::Pattern(pattern) => self.pattern(pattern)?,
            Node::Variant(variant) => match &variant.kind {
                hir::item::VariantKind::Unit => {}
                hir::item::VariantKind::Pattern(pattern) => self.push(Node::Pattern(pattern))?,
                hir::item::VariantKind::Const(value) => self.expression(value)?,
                hir::item::VariantKind::Tuple(fields) => {
                    for field in *fields {
                        self.expression(field)?;
                    }
                }
                hir::item::VariantKind::Struct(fields) => self.fields(fields)?,
                hir::item::VariantKind::SubEnum(variants) => {
                    for variant in *variants {
                        self.push(Node::Variant(variant))?;
                    }
                }
            },
        }
        Ok(())
    }
    fn protocols(&mut self, owner: OwnerId) -> Result<(), Error> {
        let members = self
            .source
            .protocols
            .for_parent(owner, &mut self.fuel, self.span)?;
        for &member in members.into_iter().flatten() {
            // The structural edge is shared; its declarations and premises
            // are still observed in this query's own lexical/proof context.
            self.reference(member)?;
        }
        Ok(())
    }
    fn owner(&mut self, owner: OwnerId) -> Result<(), Error> {
        if !self.owners.insert(owner) {
            return Ok(());
        }
        if matches!(self.query, Query::FreeLocals) {
            self.local_graph
                .entry(SourceScope::Declaration(owner))
                .or_default();
        }
        self.inherited(owner)?;
        self.protocols(owner)?;
        if let Some(ty) = self.source.self_types.get(&owner) {
            self.term(ty)?;
        }
        let package = self.source.package;
        let item = package
            .item(owner)
            .ok_or_else(|| error(self.span, "missing lexical dependency declaration"))?;
        // Only source-declared binders are bound here. The provisional lexical
        // telescope also contains captures and must not mask those free reads.
        for parameter in hir::definitions::declared_clause_parameters(item) {
            self.charge()?;
            if let Some(id) = parameter.definition {
                let id = self.identity(id);
                self.graph
                    .entry(SourceScope::Declaration(owner))
                    .or_default()
                    .bindings
                    .insert(id);
            }
        }
        if let ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) = &item.kind {
            for parameter in sig.params {
                self.charge()?;
                if let Some(id) = package.staged_parameter_definitions.get(&parameter.hir_id) {
                    let id = self.identity(*id);
                    self.graph
                        .entry(SourceScope::Declaration(owner))
                        .or_default()
                        .bindings
                        .insert(id);
                }
            }
        }
        self.graph
            .entry(SourceScope::Declaration(owner))
            .or_default();
        for attribute in &package.definition_attributes {
            self.charge()?;
            if matches!(attribute.target, hir::AttributeTarget::Definition(id) if id == owner)
                || matches!(attribute.target, hir::AttributeTarget::Member(id) if id.owner == owner)
            {
                self.expression(attribute.value)?;
            }
        }
        if let Some(application) = package.function_template_applications.get(&owner) {
            self.expression(application.template)?;
        }
        if let Some(body) = package.function_template_bodies.get(&owner) {
            self.expression(body.interface.ty)?;
            if let Some(body) = body.supplied {
                self.push(Node::Body(body))?;
            }
        }
        match &item.kind {
            ItemKind::Fn(sig, body) => {
                self.push(Node::Signature(sig))?;
                self.push(Node::Body(*body))?;
            }
            ItemKind::FnDeclaration(sig) | ItemKind::Effect(sig, _) => {
                self.push(Node::Signature(sig))?
            }
            ItemKind::TypeAlias(value, parameters, constraints)
            | ItemKind::Newtype(value, parameters, constraints) => {
                self.expression(value)?;
                self.clauses(parameters, constraints)?;
            }
            ItemKind::Const(ty, value) => {
                self.expression(ty)?;
                self.expression(value)?;
            }
            ItemKind::Struct(def) => {
                self.fields(def.fields)?;
                self.clauses(def.clause_params, def.clause_constraints)?;
                self.members(&def.nested_items)?;
            }
            ItemKind::Enum(def) => {
                self.clauses(def.clause_params, def.clause_constraints)?;
                self.members(&def.nested_items)?;
                for variant in def.variants {
                    self.push(Node::Variant(variant))?;
                }
                for config in &def.configurations {
                    self.expression(config.value)?;
                }
            }
            ItemKind::Trait(def) => {
                self.optional(def.super_traits)?;
                self.clauses(def.clause_params, def.clause_constraints)?;
                self.members(&def.items)?;
            }
            ItemKind::Impl(def) => {
                self.expression(def.self_ty)?;
                self.optional(def.trait_ref)?;
                self.optional(def.generation_trait)?;
                self.clauses(def.clause_params, def.clause_constraints)?;
                self.members(&def.items)?;
            }
            ItemKind::Associated(def) => {
                self.expression(def.ty)?;
                self.optional(def.default)?;
                for parameter in def.parameters {
                    self.bind_local(parameter.hir_id);
                    self.push(Node::Parameter(&parameter.ty.kind))?;
                }
                self.clauses(def.clause_params, def.clause_constraints)?;
            }
            ItemKind::EffectGroup(def) => {
                self.clauses(def.clause_params, def.clause_constraints)?;
                self.members(&def.operations)?;
            }
            ItemKind::Deriver(def) => {
                for parameter in def.parameters {
                    self.bind_local(parameter.hir_id);
                    self.push(Node::Parameter(&parameter.ty.kind))?;
                }
                self.expression(def.trait_ref)?;
                self.clauses(def.clause_params, def.clause_constraints)?;
                self.push(Node::Body(def.body))?;
            }
            ItemKind::Derive(def) => {
                self.expression(def.trait_ref)?;
                self.expression(def.self_ty)?;
                self.optional(def.strategy)?;
                self.clauses(def.clause_params, def.clause_constraints)?;
            }
            ItemKind::FunctionTemplateInstance { .. } | ItemKind::TestTemplateInstance { .. } => {
                return Err(error(
                    item.span,
                    "lexical dependency awaits template expansion",
                ));
            }
            ItemKind::Mod(def) => self.members(&def.items)?,
            ItemKind::Use(_) => {}
            ItemKind::Invalid => {
                return Err(error(item.span, "invalid lexical dependency declaration"));
            }
        }
        Ok(())
    }
    fn expr(&mut self, expr: &'a Expr<'h>) -> Result<(), Error> {
        if !self
            .expressions
            .insert((self.scope, self.current, local(expr.hir_id)))
        {
            return Ok(());
        }
        let package = self.source.package;
        if let Some(id) = package.member_projections.get(&expr.hir_id) {
            self.definition(*id)?;
        }
        if let Some(members) = package.deferred_member_projections.get(&expr.hir_id) {
            for (container, member) in members.iter() {
                self.definition(*container)?;
                self.definition(*member)?;
            }
        }
        if let Some(sources) = package.trait_sources.get(&expr.hir_id) {
            sources
                .charge(&mut self.fuel)
                .map_err(|message| error(expr.span, message))?;
            if !sources.pending_exports.is_empty()
                || sources
                    .names
                    .iter()
                    .any(|name| !name.pending_exports.is_empty())
            {
                return Err(error(
                    expr.span,
                    "lexical dependency awaits generated trait sources",
                ));
            }
            for name in &sources.names {
                if let Ok(Some(id)) = name.fallback {
                    self.definition(id)?;
                }
                for id in name.ambiguous_traits.iter().flatten() {
                    self.definition(*id)?;
                }
            }
        }
        match &expr.kind {
            H::DeliveredType(ty) => self.term(ty)?,
            H::ResolvedClauseParam(_, id) => {
                // The query's candidate IDs belong to source local frames.
                // A clause identity can be a transported staged input; without
                // that mapping it cannot establish independence from them.
                if matches!(self.query, Query::MayUse(_)) {
                    self.reads_local_input = true;
                }
                self.input(self.identity(*id));
            }
            H::ResolvedDef { def_id, .. } => self.definition(*def_id)?,
            H::ResolvedFnParam(_, binding) | H::ResolvedLocal(_, binding) => {
                self.read_local(local(*binding));
                if let Query::MayUse(inputs) = self.query
                    && inputs.contains(&local(*binding))
                {
                    self.reads_local_input = true;
                }
                if let Some(id) = package.staged_parameter_definitions.get(binding) {
                    self.input(self.identity(*id));
                }
                if let Some(item) = package.item(binding.owner) {
                    if let ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) = &item.kind {
                        for parameter in sig.params {
                            self.charge()?;
                            if parameter.hir_id == *binding {
                                self.push(Node::Parameter(&parameter.ty.kind))?;
                            }
                        }
                    }
                }
            }
            H::Path(path) => {
                let id = path
                    .res
                    .ok_or_else(|| error(expr.span, "lexical dependency has an unresolved path"))?;
                self.definition(id)?;
                for segment in path.segments {
                    self.arguments(segment.args)?;
                }
            }
            H::CodeQuote(value, substitutions) | H::CodeSplice(value, substitutions) => {
                self.expression(value)?;
                for substitution in substitutions {
                    self.expression(substitution.binding)?;
                }
            }
            H::CodePatternQuote(pattern, substitutions) => {
                self.push(Node::Pattern(pattern))?;
                for substitution in substitutions {
                    self.expression(substitution.binding)?;
                }
            }
            H::ContextualExprType(bindings, value) | H::ContextualPatternType(value, bindings) => {
                self.expression(value)?;
                for binding in bindings {
                    self.bind_local(binding.hir_id);
                    self.expression(binding.ty)?;
                    self.optional(binding.default)?;
                }
            }
            H::LiteralConversion { literal, .. } => self.expression(literal)?,
            H::ConstantObjectEntry(value, _)
            | H::TacticArgument(value)
            | H::Unary(_, value)
            | H::EffectRequest(value)
            | H::Assumes(value)
            | H::Defer(value)
            | H::InlineControl(value)
            | H::OptionPropagate(value)
            | H::Projection(value, _)
            | H::Ref(value)
            | H::Deref(value)
            | H::ErrorNew(value)
            | H::ErrorPropagate(value)
            | H::Semi(value)
            | H::TyPtr(value)
            | H::TyOptional(value)
            | H::TakeOld(value)
            | H::TakeBitVec(value)
            | H::TakeTagName(value)
            | H::TakeIntrinsic(value)
            | H::TakeTupleEnumerate(value)
            | H::TypeOf(value)
            | H::MultiDeref(value) => self.expression(value)?,
            H::PostLambdaBody(body) => self.push(Node::Body(*body))?,
            H::Index(a, b)
            | H::Binary(_, a, b)
            | H::Assign(a, b)
            | H::AssignOp(_, a, b)
            | H::Cast(a, b)
            | H::TyClosure(a, b)
            | H::TyFnArrow(a, b)
            | H::ErrorQualifiedType(a, b)
            | H::EffectQualifiedType(a, b)
            | H::TraitBound(a, b) => {
                self.expression(a)?;
                self.expression(b)?;
            }
            H::Range { start, end, .. } => {
                self.optional(*start)?;
                self.optional(*end)?;
            }
            H::Application(head, args)
            | H::ExtendedApplication(head, args)
            | H::NFApplication(head, args) => {
                self.expression(head)?;
                self.arguments(args)?;
            }
            H::CollectedProperties(fields) => {
                for field in *fields {
                    self.expression(field.expr)?;
                }
            }
            H::If(cond, body, other) => {
                self.expression(cond)?;
                self.push(Node::Block(body))?;
                self.optional(*other)?;
            }
            H::When(arms) => {
                for arm in *arms {
                    self.optional(arm.cond)?;
                    self.expression(arm.body)?;
                }
            }
            H::Block(block) | H::UnsafeBlock(block) | H::Loop(block) => {
                self.push(Node::Block(block))?
            }
            H::Match(value, arms) | H::EffectHandle(value, arms) => {
                self.expression(value)?;
                for arm in *arms {
                    self.arm(arm)?;
                }
            }
            H::RefinedType { requirement, base } => {
                self.expression(requirement)?;
                self.expression(base)?;
            }
            H::Return(value) | H::Resume(value) => self.optional(*value)?,
            H::Tuple(values) | H::List(values) => {
                for value in *values {
                    self.expression(value)?;
                }
            }
            H::Object(entries) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Value(value) => self.expression(value)?,
                        ObjectEntry::Property(field) => self.expression(field.expr)?,
                    }
                }
            }
            H::ErrorHandle {
                value,
                before,
                arms,
                after,
            } => {
                self.expression(value)?;
                if let Some(arm) = before {
                    self.arm(arm)?;
                }
                for arm in *arms {
                    self.arm(arm)?;
                }
                if let Some(arm) = after {
                    self.arm(arm)?;
                }
            }
            H::Closure(parameters, annotation, body) => {
                for parameter in *parameters {
                    self.bind_local(parameter.hir_id);
                    self.push(Node::Pattern(&parameter.pat))?;
                    self.optional(parameter.ty)?;
                }
                self.optional(*annotation)?;
                self.push(Node::Body(*body))?;
            }
            H::Let(binding) => {
                self.push(Node::Pattern(&binding.pattern))?;
                self.optional(binding.ty)?;
                self.optional(binding.init)?;
                self.optional(binding.guard)?;
                self.optional(binding.otherwise)?;
            }
            // A local declaration occurrence has no value evaluation. Its
            // signature/body remains checked separately; references demand it.
            H::Item(_) => {}
            H::For {
                pat,
                guard,
                iter,
                body,
                ..
            } => {
                self.push(Node::Pattern(pat))?;
                self.optional(*guard)?;
                self.expression(iter)?;
                self.push(Node::Block(body))?;
            }
            H::InlineIf {
                cond,
                then_expr,
                else_expr,
            } => {
                self.expression(cond)?;
                self.expression(then_expr)?;
                self.optional(*else_expr)?;
            }
            H::InlineMatch(arms) => {
                for arm in *arms {
                    self.arm(arm)?;
                }
            }
            H::InlineFor {
                pat, iter, body, ..
            } => {
                self.push(Node::Pattern(pat))?;
                self.expression(iter)?;
                self.expression(body)?;
            }
            H::TyClosureEnvironment(bindings) => {
                for binding in *bindings {
                    self.expression(binding.ty)?;
                    self.optional(binding.value)?;
                }
            }
            H::TyFn(parameters)
            | H::TyComptimeFn(parameters)
            | H::TyNFFn(parameters)
            | H::TyModifiedFn(_, parameters) => {
                for parameter in *parameters {
                    self.bind_local(parameter.hir_id);
                    self.push(Node::Parameter(&parameter.kind))?;
                }
            }
            H::UnresolvedIdent(_) | H::Invalid => {
                return Err(error(
                    expr.span,
                    "lexical dependency contains unresolved source",
                ));
            }
            H::Lit(_)
            | H::TacticParameter(_)
            | H::ResolvedLoopLabel(..)
            | H::SelfValue
            | H::RefinementSubject
            | H::Break(_)
            | H::BreakTo(_)
            | H::Continue(_)
            | H::ContinueTo(_)
            | H::Undefined
            | H::Null
            | H::Unit
            | H::ReachabilityType
            | H::TyPlaceholder
            | H::TyNoReturn
            | H::TyVoid
            | H::TyAny
            | H::TyType
            | H::TySelf
            | H::TermTypedWith
            | H::LambdaBound
            | H::TermTraitBound
            | H::Implication
            | H::Subtype
            | H::Forall
            | H::Exist
            | H::Builtin(_) => {}
        }
        Ok(())
    }
    fn pattern(&mut self, pattern: &'a hir::Pattern<'h>) -> Result<(), Error> {
        if !self
            .patterns
            .insert((self.scope, self.current, local(pattern.hir_id)))
        {
            return Ok(());
        }
        match &pattern.kind {
            Pattern::CodeSplice(value, _) | Pattern::Const(value) | Pattern::Comptime(value) => {
                self.expression(value)?
            }
            Pattern::Then(a, value, b) => {
                self.push(Node::Pattern(a))?;
                self.expression(value)?;
                self.push(Node::Pattern(b))?;
            }
            Pattern::Guard(inner, guard) => {
                self.push(Node::Pattern(inner))?;
                self.expression(guard)?;
            }
            Pattern::Projection(inner, _)
            | Pattern::OptionSome(inner)
            | Pattern::ErrorOk(inner)
            | Pattern::ErrorErr(inner)
            | Pattern::Not(inner)
            | Pattern::Ref(inner) => self.push(Node::Pattern(inner))?,
            Pattern::Binding(_, _, inner) => {
                self.bind_local(pattern.hir_id);
                if let Some(inner) = inner {
                    self.push(Node::Pattern(inner))?;
                }
            }
            Pattern::Tuple(values) | Pattern::Or(values) => {
                for value in *values {
                    self.push(Node::Pattern(value))?;
                }
            }
            Pattern::NormalForm(head, args) => {
                self.expression(head)?;
                for arg in *args {
                    self.push(Node::Pattern(&arg.pattern))?;
                }
            }
            Pattern::Struct(head, fields, _) | Pattern::AppStruct(head, fields) => {
                self.push(Node::Pattern(head))?;
                for field in *fields {
                    self.push(Node::Pattern(&field.pat))?;
                }
            }
            Pattern::List(values, rest) => {
                for value in *values {
                    self.push(Node::Pattern(value))?;
                }
                if let Some(rest) = rest {
                    self.push(Node::Pattern(rest))?;
                }
            }
            Pattern::AppTuple(head, values) => {
                self.push(Node::Pattern(head))?;
                for value in *values {
                    self.push(Node::Pattern(value))?;
                }
            }
            Pattern::Range(start, end, _) => {
                self.optional(*start)?;
                self.optional(*end)?;
            }
            Pattern::BitVec { segments, .. } => {
                for segment in segments {
                    match segment {
                        BitSegment::Typed { pattern, ty } => {
                            self.push(Node::Pattern(pattern))?;
                            self.expression(ty)?;
                        }
                        BitSegment::Computed(value) => self.expression(value)?,
                        BitSegment::Literal { .. } => {}
                    }
                }
            }
            Pattern::Wild | Pattern::OptionNull | Pattern::Async => {}
            Pattern::Invalid => {
                return Err(error(pattern.span, "invalid lexical dependency pattern"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "dependency_tests.rs"]
mod tests;
