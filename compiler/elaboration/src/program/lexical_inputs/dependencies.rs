//! Declaration-rooted lexical reads. OwnerNodes is an index, not a complete
//! syntax tree (in particular after template rebinding). Walk the actual roots,
//! close referenced local declarations, and retain inherited proof premises.
use super::*;
use hir::clause::{ClauseConstraintKind as Constraint, ClauseParamKind as Clause};
use hir::expr::ObjectEntry;
use hir::pattern::PatternArm;
use hir::pattern::{BitSegment, PatternKind as Pattern};
use std::collections::VecDeque;

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

#[derive(Default)]
struct OwnerReads {
    bindings: BTreeSet<DeclarationId>,
    direct: BTreeSet<DeclarationId>,
    callees: BTreeSet<OwnerId>,
}

struct Reads<'a, 'c, 'h> {
    checker: &'a Checker<'c, 'h>,
    pending: Vec<(OwnerId, Node<'a, 'h>)>,
    current: OwnerId,
    expressions: BTreeSet<(OwnerId, n::LocalId)>,
    patterns: BTreeSet<(OwnerId, n::LocalId)>,
    owners: BTreeSet<OwnerId>,
    graph: BTreeMap<OwnerId, OwnerReads>,
    protocol_owners: BTreeMap<OwnerId, Vec<OwnerId>>,
    fuel: usize,
    span: Span,
}

pub(super) fn collect(
    checker: &Checker<'_, '_>,
    owner: OwnerId,
    fuel: &mut usize,
    span: Span,
) -> Result<BTreeSet<DeclarationId>, Error> {
    let mut reads = Reads {
        checker,
        pending: Vec::new(),
        owners: BTreeSet::new(),
        current: owner,
        expressions: BTreeSet::new(),
        patterns: BTreeSet::new(),
        graph: BTreeMap::new(),
        protocol_owners: BTreeMap::new(),
        fuel: *fuel,
        span,
    };
    let result =
        (|| {
            // Local protocol contributions are part of the declaration's checking
            // environment even when a method is selected without a ResolvedDef.
            // Retain their reads until selection can provide narrower proof edges.
            for (&child, &parent) in &checker.package.lexical_parents {
                reads.charge()?;
                if checker.package.item(child).is_some_and(|item| {
                    matches!(item.kind, ItemKind::Impl(_) | ItemKind::Derive(_))
                }) {
                    reads.protocol_owners.entry(parent).or_default().push(child);
                }
            }
            reads.push(Node::Owner(owner))?;
            while let Some((context, node)) = reads.pending.pop() {
                reads.current = match &node {
                    Node::Owner(owner) => *owner,
                    _ => context,
                };
                reads.node(node)?;
            }
            reads.close(owner)
        })();
    *fuel = reads.fuel;
    result
}

impl<'a, 'c, 'h> Reads<'a, 'c, 'h> {
    fn charge(&mut self) -> Result<(), Error> {
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(self.span, "lexical dependency observation budget exhausted"))?;
        Ok(())
    }
    fn push(&mut self, node: Node<'a, 'h>) -> Result<(), Error> {
        self.charge()?;
        self.pending.push((self.current, node));
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
        self.graph
            .entry(self.current)
            .or_default()
            .direct
            .insert(id);
    }
    fn term(&mut self, term: &Term) -> Result<(), Error> {
        let direct = &mut self.graph.entry(self.current).or_default().direct;
        nir::type_computations::rigid_declarations_bounded(term, &mut self.fuel, direct)
            .map_err(|e| error(self.span, e.to_string()))
    }
    fn reference(&mut self, owner: OwnerId) -> Result<(), Error> {
        self.graph
            .entry(self.current)
            .or_default()
            .callees
            .insert(owner);
        self.push(Node::Owner(owner))
    }
    /// Solve lexical reads on the declaration graph, not a union of syntax from
    /// unrelated invocation frames. A callee's own binders do not become caller
    /// captures; its argument expressions were observed in the caller already.
    /// Work-list propagation also closes recursive groups without body expansion.
    fn close(&mut self, root: OwnerId) -> Result<BTreeSet<DeclarationId>, Error> {
        let mut reverse = BTreeMap::<OwnerId, BTreeSet<OwnerId>>::new();
        let mut needed = BTreeMap::<OwnerId, BTreeSet<DeclarationId>>::new();
        for (&owner, reads) in &self.graph {
            for input in &reads.direct {
                self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
                    error(self.span, "lexical dependency closure budget exhausted")
                })?;
                if !reads.bindings.contains(input) {
                    needed.entry(owner).or_default().insert(*input);
                }
            }
            needed.entry(owner).or_default();
            for callee in &reads.callees {
                self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
                    error(self.span, "lexical dependency closure budget exhausted")
                })?;
                if !self.graph.contains_key(callee) {
                    return Err(error(self.span, "lexical callee observation is incomplete"));
                }
                reverse.entry(*callee).or_default().insert(owner);
            }
        }
        let mut queued = needed.keys().copied().collect::<BTreeSet<_>>();
        let mut pending = VecDeque::from_iter(queued.iter().copied());
        while let Some(callee) = pending.pop_front() {
            queued.remove(&callee);
            self.charge()?;
            self.fuel = self
                .fuel
                .checked_sub(needed[&callee].len())
                .ok_or_else(|| error(self.span, "lexical dependency closure budget exhausted"))?;
            let outgoing = needed[&callee].clone();
            for caller in reverse.get(&callee).into_iter().flatten() {
                let mut changed = false;
                for input in &outgoing {
                    self.charge()?;
                    if !self.graph[caller].bindings.contains(input) {
                        changed |= needed.get_mut(caller).unwrap().insert(*input);
                    }
                }
                if changed && queued.insert(*caller) {
                    pending.push_back(*caller);
                }
            }
        }
        needed
            .remove(&root)
            .ok_or_else(|| error(self.span, "missing lexical root observation"))
    }
    fn identity(&self, id: hir::common::DefId) -> DeclarationId {
        DeclarationId {
            package: semantic_package(&self.checker.package_identities, id.pkg),
            local: id.index,
        }
    }
    fn definition(&mut self, id: hir::common::DefId) -> Result<(), Error> {
        self.charge()?;
        // Global declarations cannot close over this lexical scope. Their
        // explicit applications are walked at the use site instead.
        let Some(owner) = self.checker.package.definition_owner(id) else {
            return Ok(());
        };
        let mut cursor = owner;
        let mut seen = BTreeSet::new();
        loop {
            self.charge()?;
            if !seen.insert(cursor) {
                return Err(error(self.span, "cyclic lexical declaration ownership"));
            }
            if self.checker.package.lexical_parents.contains_key(&cursor) {
                return self.reference(owner);
            }
            let Some(parent) = self.checker.self_containers.get(&cursor).copied() else {
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
        while let Some(parent) = self.checker.package.lexical_parents.get(&cursor).copied() {
            self.charge()?;
            if !seen.insert(parent) {
                return Err(error(self.span, "cyclic lexical proof environment"));
            }
            self.protocols(parent)?;
            let item = self
                .checker
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
                    .checker
                    .package
                    .body(id)
                    .ok_or_else(|| error(self.span, "missing lexical dependency body"))?;
                for parameter in body.params {
                    self.push(Node::Parameter(&parameter.ty.kind))?;
                }
                self.expression(body.value)?;
            }
            Node::Signature(sig) => {
                for parameter in sig.params {
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
        let count = self.protocol_owners.get(&owner).map_or(0, Vec::len);
        for index in 0..count {
            let member = self.protocol_owners[&owner][index];
            self.reference(member)?;
        }
        Ok(())
    }
    fn owner(&mut self, owner: OwnerId) -> Result<(), Error> {
        if !self.owners.insert(owner) {
            return Ok(());
        }
        self.inherited(owner)?;
        self.protocols(owner)?;
        if let Some(ty) = self.checker.self_types.get(&owner) {
            self.term(ty)?;
        }
        let package = self.checker.package;
        let item = package
            .item(owner)
            .ok_or_else(|| error(self.span, "missing lexical dependency declaration"))?;
        // Only source-declared binders are bound here. The provisional lexical
        // telescope also contains captures and must not mask those free reads.
        for parameter in hir::definitions::declared_clause_parameters(item) {
            self.charge()?;
            if let Some(id) = parameter.definition {
                let id = self.identity(id);
                self.graph.entry(owner).or_default().bindings.insert(id);
            }
        }
        if let ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) = &item.kind {
            for parameter in sig.params {
                self.charge()?;
                if let Some(id) = package.staged_parameter_definitions.get(&parameter.hir_id) {
                    let id = self.identity(*id);
                    self.graph.entry(owner).or_default().bindings.insert(id);
                }
            }
        }
        self.graph.entry(owner).or_default();
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
        if !self.expressions.insert((self.current, local(expr.hir_id))) {
            return Ok(());
        }
        let package = self.checker.package;
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
                self.input(self.identity(*id));
            }
            H::ResolvedDef { def_id, .. } => self.definition(*def_id)?,
            H::ResolvedFnParam(_, binding) | H::ResolvedLocal(_, binding) => {
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
        if !self.patterns.insert((self.current, local(pattern.hir_id))) {
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
