//! Source-ordered staged suppliers and declaration-scoped defaults for open calls.
use super::*;

/// Open signature formation can receive an executed value in addition to a
/// producer and its static identity. Ordinary source checking owns no VM values.
pub(super) struct StagedInputs {
    pub(super) values: Vec<Option<n::Expr>>,
    pub(super) symbols: Vec<Option<Term>>,
    pub(super) live: Vec<Option<comptime_interpretor::Value>>,
}

impl StagedInputs {
    pub(super) fn new(count: usize) -> Self {
        Self {
            values: vec![None; count],
            symbols: vec![None; count],
            live: vec![None; count],
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
    pub(super) fn prepare_open_comptime_prefix(
        &mut self,
        call: &Expr<'_>,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        definition: &Definition,
        routes: &[arguments::RoutedArgument<'_>],
        target: usize,
        parameters: &[Parameter],
        values: &mut [Option<n::Expr>],
        symbols: &mut [Option<Term>],
        live: &mut [Option<comptime_interpretor::Value>],
    ) -> Result<BTreeMap<usize, Term>, Error> {
        let mut early_domains = BTreeMap::new();
        let end = Self::open_comptime_prefix_end(routes, target);
        let mut inputs = Vec::new();
        for (route_index, route) in routes.iter().enumerate() {
            if values[route.parameter].is_some()
                && (symbols[route.parameter].is_some() || live[route.parameter].is_some())
            {
                continue;
            }
            for (field, (position, _)) in route.source_inputs().enumerate() {
                if position <= end {
                    inputs.push((position, route_index, field));
                }
            }
        }
        inputs.sort_unstable_by_key(|(position, _, _)| *position);
        let mut collections = BTreeMap::<usize, StagedCollection>::new();
        for (_, route_index, field) in inputs {
            let route = &routes[route_index];
            let slot = route.parameter;
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
                    call.span,
                )?;
                let ty =
                    self.admit_supplied_annotation(data, route, call, &mut values[slot], true)?;
                early_domains.insert(slot, ty.clone());
                ty
            };
            route.require_collection_type(self, &ty, call.span)?;
            let aggregate = route.expression(call);
            let input = if route.is_collected() {
                route.source_inputs().nth(field).unwrap().1
            } else {
                &aggregate
            };
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
                    call.span,
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

    pub(super) fn prepare_open_live_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[Parameter],
        symbols: &[Option<Term>],
        live: &[Option<comptime_interpretor::Value>],
        span: Span,
        unresolved: &[DeclarationId],
    ) -> Result<Option<(comptime_interpretor::Value, Term)>, Error> {
        let assumptions = self.context.assumptions().to_vec();
        self.in_open_function_scope(owner, None, |this| {
            this.import_closed_caller_assumptions(assumptions);
            let ItemKind::Fn(sig, _) = &this.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                return Err(error(
                    span,
                    "missing default for an unsupplied open parameter",
                ));
            };
            this.default_input_scope = Some(
                parameters
                    .iter()
                    .map(|p| (p.sort, Some(p.ty.clone())))
                    .collect(),
            );
            this.default_input_owner = Some(owner);
            this.in_staged_input = true;
            let mut graph = Vec::new();
            for (index, parameter) in sig.params.iter().enumerate().take(slot) {
                let binding = local(parameter.hir_id);
                let domain = &parameters[index];
                this.locals.insert(binding, domain.ty.clone());
                if let Some(value) = &live[index] {
                    graph.push((binding, domain.ty.clone(), value.clone()));
                } else if let Some(value) = &symbols[index] {
                    this.input_facts
                        .specialized
                        .push((binding, domain.ty.clone(), value.clone()));
                    if domain.ty == p(P::Type) {
                        this.input_facts.types.insert(binding, value.clone());
                    } else {
                        this.input_facts
                            .values
                            .insert(binding, (value.clone(), domain.ty.clone()));
                    }
                }
            }
            let prepared = this.prepare_required_expression_with_live_inputs(
                default,
                Some(&parameters[slot].ty),
                None,
                Vec::new(),
                false,
                &graph,
            )?;
            if !unresolved.is_empty()
                && this.open_computation_uses_clauses(&prepared, unresolved, default.span)?
            {
                return Ok(None);
            }
            let selected: BTreeSet<_> = prepared
                .inputs
                .iter()
                .map(|(binding, _, _)| *binding)
                .collect();
            this.invoke_required_publishing(
                prepared,
                graph
                    .into_iter()
                    .filter(|(binding, _, _)| selected.contains(binding))
                    .map(|(binding, _, value)| (binding, value))
                    .collect(),
                default.hir_id.owner,
                default.span,
            )
            .map(Some)
        })
    }

    pub(super) fn prepare_open_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[Parameter],
        symbols: &[Option<Term>],
        force_staged: bool,
        span: Span,
    ) -> Result<(n::Expr, Option<Term>), Error> {
        let domains = parameters
            .iter()
            .map(|p| (p.sort, Some(p.ty.clone())))
            .collect::<Vec<_>>();
        self.prepare_open_default_in_scope(owner, slot, &domains, symbols, force_staged, span)
    }

    pub(super) fn prepare_open_default_in_scope(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[(Sort, Option<Term>)],
        symbols: &[Option<Term>],
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
            for (index, parameter) in sig.params.iter().enumerate().take(slot) {
                let binding = local(parameter.hir_id);
                let (sort, domain) = &parameters[index];
                let Some(domain) = domain else {
                    continue;
                };
                this.locals.insert(binding, domain.clone());
                if let Some(value) = &symbols[index] {
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
            let staged = force_staged || parameters[slot].0 != Sort::Runtime;
            this.in_staged_input = staged;
            let value = this.expr(default, Some(expected))?;
            if staged {
                let (value, symbol) = this.freeze_dependency_input(default, value, vec![])?;
                Ok((value, Some(symbol)))
            } else {
                if !sig.modifiers.is_comptime {
                    this.require_stable_default(&value, default.span)?;
                }
                Ok((value, None))
            }
        })
    }
}
