//! Shared lexical input plans for declaration formation and caller supply.
use super::*;

/// The implicit prefix of a local declaration's clause telescope. This is
/// checker-local metadata over immutable HIR, not a persisted dependency summary.
/// Selection still conservatively retains the enclosing telescope; precise
/// dependency and proof closure must replace selection at this single producer.
#[derive(Clone, Default)]
pub(super) struct LexicalInputPlan {
    inputs: Vec<LexicalInput>,
}

#[derive(Clone)]
struct LexicalInput {
    declaration: DeclarationId,
    sort: Sort,
    supply: LexicalSupply,
}

#[derive(Clone)]
enum LexicalSupply {
    Declaration,
    Staged(hir::HirId),
}

impl LexicalInputPlan {
    pub(super) fn declarations(&self) -> impl Iterator<Item = DeclarationId> + '_ {
        self.inputs.iter().map(|input| input.declaration)
    }
}

impl std::fmt::Display for LexicalInputPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(lexical-input-plan")?;
        for input in &self.inputs {
            write!(
                f,
                " (input {} (sort {}) (supply ",
                input.declaration, input.sort
            )?;
            match input.supply {
                LexicalSupply::Declaration => f.write_str("declaration")?,
                LexicalSupply::Staged(binding) => write!(f, "(staged {})", local(binding))?,
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_input_dump_distinguishes_supply_frames() {
        let plan = LexicalInputPlan {
            inputs: vec![
                LexicalInput {
                    declaration: DeclarationId {
                        package: 3,
                        local: 4,
                    },
                    sort: Sort::Type,
                    supply: LexicalSupply::Declaration,
                },
                LexicalInput {
                    declaration: DeclarationId {
                        package: 3,
                        local: 5,
                    },
                    sort: Sort::ComptimeValue,
                    supply: LexicalSupply::Staged(hir::HirId {
                        owner: OwnerId::new(hir::LocalDefId::new(8)),
                        local_id: hir::ItemLocalId::new(2),
                    }),
                },
            ],
        };
        assert_eq!(
            plan.to_string(),
            "(lexical-input-plan \
             (input (declaration (package 3) (local 4)) (sort type) (supply declaration)) \
             (input (declaration (package 3) (local 5)) (sort comptime-value) \
             (supply (staged (local 8 2)))))"
        );
        assert_eq!(
            LexicalInputPlan::default().to_string(),
            "(lexical-input-plan)"
        );
    }
}

impl Checker<'_, '_> {
    /// Form the prefix once from a completed parent signature. Both signature
    /// construction and all ordinary local-function call paths consume it.
    /// A missing parent signature is an error, never an empty capture list.
    pub(super) fn lexical_input_plan(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<LexicalInputPlan, Error> {
        if let Some(plan) = self.lexical_input_plans.get(&owner) {
            return Ok(plan.clone());
        }
        let Some(parent) = self.package.lexical_parents.get(&owner).copied() else {
            return Ok(LexicalInputPlan::default());
        };
        self.ensure_signature(parent, span)?;
        let parameters = self.type_parameters[&parent]
            .iter()
            .copied()
            .chain(self.nominal_capture_ids(parent))
            .collect::<Vec<_>>();
        let mut inputs = self
            .input_binders(&parameters)
            .into_iter()
            .map(|binder| LexicalInput {
                declaration: binder.declaration,
                sort: binder.sort,
                supply: LexicalSupply::Declaration,
            })
            .collect::<Vec<_>>();
        let staged = self.lexical_staged_inputs(parent, span)?;
        for (binding, declaration, sort, _) in &staged {
            if !inputs.iter().any(|input| input.declaration == *declaration) {
                inputs.push(LexicalInput {
                    declaration: *declaration,
                    sort: *sort,
                    supply: LexicalSupply::Staged(*binding),
                });
            }
        }
        // Publish domain metadata only after every staged domain was formed.
        // The existing declaration-domain index remains its single authority.
        for (_, declaration, _, domain) in staged {
            self.lexical_input_domains.insert(declaration, domain);
        }
        let plan = LexicalInputPlan { inputs };
        self.lexical_input_plans.insert(owner, plan.clone());
        Ok(plan)
    }

    /// Lexical inputs are fixed by the enclosing instance; source arguments
    /// instantiate only the local function's explicit clause telescope.
    pub(super) fn lexical_function_arguments(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(..))
        ) {
            return Ok(Vec::new());
        }
        let plan = self.lexical_input_plan(owner, span)?;
        let mut arguments = Vec::with_capacity(plan.inputs.len());
        for input in plan.inputs {
            let value = match input.supply {
                LexicalSupply::Declaration => {
                    if let Some(value) = self.alias_substitution.get(&input.declaration).cloned() {
                        self.copy_alias_type(&value, span)?
                    } else {
                        Term::Rigid {
                            declaration: input.declaration,
                            sort: input.sort,
                        }
                    }
                }
                LexicalSupply::Staged(binding) => {
                    // Descendants use their fixed lexical input, not the
                    // parent's Bound index: a recursive callee's own parameter
                    // at that index can have a completely different sort.
                    if let Some((value, _)) = self.captured_staged_input(binding, span)? {
                        value
                    } else {
                        let value = if input.sort == Sort::Type {
                            self.type_input_bindings.get(&local(binding)).cloned()
                        } else {
                            self.value_input_bindings
                                .get(&local(binding))
                                .map(|(value, _)| value.clone())
                        };
                        value.ok_or_else(|| {
                            error(span, "lexical staged input is unavailable at this use")
                        })?
                    }
                }
            };
            arguments.push(value);
        }
        Ok(arguments)
    }

    /// Distinguish an unimplemented invocation environment from an unresolved
    /// name. Only diagnose after ordinary locals/closure captures were tried:
    /// lambdas already carry invocation values through their own environments.
    pub(super) fn unavailable_lexical_input(&self, binding: hir::HirId) -> &'static str {
        let fallback = "local binding is unavailable in this scope";
        let Some(mut owner) = self.signature_owner else {
            return fallback;
        };
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.package.lexical_parents.get(&owner).copied() {
            if !seen.insert(parent) {
                return fallback;
            }
            if parent == binding.owner {
                if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                    self.package.item(parent).map(|item| &item.kind)
                {
                    if signature.modifiers.is_comptime
                        && signature.params.iter().any(|parameter| {
                            parameter.hir_id == binding && !parameter.ty.is_comptime()
                        })
                    {
                        return "capturing an ordinary comptime function parameter in a local declaration requires an invocation environment, which is not yet implemented";
                    }
                }
                return fallback;
            }
            owner = parent;
        }
        fallback
    }

    pub(super) fn lift_lexical_type(
        &self,
        parent: OwnerId,
        ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(parent).map(|item| &item.kind)
        else {
            return Ok(ty.clone());
        };
        let mut frame = vec![None; sig.params.len()];
        for (binding, id, sort, _) in self.lexical_staged_inputs(parent, span)? {
            let index = sig
                .params
                .iter()
                .position(|parameter| parameter.hir_id == binding)
                .unwrap();
            frame[index] = Some(Term::Rigid {
                declaration: id,
                sort,
            });
        }
        let lifted = ty
            .substitute_partial(&frame)
            .map_err(|e| error(span, e.to_string()))?;
        if !lifted
            .substitution_parameters()
            .map_err(|e| error(span, e.to_string()))?
            .is_empty()
        {
            return Err(error(
                span,
                "lexical type requirement depends on an unavailable runtime input",
            ));
        }
        Ok(lifted)
    }

    pub(super) fn lexical_staged_inputs(
        &self,
        parent: OwnerId,
        span: Span,
    ) -> Result<Vec<(hir::HirId, DeclarationId, Sort, Term)>, Error> {
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(parent).map(|item| &item.kind)
        else {
            return Ok(Vec::new());
        };
        let domains = if let Some(signature) = self
            .signatures
            .get(&parent)
            .or_else(|| self.interface_signatures.get(&parent))
        {
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Ok(Vec::new());
            };
            parameters
                .iter()
                .map(|p| Some((p.sort, p.ty.clone())))
                .collect::<Vec<_>>()
        } else if let Some(definition) = self.open_functions.get(&parent) {
            definition
                .input_domains()
                .into_iter()
                .map(|(sort, domain)| domain.map(|domain| (sort, domain)))
                .collect()
        } else {
            return Err(error(span, "lexical parent signature is unavailable"));
        };
        let mut result = Vec::new();
        let mut frame = vec![None; sig.params.len()];
        for (index, parameter) in sig.params.iter().enumerate() {
            if !parameter.ty.is_comptime() {
                continue;
            }
            let (sort, domain) = domains
                .get(index)
                .and_then(Option::as_ref)
                .ok_or_else(|| error(span, "lexical staged input requires a formed domain"))?;
            let id = self
                .package
                .staged_parameter_definitions
                .get(&parameter.hir_id)
                .ok_or_else(|| error(span, "staged parameter has no source identity"))?;
            let id = DeclarationId {
                package: semantic_package(&self.package_identities, id.pkg),
                local: id.index,
            };
            let domain = domain
                .substitute_partial(&frame)
                .map_err(|e| error(span, e.to_string()))?;
            if !domain
                .substitution_parameters()
                .map_err(|e| error(span, e.to_string()))?
                .is_empty()
            {
                return Err(error(
                    span,
                    "lexical staged input domain depends on an unavailable runtime input",
                ));
            }
            frame[index] = Some(Term::Rigid {
                declaration: id,
                sort: *sort,
            });
            result.push((parameter.hir_id, id, *sort, domain));
        }
        Ok(result)
    }

    pub(super) fn captured_staged_input(
        &mut self,
        binding: hir::HirId,
        span: Span,
    ) -> Result<Option<(Term, Term)>, Error> {
        let Some(mut owner) = self.signature_owner else {
            return Ok(None);
        };
        if owner == binding.owner {
            return Ok(None);
        }
        let Some(id) = self.package.staged_parameter_definitions.get(&binding) else {
            return Ok(None);
        };
        let id = DeclarationId {
            package: semantic_package(&self.package_identities, id.pkg),
            local: id.index,
        };
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.package.lexical_parents.get(&owner).copied() {
            if !seen.insert(parent) {
                return Err(error(span, "cyclic lexical definition environment"));
            }
            if parent == binding.owner {
                let domain = self
                    .lexical_input_domains
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| error(span, "lexical staged input domain is unavailable"))?;
                let domain = self.substituted_trait_domain(&domain, span)?;
                let symbol = if let Some(value) = self.alias_substitution.get(&id).cloned() {
                    self.copy_alias_type(&value, span)?
                } else {
                    Term::Rigid {
                        declaration: id,
                        sort: if domain == p(P::Type) {
                            Sort::Type
                        } else {
                            Sort::ComptimeValue
                        },
                    }
                };
                return Ok(Some((symbol, domain)));
            }
            owner = parent;
        }
        Ok(None)
    }
}
