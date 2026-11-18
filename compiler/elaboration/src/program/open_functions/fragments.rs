//! Definition-side validation of a checked region in its compacted input frame.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn check_open_fragment(
        &mut self,
        value: &n::Expr,
        span: Span,
        arithmetic: bool,
    ) -> Result<(), Error> {
        if !self.open_loop_transfers.is_empty() {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let checked = (|| {
                for (target, found) in &mut self.open_loop_transfers {
                    if !found.breaks {
                        found.breaks = control::loop_may_complete(value, *target, &mut fuel)?;
                    }
                    if !found.continues {
                        found.continues = control::loop_may_continue(value, *target, &mut fuel)?;
                    }
                }
                Ok::<_, String>(())
            })();
            self.required_steps -= (before - fuel) as u64;
            checked.map_err(|message| error(span, message))?;
        }
        let owner = self.signature_owner.unwrap();
        let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            &self.package.item(owner).unwrap().kind
        else {
            unreachable!()
        };
        let mut staged_locals = Vec::new();
        if self.in_staged_input {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let free = nir::free_bindings::collect(value, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            for binding in free.map_err(|message| error(span, message))? {
                // All inputs of a comptime function are available during its
                // invocation, including ordinary parameters used by defaults.
                // They already belong to the fragment's parameter telescope.
                if sig.modifiers.is_comptime
                    && sig
                        .params
                        .iter()
                        .any(|parameter| local(parameter.hir_id) == binding)
                {
                    continue;
                }
                if self.required_runtime_parameter(binding) {
                    return Err(error(
                        span,
                        "required comptime computation cannot read a runtime parameter",
                    ));
                }
                if !self.input_facts.types.contains_key(&binding)
                    && !self.input_facts.values.contains_key(&binding)
                {
                    if self.open_staged_locals.contains(&binding) {
                        let ty = self
                            .locals
                            .get(&binding)
                            .ok_or_else(|| error(span, "staged local has no established domain"))?;
                        staged_locals.push((binding, ty.clone()));
                    } else {
                        return Err(error(
                            span,
                            format!(
                                "required comptime computation cannot read an unstaged local {binding}"
                            ),
                        ));
                    }
                }
            }
        }
        let definition = &self.open_functions[&owner];
        let mut parameters = Vec::new();
        let mut bindings = Vec::new();
        let mut mapping = vec![None; definition.parameters.len()];
        for (slot, (parameter, (sort, annotation))) in
            sig.params.iter().zip(&definition.parameters).enumerate()
        {
            // A staged fragment of a runtime function cannot read runtime
            // inputs (checked above), nor store them as synthetic comptime
            // arguments merely because they occur in the enclosing signature.
            if self.in_staged_input && !sig.modifiers.is_comptime && *sort == Sort::Runtime {
                continue;
            }
            let domain = match annotation {
                Annotation::Known(ty) => Some(ty.clone()),
                _ => self
                    .definition_refinements
                    .subjects
                    .get(&local(parameter.hir_id))
                    .map(|binder| Term::Rigid {
                        declaration: binder.declaration,
                        sort: binder.sort,
                    }),
            };
            if let Some(ty) = domain {
                mapping[slot] = Some((parameters.len(), *sort));
                parameters.push(Parameter { sort: *sort, ty });
                bindings.push(local(parameter.hir_id));
            }
        }
        // Locally computed staged values supply domains for definition checks.
        // Their actual values belong to the invocation that executes the source.
        for (binding, ty) in staged_locals {
            bindings.push(binding);
            parameters.push(Parameter {
                sort: Sort::ComptimeValue,
                ty,
            });
        }
        // Abstract inline selectors belong only to this definition-side
        // fragment. They are never runtime arguments of the source function.
        for (binding, ty) in &self.ordinary_behavior_inputs {
            if !bindings.contains(binding) {
                bindings.push(*binding);
                parameters.push(Parameter {
                    sort: Sort::ComptimeValue,
                    ty: ty.clone(),
                });
            }
        }
        // The fragment excludes unknown runtime domains. Every remaining type
        // reference must cross the same compaction map, including contracts and
        // operands nested in trait method values or callable signatures.
        for parameter in &mut parameters {
            parameter.ty = parameter
                .ty
                .relocate_slots(&mapping)
                .map_err(|failure| error(span, failure.to_string()))?;
        }
        constants::charge_copy(value, &mut self.required_steps, span)?;
        let mut value = value.clone();
        if !self.in_staged_input {
            if let Some(effects) = &self.effect_environment {
                value = n::Expr {
                    source: value.source,
                    ty: Term::computation(effects.clone(), value.ty.clone()),
                    kind: K::EffectComputation {
                        effects: effects.clone(),
                        body: Box::new(value),
                    },
                };
            }
        }
        let references =
            nir::copy_budget::expression_substitution_parameters(&value, &mut self.required_steps)
                .map_err(|failure| error(span, failure.to_string()))?;
        if references
            .into_iter()
            .any(|(slot, sort)| mapping.get(slot).copied().flatten() != Some((slot, sort)))
        {
            value
                .relocate_slots(&mapping)
                .map_err(|failure| error(span, failure.to_string()))?;
        }
        let requirements = self
            .definition_requirements(owner)
            .iter()
            .chain(&self.definition_refinements.requirements)
            .map(|requirement| {
                required::map_requirement(
                    requirement,
                    &mut |ty| {
                        ty.relocate_slots(&mapping)
                            .map_err(|failure| error(span, failure.to_string()))
                    },
                    span,
                    0,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let capabilities = self
            .capabilities
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|requirement| {
                Ok(n::TraitRequirement {
                    interface: requirement.interface,
                    subject: requirement
                        .subject
                        .relocate_slots(&mapping)
                        .map_err(|failure| error(span, failure.to_string()))?,
                    interface_arguments: requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| {
                            ty.relocate_slots(&mapping)
                                .map_err(|failure| error(span, failure.to_string()))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let imports = self.fragment_declarations(&value, span)?;
        let region = n::DefinitionRegion {
            owner: declaration(self.package, &self.package_identities, owner),
            phase: if self.in_staged_input || sig.modifiers.is_comptime {
                n::FunctionPhase::Comptime
            } else {
                n::FunctionPhase::Runtime
            },
            signature: Term::Function {
                parameters,
                result: Box::new(value.ty.clone()),
            },
            parameters: bindings,
            type_parameters: self
                .definition_refinements
                .subjects
                .values()
                .map(|binder| binder.declaration)
                .chain(self.definition_refinements.tuple_elements.iter().copied())
                .collect(),
            requirements,
            capabilities,
            body: value,
        };
        let package = n::Package {
            target: self.target,
            normal_forms: self.normal_form_metadata(),
            function_parameters: self.function_parameter_metadata(),
            structs: self.structs.values().cloned().collect(),
            newtypes: self.newtypes.values().cloned().collect(),
            alias_contracts: self.alias_contracts.values().cloned().collect(),
            enums: self.enums.values().cloned().collect(),
            traits: self.checked_traits.to_vec(),
            ..Default::default()
        };
        if arithmetic {
            crate::verification::flow::validate_definition_region_with_declarations(
                &package, &imports, &region,
            )
            .map_err(|mut failure| {
                failure.source = failure
                    .source
                    .original(&self.expansion_sources)
                    .unwrap_or(failure.source);
                error(span, failure.to_string())
            })
        } else {
            nir::phases::inspect_definition_region_with_declarations(&package, &imports, &region)
                .map(|_| ())
                .map_err(|failure| error(span, failure.message))
        }
    }

    fn fragment_declarations(
        &mut self,
        root: &n::Expr,
        span: Span,
    ) -> Result<nir::callable_source::RegionDeclarations, Error> {
        let mut imports = nir::callable_source::RegionDeclarations::new();
        // The phase pass inspects these method contracts as well as the fragment.
        // Their references are roots even if the fragment never calls that method.
        let contracts = self
            .checked_traits
            .iter()
            .flat_map(|interface| &interface.methods)
            .flat_map(|method| {
                method
                    .program
                    .preconditions
                    .iter()
                    .chain(&method.program.postconditions)
            });
        let mut pending = Vec::new();
        for expression in std::iter::once(root).chain(contracts) {
            pending.extend(
                required::references(expression, &mut self.required_steps)
                    .map_err(|failure| error(span, failure.to_string()))?
                    .into_iter()
                    .filter_map(required::Dependency::callable),
            );
        }
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let declaration = if let Some(declaration) = self.generated_functions.declaration(id) {
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let copy = nir::copy_budget::clone_function_declaration(declaration, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                copy.map_err(|_| error(span, "region declaration snapshot budget exhausted"))?
            } else {
                let Some(owner) = self.source_declarations.owner(id) else {
                    continue;
                };
                self.ensure_signature(owner, span)?;
                let Some(ItemKind::Fn(sig, _)) = self.package.item(owner).map(|item| &item.kind)
                else {
                    continue;
                };
                if sig.modifiers.is_normal_form_head {
                    continue;
                }
                self.ensure_contract(owner, span)?;
                let Some(signature) = self.declared_signature(owner).map(|sig| sig.into_owned())
                else {
                    continue;
                };
                self.in_function_scope(owner, None, |this| {
                    this.with_source_declaration(
                        owner,
                        signature,
                        function_bodies::Defaults::CallPlan,
                        span,
                        |_, declaration| Ok(declaration),
                    )
                })?
            };
            for contract in declaration
                .preconditions
                .iter()
                .chain(&declaration.postconditions)
                .chain(declaration.defaults.iter().map(|(_, expr)| expr))
            {
                pending.extend(
                    required::references(contract, &mut self.required_steps)
                        .map_err(|failure| error(span, failure.to_string()))?
                        .into_iter()
                        .filter_map(required::Dependency::callable),
                );
            }
            imports.insert(id, declaration);
        }
        Ok(imports)
    }
}
