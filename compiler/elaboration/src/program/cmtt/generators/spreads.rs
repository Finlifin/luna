//! Generator spread packets retain source-position evaluation and a completed
//! element cache. Width discovery never invokes the packet producer.
use super::*;

#[derive(Default)]
pub(in crate::program::cmtt) struct SpreadPlan<'h> {
    packets: Vec<Packet<'h>>,
    pub(in crate::program::cmtt) fields: BTreeMap<n::LocalId, (usize, usize)>,
    pub(in crate::program::cmtt) empty: Vec<(usize, usize)>,
}
struct Packet<'h> {
    source: Expr<'h>,
    width: usize,
    types: Option<Vec<Term>>,
    // Outer None: producer not evaluated. Empty slot: already delivered.
    // Data(None) is definition-checking type evidence; LiveData retains
    // invocation-local pointer identity without freezing the packet.
    values: Option<Vec<Option<(Argument, Term)>>>,
}
impl SpreadPlan<'_> {
    pub(in crate::program::cmtt) fn width(&self, packet: usize) -> usize {
        self.packets[packet].width
    }

    pub(in crate::program::cmtt) fn field_type(&self, id: n::LocalId) -> Option<&Term> {
        let (packet, element) = self.fields.get(&id)?;
        self.packets.get(*packet)?.types.as_ref()?.get(*element)
    }
    pub(in crate::program::cmtt) fn source_start(&self, packet: usize) -> u32 {
        self.packets[packet].source.span.lo().0
    }
}
impl fmt::Display for SpreadPlan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(generator-spreads (packets")?;
        for (index, packet) in self.packets.iter().enumerate() {
            write!(
                f,
                " (packet (index {index}) (source {}) (width {}) (prepared {}) (types",
                local(packet.source.hir_id),
                packet.width,
                packet.values.is_some()
            )?;
            match &packet.types {
                Some(types) => {
                    for ty in types {
                        write!(f, " {ty}")?;
                    }
                }
                None => f.write_str(" unknown")?,
            }
            f.write_str("))")?;
        }
        f.write_str(") (fields")?;
        for (id, (packet, element)) in &self.fields {
            write!(f, " (field {} (packet {packet}) (element {element}))", id)?;
        }
        f.write_str(") (empty")?;
        for (position, packet) in &self.empty {
            write!(f, " (at {position} (packet {packet}))")?;
        }
        f.write_str("))")
    }
}
impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn invoke_spread_generator(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
        type_plan: Option<GeneratorTypePlan>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        let owner = self
            .generator_owner(callee)
            .ok_or_else(|| error(site.span, "expected a code generator"))?;
        self.ensure_code_generator(owner, site.span)?;
        self.with_generator_spreads(owner, site.span, arguments, |this, inputs, plan| {
            this.invoke_code_generator_inputs(
                site,
                callee,
                inputs,
                expected,
                result_context,
                plan,
                type_plan,
            )
        })
    }

    pub(in crate::program::cmtt) fn with_generator_spreads<R>(
        &mut self,
        owner: OwnerId,
        span: Span,
        arguments: &[Arg<'_>],
        action: impl FnOnce(&mut Self, &[Arg<'_>], &mut SpreadPlan<'_>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let mut widths = Vec::new();
        for argument in arguments {
            let width = match argument {
                Arg::Expand(packet) => {
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let width = self.spread_width(packet, &mut fuel, 0);
                    self.required_steps = fuel as u64;
                    width?
                }
                Arg::Positional(_) => Some(1),
                _ => Some(0),
            };
            widths.push(width);
        }
        let unknown = widths
            .iter()
            .enumerate()
            .filter_map(|(index, width)| width.is_none().then_some(index))
            .collect::<Vec<_>>();
        if let [index] = unknown.as_slice() {
            let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                &self.package.item(owner).unwrap().kind
            else {
                unreachable!()
            };
            let unfilled_collection = signature.params.iter().any(|param| {
                matches!(param.ty.kind, TyParamKind::Varadic(..))
                    && !arguments
                        .iter()
                        .any(|arg| matches!(arg,Arg::Named(name,_) if name.name==param.name.name))
            });
            if !unfilled_collection {
                let post_target = arguments::post_lambda_target(
                    arguments,
                    Some(signature.params),
                    signature.params.len(),
                    span,
                )?;
                let anchored = signature
                    .params
                    .iter()
                    .enumerate()
                    .filter(|(index, param)| {
                        Some(*index) != post_target
                            && !param.ty.is_implicit()
                            && matches!(param.ty.kind, TyParamKind::Positional(_))
                    })
                    .count();
                let known = widths
                    .iter()
                    .flatten()
                    .try_fold(0usize, |sum, width| sum.checked_add(*width))
                    .ok_or_else(|| error(span, "generator spread width overflow"))?;
                widths[*index] = anchored.checked_sub(known);
            }
        }
        let mut plan = SpreadPlan::default();
        let mut flat = Vec::new();
        for (position, argument) in arguments.iter().enumerate() {
            match argument {
                Arg::Expand(packet) => {
                    if let H::Tuple(fields) = packet.kind {
                        self.required_steps = self
                            .required_steps
                            .checked_sub(fields.len() as u64)
                            .ok_or_else(|| {
                                error(packet.span, "generator argument expansion budget exhausted")
                            })?;
                        flat.extend(fields.iter().map(|field| (None, field.clone(), false)));
                        continue;
                    }
                    let width=widths[position].ok_or_else(||error(packet.span,"generator spread width needs a known tuple interface or a unique destination layout"))?;
                    self.required_steps = self
                        .required_steps
                        .checked_sub(width as u64 + 1)
                        .ok_or_else(|| {
                            error(packet.span, "generator argument expansion budget exhausted")
                        })?;
                    let types = self.generator_packet_type_hint(packet, 0)?;
                    let types = types.filter(|types| types.len() == width);
                    let index = plan.packets.len();
                    plan.packets.push(Packet {
                        source: (**packet).clone(),
                        width,
                        types,
                        values: None,
                    });
                    if width == 0 {
                        plan.empty.push((flat.len(), index));
                    }
                    for element in 0..width {
                        let binding = self.spread_binding(packet.span)?;
                        let id = hir::HirId::new(
                            hir::OwnerId::INVALID,
                            hir::ItemLocalId::new(binding.index),
                        );
                        plan.fields.insert(local(id), (index, element));
                        flat.push((
                            None,
                            Expr {
                                hir_id: id,
                                span: packet.span,
                                kind: H::ResolvedLocal(
                                    hir::Symbol::intern("$generator-spread"),
                                    id,
                                ),
                            },
                            false,
                        ));
                    }
                }
                Arg::Positional(value) => flat.push((None, (**value).clone(), false)),
                Arg::PostLambda(value) => flat.push((None, (**value).clone(), true)),
                Arg::Named(name, value) => flat.push((Some(*name), (**value).clone(), false)),
                _ => {
                    return Err(error(
                        span,
                        "unsupported generator argument supply protocol",
                    ));
                }
            }
        }
        let inputs = flat
            .iter()
            .map(|(name, value, post)| {
                if *post {
                    Arg::PostLambda(value)
                } else {
                    match name {
                        Some(name) => Arg::Named(*name, value),
                        None => Arg::Positional(value),
                    }
                }
            })
            .collect::<Vec<_>>();
        action(self, &inputs, &mut plan)
    }

    pub(in crate::program::cmtt) fn generator_packet_type_hint(
        &mut self,
        value: &Expr<'_>,
        depth: usize,
    ) -> Result<Option<Vec<Term>>, Error> {
        let Some(ty) = self.generator_expression_type_hint(value, depth)? else {
            return Ok(None);
        };
        Ok(match self.structural_type(&ty, value.span)? {
            Term::Tuple(fields) => Some(fields),
            _ => None,
        })
    }

    pub(in crate::program::cmtt) fn generator_packet_context(
        &mut self,
        plan: &SpreadPlan<'_>,
        packet: usize,
        routes: &[arguments::RoutedArgument<'_>],
        definition: &Generator,
        unresolved: &[DeclarationId],
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if plan.packets[packet].values.is_some() {
            return Ok(None);
        }
        let mut fields = vec![None; plan.packets[packet].width];
        for route in routes {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "generator packet context budget exhausted"))?;
            let Input::Data(ty) = &definition.inputs[route.parameter].1 else {
                continue;
            };
            if unresolved.iter().any(|id| ty.mentions_rigid(*id)) {
                continue;
            }
            let collection = if route.is_collected() {
                match self.structural_type(ty, span)? {
                    Term::Tuple(fields) => Some(fields),
                    _ => None,
                }
            } else {
                None
            };
            for (element, (_, input)) in route.source_inputs().enumerate() {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "generator packet context budget exhausted"))?;
                if let Some((owner, index)) = plan.fields.get(&local(input.hir_id)) {
                    if *owner == packet {
                        let ty = if route.is_collected() {
                            collection.as_ref().and_then(|fields| fields.get(element))
                        } else {
                            Some(ty)
                        };
                        if let Some(ty) = ty {
                            fields[*index] = Some(self.copy_alias_type(ty, span)?);
                        }
                    }
                }
            }
        }
        fields
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .map(|fields| self.tuple_value_type(fields, span))
            .transpose()
    }

    pub(in crate::program::cmtt) fn prepare_generator_packet(
        &mut self,
        plan: &mut SpreadPlan<'_>,
        index: usize,
        expected: Option<&Term>,
    ) -> Result<(), Error> {
        let packet = &mut plan.packets[index];
        if packet.values.is_some() {
            return Ok(());
        }
        let source = &packet.source;
        let (value, ty) = if self.checking_code_generator {
            let checked = self.expr(source, expected)?;
            self.validate_generator_data(&checked, source.span)?;
            (None, checked.ty)
        } else {
            let (value, ty) = self.generator_data_expression(source, expected)?;
            (Some(value), ty)
        };
        let Term::Tuple(types) = self.structural_type(&ty, source.span)? else {
            return Err(error(source.span, "generator spread must produce a tuple"));
        };
        if types.len() != packet.width {
            return Err(error(
                source.span,
                "generator spread width changed during preparation",
            ));
        }
        let values = if let Some(value) = value {
            // Validate the whole packet before splitting: a live pointer must
            // not exempt a callback sibling from capture validation.
            self.generator_value_needs_live_storage(&value, source.span)?;
            let comptime_interpretor::Value::Tuple(fields) = value else {
                return Err(error(
                    source.span,
                    "generator spread did not produce a tuple",
                ));
            };
            if fields.len() != types.len() {
                return Err(error(
                    source.span,
                    "generator spread value/type width mismatch",
                ));
            }
            let mut values = Vec::with_capacity(fields.len());
            for (value, ty) in fields.into_iter().zip(&types) {
                values.push(
                    if self.generator_value_needs_live_storage(&value, source.span)? {
                        Argument::LiveData(value)
                    } else {
                        Argument::Data(Some(self.materialize_generator_data(
                            value,
                            ty.clone(),
                            source.span,
                        )?))
                    },
                );
            }
            values
        } else {
            (0..types.len()).map(|_| Argument::Data(None)).collect()
        };
        packet.values = Some(values.into_iter().zip(types).map(Some).collect());
        Ok(())
    }

    pub(in crate::program::cmtt) fn generator_packet_field(
        &mut self,
        plan: &mut SpreadPlan<'_>,
        packet: usize,
        element: usize,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<(Argument, Term), Error> {
        self.prepare_generator_packet(plan, packet, None)?;
        let (value, ty) = plan.packets[packet].values.as_mut().unwrap()[element]
            .take()
            .ok_or_else(|| error(span, "generator spread element supplied more than once"))?;
        if let Some(expected) = expected {
            self.require(&ty, expected, span)?;
        }
        Ok((value, ty))
    }
}
