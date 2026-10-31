//! Complete a missing Type input only after explicit source inputs. Defaults
//! before it execute in parameter order and are retained for the body frame.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn complete_generator_type_defaults<
        O: nir::code_interfaces::SignatureOutput + Clone,
    >(
        &mut self,
        owner: OwnerId,
        mut definition: Generator<O>,
        last: usize,
        values: &mut [Option<Argument>],
        actual: &mut [Option<Term>],
        completed: &mut BTreeMap<DeclarationId, Term>,
        span: Span,
    ) -> Result<BTreeSet<usize>, Error> {
        // The prefix installs owned interface copies, while values themselves
        // move into and out of the frame. Charge those copies before any default.
        Self::charge_code_generator(&definition, &mut self.required_steps, span)?;
        self.generator_frame(owner, |this| {
            this.install_generator_capture_values(&definition.captures, span)?;
            this.alias_substitution.extend(
                definition
                    .types
                    .iter()
                    .map(|(id, value)| (*id, Arc::new(value.clone()))),
            );
            this.install_generator_clause_values(
                owner,
                &definition.types,
                &definition.clause_domains,
            )?;
            let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                &this.package.item(owner).unwrap().kind
            else {
                unreachable!()
            };
            let mut defaults = BTreeSet::new();
            for slot in 0..=last {
                let binder = definition.input_binders.get(&slot).copied();
                let (binding, input) = definition.inputs[slot].clone();
                let argument = if let Some(argument) = values[slot].take() {
                    argument
                } else {
                    let TyParamKind::Optional(_, _, expression) = signature.params[slot].ty.kind
                    else {
                        return Err(error(
                            signature.params[slot].span,
                            "missing contextual input",
                        ));
                    };
                    defaults.insert(slot);
                    if binder.is_some() {
                        let ty = this.generator_required_type(expression)?;
                        Argument::Data(Some(n::Expr {
                            source: source(expression.span),
                            ty: p(P::Type),
                            kind: K::TypeValue(ty),
                        }))
                    } else {
                        this.generator_default(expression, &input)?
                    }
                };
                if let Some(binder) = binder {
                    let Argument::Data(Some(n::Expr {
                        kind: K::TypeValue(ty),
                        ..
                    })) = &argument
                    else {
                        return Err(error(
                            span,
                            "contextual Type prefix needs a completed value",
                        ));
                    };
                    completed.insert(binder.declaration, this.copy_alias_type(ty, span)?);
                    this.specialize_generator_input(&mut definition, slot, ty, span)?;
                }
                this.immutable.insert(binding);
                this.install_generator_type_input(binding, definition.input_value(slot));
                match (input, argument) {
                    (Input::Data(ty), Argument::Data(value)) => {
                        actual[slot].get_or_insert_with(|| ty.clone());
                        this.locals.insert(binding, ty);
                        this.generator_data.insert(binding, value);
                    }
                    (Input::Data(ty), Argument::LiveData(value)) => {
                        actual[slot].get_or_insert_with(|| ty.clone());
                        this.install_live_generator_input(binding, &ty, value, span);
                    }
                    (Input::Code(interface), Argument::Code(CodeValue::Expression(code))) => {
                        let code = this.ascribe_code(code, Some(&interface), span)?;
                        this.code_values.insert(binding, code);
                    }
                    (Input::Pattern(interface), Argument::Code(CodeValue::Pattern(code))) => {
                        this.ascribe_pattern_input(&code, &interface, span)?;
                        this.pattern_codes.insert(binding, code);
                    }
                    _ => return Err(error(span, "contextual default input category mismatch")),
                }
            }
            // Retain post-default storage, not the original argument snapshot:
            // a default can update values reached through earlier inputs.
            for slot in 0..=last {
                let (binding, input) = &definition.inputs[slot];
                values[slot] = Some(match input {
                    Input::Data(_) => {
                        let value = this
                            .generator_data
                            .remove(binding)
                            .ok_or_else(|| error(span, "missing prepared prefix data"))?;
                        if value.as_ref().is_some_and(
                            |value| matches!(value.kind, K::Local(id) if id == *binding),
                        ) {
                            let cell = this
                                .generator_cells
                                .get(binding)
                                .ok_or_else(|| error(span, "missing prepared prefix storage"))?
                                .1
                                .clone();
                            Argument::LiveData(this.read_generator_cell(&cell, span)?)
                        } else {
                            Argument::Data(value)
                        }
                    }
                    Input::Code(_) => Argument::Code(CodeValue::Expression(
                        this.code_values
                            .remove(binding)
                            .ok_or_else(|| error(span, "missing prepared prefix code"))?,
                    )),
                    Input::Pattern(_) => Argument::Code(CodeValue::Pattern(
                        this.pattern_codes
                            .remove(binding)
                            .ok_or_else(|| error(span, "missing prepared prefix pattern"))?,
                    )),
                });
            }
            Ok(defaults)
        })
    }
}
