//! Source-ordered staged suppliers and declaration-scoped defaults for open calls.
use super::pending_inputs::PendingInputs;
use super::*;

// (written parameter, writing default) in declaration order. A residual
// runtime default can be prepared after a later staged default has run.
type DefaultWrites = BTreeMap<(usize, usize), (n::Expr, Term)>;

use super::live_defaults::LiveWrites;

/// Open signature formation can receive an executed value in addition to a
/// producer and its static identity. Ordinary source checking owns no VM values.
pub(super) struct StagedInputs<'src> {
    pub(super) values: Vec<Option<n::Expr>>,
    pub(super) symbols: Vec<Option<Term>>,
    pub(super) live: Vec<Option<comptime_interpretor::Value>>,
    pub(super) pending: PendingInputs<'src>,
    /// Post-default slot contents, separate from the arguments delivered to the body.
    pub(super) default_writes: DefaultWrites,
    pub(super) live_writes: LiveWrites,
}

impl StagedInputs<'_> {
    pub(super) fn new(count: usize) -> Self {
        Self {
            values: vec![None; count],
            symbols: vec![None; count],
            live: vec![None; count],
            pending: PendingInputs::default(),
            default_writes: BTreeMap::new(),
            live_writes: BTreeMap::new(),
        }
    }
}

struct StagedCollection {
    domain: Term,
    fields: Vec<n::Expr>,
    symbols: Vec<Option<Term>>,
    values: Vec<comptime_interpretor::Value>,
}

impl Checker<'_, '_> {
    /// If signature formation only reads closed literal suppliers, the caller
    /// can retain every other operand in its own control flow and activation.
    pub(super) fn signature_independent_of_producers(
        &mut self,
        definition: &Definition,
        sig: &hir::item::FnSig<'_>,
        routes: &[arguments::RoutedArgument<'_>],
        needed: &BTreeSet<usize>,
        span: Span,
    ) -> Result<bool, Error> {
        if !sig.modifiers.is_comptime || needed.is_empty() {
            return Ok(false);
        }
        for slot in needed {
            if let Some(route) = routes.iter().find(|route| route.parameter == *slot) {
                for (_, input) in route.source_inputs() {
                    if !self.is_closed_input_literal(input)? {
                        return Ok(false);
                    }
                }
            } else {
                let TyParamKind::Optional(_, _, default) = sig.params[*slot].ty.kind else {
                    return Ok(false);
                };
                if !self.is_closed_input_literal(default)? {
                    return Ok(false);
                }
            }
        }
        for (slot, annotation) in definition
            .parameters
            .iter()
            .map(|(_, annotation)| annotation)
            .chain(std::iter::once(&definition.result))
            .enumerate()
        {
            if let Annotation::Pending(_) | Annotation::Suspended { .. } = annotation {
                let source = annotation_source(sig, slot)
                    .ok_or_else(|| error(span, "missing deferred input annotation"))?;
                if !self.annotation_only_reads_inputs(source)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    // A source-only annotation can still be inspected before any VM entry
    // exists. Follow transparent unconstrained aliases; calls, projections
    // and constructors requiring user code keep ordinary evaluation order.
    fn annotation_only_reads_inputs(&mut self, source: &Expr<'_>) -> Result<bool, Error> {
        let mut pending = vec![source];
        let mut aliases = BTreeSet::new();
        while let Some(value) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(value.span, "annotation observation budget exhausted"))?;
            match value.kind {
                H::Builtin(_)
                | H::ResolvedFnParam(..)
                | H::DeliveredType(_)
                | H::Lit(_)
                | H::TyAny
                | H::TyType
                | H::TyVoid
                | H::TyNoReturn => {}
                H::Defer(inner) | H::TyOptional(inner) | H::TyTraitObject(inner) => {
                    pending.push(inner)
                }
                H::Block(block) if block.stmts.is_empty() => pending.extend(block.expr),
                H::If(cond, yes, no) if yes.stmts.is_empty() => {
                    pending.push(cond);
                    pending.extend(yes.expr);
                    pending.extend(no);
                }
                H::ResolvedDef { def_id, .. } => {
                    let item = self
                        .package
                        .definition_owner(def_id)
                        .and_then(|owner| self.package.item(owner));
                    let Some(ItemKind::TypeAlias(target, [], [])) = item.map(|item| &item.kind)
                    else {
                        return Ok(false);
                    };
                    if aliases.insert(def_id) {
                        pending.push(target);
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    pub(super) fn open_comptime_prefix_end(
        routes: &[arguments::RoutedArgument<'_>],
        target: usize,
    ) -> usize {
        let mut end = routes
            .iter()
            .find(|route| route.parameter == target)
            .and_then(|route| route.source_inputs().map(|(position, _)| position).max())
            .unwrap_or(usize::MAX);
        // Complete every collection touched by this prefix. A collection can
        // straddle named inputs; extending the prefix must include their
        // producers too, rather than evaluating an aggregate ahead of them.
        loop {
            let previous = end;
            for route in routes {
                let mut positions = route.source_inputs().map(|(position, _)| position);
                if let Some(first) = positions.next() {
                    if first <= end {
                        end = end.max(positions.last().unwrap_or(first));
                    }
                }
            }
            if previous == end {
                break;
            }
        }
        end
    }

    // Signature dependencies may demand ordinary comptime inputs. Complete
    // their source prefix first, so no producer is replayed or overtaken.
    pub(super) fn prepare_open_comptime_prefix<'src>(
        &mut self,
        call: &Expr<'_>,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        definition: &Definition,
        routes: &[arguments::RoutedArgument<'src>],
        target: usize,
        parameters: &[Parameter],
        values: &mut [Option<n::Expr>],
        symbols: &mut [Option<Term>],
        live: &mut [Option<comptime_interpretor::Value>],
        pending: &mut PendingInputs<'src>,
    ) -> Result<BTreeMap<usize, Term>, Error> {
        let mut early_domains = BTreeMap::new();
        let end = Self::open_comptime_prefix_end(routes, target);
        let mut inputs = Vec::new();
        for (route_index, route) in routes.iter().enumerate() {
            let slot = route.parameter;
            if pending.contains(slot)
                && let Some(parameter) = parameters.get(slot)
            {
                let (expr, symbol, value) = pending.deliver(self, route, &parameter.ty, call)?;
                values[slot] = Some(expr);
                symbols[slot] = symbol;
                live[slot] = Some(value);
            }
            if values[route.parameter].is_some()
                && (symbols[route.parameter].is_some() || live[route.parameter].is_some())
            {
                continue;
            }
            for (field, (position, original)) in route.source_inputs().enumerate() {
                if position <= end {
                    inputs.push((position, route_index, field, original));
                }
            }
        }
        inputs.sort_unstable_by_key(|(position, ..)| *position);
        let mut collections = BTreeMap::<usize, StagedCollection>::new();
        for (_, route_index, field, original) in inputs {
            let route = &routes[route_index];
            let slot = route.parameter;
            let aggregate = route.expression(call);
            let input = if route.is_collected() {
                original
            } else {
                &aggregate
            };
            if pending.contains(slot) {
                pending.acquire(self, slot, field, input, original, call)?;
                continue;
            }
            if parameters.get(slot).is_none() && !collections.contains_key(&slot) {
                let available = |index: usize| symbols[index].is_some() || live[index].is_some();
                let waits = match &definition.parameters[slot].1 {
                    Annotation::Known(ty) => self
                        .term_substitution_inputs(ty, call.span)?
                        .keys()
                        .any(|i| symbols[*i].is_none()),
                    Annotation::Description(_) => false,
                    Annotation::InputType { slot } => symbols[*slot].is_none(),
                    Annotation::Pending(prepared) => prepared
                        .pending_parameters()
                        .flatten()
                        .any(|i| !available(i)),
                    Annotation::Suspended { .. } => {
                        let source = annotation_source(sig, slot).ok_or_else(|| {
                            error(call.span, "missing suspended parameter annotation")
                        })?;
                        self.expression_parameter_dependencies(owner, source)?
                            .into_iter()
                            .any(|i| i >= parameters.len() || !available(i))
                    }
                };
                if waits {
                    pending.acquire(self, slot, field, input, original, call)?;
                    continue;
                }
            }
            let ty = if let Some(packet) = collections.get(&slot) {
                packet.domain.clone()
            } else if let Some(parameter) = parameters.get(slot) {
                // This domain was already formed in declaration order. Live
                // annotation effects must not run again during input staging.
                parameter.ty.clone()
            } else {
                let data = self.instantiate_open_annotation_with_live(
                    owner,
                    slot,
                    &definition.parameters[slot].1,
                    sig,
                    symbols,
                    parameters,
                    live,
                    call,
                )?;
                let ty =
                    self.admit_supplied_annotation(data, route, call, &mut values[slot], true)?;
                early_domains.insert(slot, ty.clone());
                ty
            };
            route.require_collection_type(self, &ty, call.span)?;
            let expected = if route.is_collected() {
                let Term::Tuple(fields) = self.structural_type(&ty, call.span)? else {
                    unreachable!("collection type checked above")
                };
                if fields.len() != route.source_inputs().count() {
                    return Err(error(
                        call.span,
                        "variadic collection argument count mismatch",
                    ));
                }
                fields[field].clone()
            } else {
                ty.clone()
            };
            let prepared = if route.is_collected() {
                match values[slot].as_ref().map(|value| &value.kind) {
                    Some(K::Tuple(fields)) => fields.get(field).cloned(),
                    Some(_) => {
                        return Err(error(
                            call.span,
                            "prepared collection requires a tuple value",
                        ));
                    }
                    None => None,
                }
            } else {
                values[slot].take()
            };
            let value = if let Some(value) = prepared {
                self.require_expression(&value.ty, &expected, call.span)?;
                value
            } else if let Term::Lift(inner) = &expected {
                self.runtime_lift_input_expr(input, &expected, inner)?.0
            } else {
                self.with_staged_input(true, |this| this.expr(input, Some(&expected)))?
            };
            if route.is_collected() {
                let packet = if let K::Local(binding) = value.kind {
                    self.spread_input_value(binding, input)?
                } else {
                    None
                };
                let domain = value.ty.clone();
                let (value, completed) = if let Some(completed) = packet {
                    (value, completed)
                } else {
                    let (completed, _) = self.required_normal_input(input, value, vec![])?;
                    (
                        self.materialize_local_constant(completed.clone(), domain, input.span)?,
                        completed,
                    )
                };
                let symbol = self.optional_normal_term(completed.clone(), input.span)?;
                let packet = collections.entry(slot).or_insert_with(|| StagedCollection {
                    domain: ty,
                    fields: Vec::new(),
                    symbols: Vec::new(),
                    values: Vec::new(),
                });
                packet.fields.push(value);
                packet.symbols.push(symbol);
                packet.values.push(completed);
                if packet.fields.len() == route.source_inputs().count() {
                    let packet = collections.remove(&slot).unwrap();
                    values[slot] = Some(n::Expr {
                        source: source(call.span),
                        ty: packet.domain,
                        kind: K::Tuple(packet.fields),
                    });
                    symbols[slot] = packet
                        .symbols
                        .into_iter()
                        .collect::<Option<Vec<_>>>()
                        .map(Term::TupleValue);
                    live[slot] = Some(comptime_interpretor::Value::Tuple(packet.values));
                }
                continue;
            }
            if matches!(value.ty.callable_signature(), Term::Function { .. })
                && value.ty.is_comptime_callable()
            {
                let packet = if let K::Local(binding) = value.kind {
                    self.spread_input_value(binding, input)?
                } else {
                    None
                };
                let packet_expression = packet.as_ref().map(|_| value.clone());
                let ty = value.ty.clone();
                let completed = if let Some(completed) = packet {
                    completed
                } else {
                    self.required_normal_input(input, value, vec![])?.0
                };
                if matches!(&completed, comptime_interpretor::Value::Closure(_)) {
                    values[slot] = Some(if let Some(value) = packet_expression {
                        value
                    } else {
                        self.materialize_local_constant(completed.clone(), ty, input.span)?
                    });
                    symbols[slot] = None;
                    live[slot] = Some(completed);
                    continue;
                }
                let (value, symbol) =
                    self.freeze_completed_dependency(completed, ty, input.span)?;
                values[slot] = Some(value);
                symbols[slot] = Some(symbol);
                continue;
            }
            let (value, symbol) = if slot != target
                && matches!(value.kind, K::Function(_) | K::FunctionInstance { .. })
            {
                // Declaration references need no producer evaluation and can
                // be delivered without a normal-form function-value symbol.
                (value, None)
            } else if slot != target
                && matches!(value.ty.callable_signature(), Term::Function { .. })
            {
                if let K::Local(binding) = value.kind {
                    if self.prepare_spread_input(binding, input)?.is_some() {
                        // Keep the original local: the shared packet preparation
                        // owns materialization and preserves field identity.
                        (value, None)
                    } else {
                        let (completed, ty) = self.required_normal_input(input, value, vec![])?;
                        (
                            self.materialize_local_constant(completed, ty, input.span)?,
                            None,
                        )
                    }
                } else {
                    let (completed, ty) = self.required_normal_input(input, value, vec![])?;
                    (
                        self.materialize_local_constant(completed, ty, input.span)?,
                        None,
                    )
                }
            } else if slot != target
                && matches!(
                    self.structural_type(&value.ty, input.span)?,
                    Term::Tuple(_) | Term::Record(_) | Term::Nominal { .. }
                )
            {
                // Packet locals belong to the shared packet materialization.
                // Rebuilding an individual aggregate here would split captures
                // shared with sibling fields, and evaluating the local alone
                // would omit its packet preparation scope.
                let packet = if let K::Local(binding) = value.kind {
                    self.spread_input_value(binding, input)?
                } else {
                    None
                };
                let (value, symbol) = if let Some(completed) = packet {
                    let symbol = self.optional_normal_term(completed, input.span)?;
                    (value, symbol)
                } else {
                    let (completed, ty) = self.required_normal_input(input, value, vec![])?;
                    let symbol = self.optional_normal_term(completed.clone(), input.span)?;
                    let value = self.materialize_local_constant(completed, ty, input.span)?;
                    (value, symbol)
                };
                if let Some(symbol) = &symbol {
                    symbol
                        .check_scope(&[])
                        .map_err(|cause| error(call.span, cause.to_string()))?;
                }
                (value, symbol)
            } else {
                let (value, symbol) = self.freeze_open_call_input(input, value)?;
                symbol
                    .check_scope(&[])
                    .map_err(|cause| error(call.span, cause.to_string()))?;
                (value, Some(symbol))
            };
            values[slot] = Some(value);
            symbols[slot] = symbol;
        }
        // An empty collection has no producer to schedule, but remains an
        // assigned argument and can itself be a signature dependency.
        for route in routes
            .iter()
            .filter(|route| route.is_collected() && route.source_inputs().next().is_none())
        {
            let slot = route.parameter;
            if values[slot].is_some() && symbols[slot].is_some() {
                continue;
            }
            if parameters.get(slot).is_none() && definition.parameters[slot].1.needs_instance() {
                // An empty source prefix must not execute a later annotation
                // before declaration-ordered domain formation reaches it.
                continue;
            }
            let ty = if let Some(parameter) = parameters.get(slot) {
                parameter.ty.clone()
            } else {
                self.instantiate_open_annotation(
                    owner,
                    slot,
                    &definition.parameters[slot].1,
                    sig,
                    symbols,
                    parameters,
                    call,
                )?
                .identity(call.span)?
            };
            route.require_collection_type(self, &ty, call.span)?;
            if !matches!(self.structural_type(&ty, call.span)?, Term::Tuple(fields) if fields.is_empty())
            {
                return Err(error(
                    call.span,
                    "variadic collection argument count mismatch",
                ));
            }
            values[slot] = Some(n::Expr {
                source: source(call.span),
                ty,
                kind: K::Tuple(Vec::new()),
            });
            symbols[slot] = Some(Term::TupleValue(Vec::new()));
        }
        Ok(early_domains)
    }

    pub(super) fn prepare_open_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[Parameter],
        symbols: &[Option<Term>],
        writes: &mut DefaultWrites,
        force_staged: bool,
        span: Span,
    ) -> Result<(n::Expr, Option<Term>), Error> {
        let domains = parameters
            .iter()
            .map(|p| (p.sort, Some(p.ty.clone())))
            .collect::<Vec<_>>();
        self.prepare_open_default_in_scope(
            owner,
            slot,
            &domains,
            symbols,
            writes,
            force_staged,
            span,
        )
    }

    pub(super) fn prepare_open_default_in_scope(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[(Sort, Option<Term>)],
        symbols: &[Option<Term>],
        writes: &mut DefaultWrites,
        force_staged: bool,
        span: Span,
    ) -> Result<(n::Expr, Option<Term>), Error> {
        let expected = parameters
            .get(slot)
            .and_then(|(_, ty)| ty.as_ref())
            .ok_or_else(|| error(span, "default input domain has not formed"))?;
        self.in_open_function_scope(owner, None, |this| {
            let ItemKind::Fn(sig, _) = &this.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                return Err(error(
                    span,
                    "missing default for an unsupplied open parameter",
                ));
            };
            this.default_input_scope = Some(parameters.into());
            this.default_input_owner = Some(owner);
            let staged = force_staged || parameters[slot].0 != Sort::Runtime;
            let mut slots = BTreeMap::new();
            let mut changed = BTreeMap::new();
            for (index, parameter) in sig.params.iter().enumerate().take(slot) {
                let binding = local(parameter.hir_id);
                let (sort, domain) = &parameters[index];
                let Some(domain) = domain else {
                    continue;
                };
                this.locals.insert(binding, domain.clone());
                let previous_write = writes
                    .range((index, 0)..(index, slot))
                    .next_back()
                    .map(|(_, data)| data);
                if !staged && let Some((value, _)) = previous_write {
                    changed.insert(binding, value);
                }
                if let Some(value) = previous_write
                    .map(|(_, symbol)| symbol)
                    .or(symbols[index].as_ref())
                {
                    slots.insert(binding, (index, domain));
                    this.input_facts
                        .specialized
                        .push((binding, domain.clone(), value.clone()));
                    if *sort == Sort::Type {
                        this.input_facts.types.insert(binding, value.clone());
                    } else {
                        this.input_facts
                            .values
                            .insert(binding, (value.clone(), domain.clone()));
                    }
                }
            }
            this.in_staged_input = staged;
            let value = this.expr(default, Some(expected))?;
            if staged {
                let (value, symbol) =
                    this.evaluate_open_default(default, value, slot, slots, writes)?;
                Ok((value, Some(symbol)))
            } else {
                if !sig.modifiers.is_comptime {
                    this.require_stable_default(&value, default.span)?;
                }
                if changed.is_empty() {
                    return Ok((value, None));
                }
                let mut fuel = usize::try_from(this.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let result = (|| -> Result<_, String> {
                    let mut bindings = BTreeMap::new();
                    for id in nir::free_bindings::collect(&value, &mut fuel)? {
                        if let Some(&updated) = changed.get(&id) {
                            bindings.insert(id, updated.clone());
                        }
                    }
                    iir::staging::prepare_input_with_layouts(
                        iir::staging::Layouts::from_index(
                            &this.normal_form_metadata(),
                            &this.newtypes,
                        )
                        .with_target(this.target),
                        &value,
                        &bindings,
                        &mut fuel,
                    )
                })();
                this.required_steps -= (before - fuel) as u64;
                Ok((
                    result.map_err(|message| error(default.span, message))?,
                    None,
                ))
            }
        })
    }

    fn evaluate_open_default(
        &mut self,
        default: &Expr<'_>,
        value: n::Expr,
        position: usize,
        slots: BTreeMap<n::LocalId, (usize, &Term)>,
        writes: &mut DefaultWrites,
    ) -> Result<(n::Expr, Term), Error> {
        let span = default.span;
        if slots.is_empty() {
            return self.freeze_dependency_input(default, value, vec![]);
        }
        let (packet, outputs) = dependent_inputs::DefaultOutputs::prepare(
            self,
            value,
            |id| slots.get(&id).map(|&(_, ty)| ty),
            span,
        )?;
        let Some(outputs) = outputs else {
            return self.freeze_dependency_input(default, packet, vec![]);
        };
        let (completed, _) = self.required_normal_input(default, packet, vec![])?;
        let mut updated = Vec::new();
        let result = outputs.finish(self, completed, span, |id, value, symbol| {
            updated.push(((slots[&id].0, position), (value, symbol)));
        })?;
        writes.extend(updated);
        Ok(result)
    }
}
