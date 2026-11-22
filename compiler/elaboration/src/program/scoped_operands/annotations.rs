//! Local type annotations demand a concrete identity when execution reaches
//! the declaration. The checked initializer still reads its original storage.
use super::*;
mod collections;
use crate::program::normal::observation::{self, Failure};

// Alias roots identify an immutable value, never a mutable slot. Keep these
// separate from completed Type evidence, specialization inputs and caches.
#[derive(Clone, Default)]
pub(in crate::program) struct NamedTypes {
    roots: LocalBindings<n::LocalId>,
    witnesses: LocalBindings<nir::DeclarationParameter>,
}

impl NamedTypes {
    pub(in crate::program) fn root(&self, local: n::LocalId) -> n::LocalId {
        self.roots.get(&local).copied().unwrap_or(local)
    }
    pub(in crate::program) fn alias(&mut self, binding: n::LocalId, source: n::LocalId) {
        self.roots.insert(binding, self.root(source));
    }
    pub(in crate::program) fn get(&self, local: n::LocalId) -> Option<nir::DeclarationParameter> {
        self.witnesses.get(&self.root(local)).copied()
    }
    pub(super) fn bind(&mut self, local: n::LocalId, binder: nir::DeclarationParameter) {
        self.witnesses.insert(self.root(local), binder);
    }
    pub(in crate::program) fn is_empty(&self) -> bool {
        self.roots.is_empty() && self.witnesses.is_empty()
    }
}

impl std::fmt::Display for NamedTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(named-types")?;
        for (binding, root) in &self.roots {
            write!(f, " (alias {binding} {root})")?;
        }
        for (binding, binder) in &self.witnesses {
            write!(f, " (local {binding} {binder})")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone)]
pub(in crate::program) struct Annotation {
    site: hir::HirId,
    forming: bool,
    pub(super) inputs: Vec<AnnotationStep>,
}

#[derive(Clone)]
pub(super) struct AnnotationStep {
    pub(super) preparation: Option<Arc<n::Expr>>,
    pub(super) outputs: Arc<[Output]>,
}

#[derive(Clone)]
pub(super) struct Output {
    pub(super) local: n::LocalId,
    pub(super) input: Input,
    pub(super) domain: Term,
    pub(super) components: Vec<n::TypeComponent>,
}

impl Output {
    fn new(local: n::LocalId, input: Input, domain: Term) -> Self {
        Self {
            local,
            input,
            domain,
            components: vec![],
        }
    }
}

impl Annotation {
    pub(in crate::program) fn new(site: hir::HirId) -> Self {
        Self {
            site,
            forming: false,
            inputs: Vec::new(),
        }
    }
}

impl Checker<'_, '_> {
    /// Only a local actually read by this structural observation can demand a
    /// witness. Unselected branches may carry gradual Type data.
    pub(in crate::program) fn observe_annotation_head(
        &mut self,
        checked: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if !self.collecting_named_types() && self.input_facts.named_types.witnesses.is_empty() {
            return self.checked_normal_input_in(checked, &BTreeMap::new(), span);
        }
        // If a later operand needs execution, the VM will run the entire
        // plan. Demands from an abandoned partial observation must not run
        // before that plan (a later argument may return from the caller).
        let checkpoint = self
            .input_facts
            .annotation
            .as_ref()
            .filter(|a| a.forming)
            .map(|a| (self.input_facts.named_types.clone(), a.inputs.len()));
        let result = self.observe_head_with_named_inputs(checked, span);
        if !matches!(result, Ok(Some(_)))
            && let Some((named, count)) = checkpoint
        {
            self.input_facts.named_types = named;
            self.input_facts
                .annotation
                .as_mut()
                .unwrap()
                .inputs
                .truncate(count);
        }
        result
    }

    fn observe_head_with_named_inputs(
        &mut self,
        checked: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let mut bindings = BTreeMap::new();
        loop {
            self.input_facts.observing_annotation_head = true;
            let result = self.with_observation_budget(span, |checker, budget| {
                match checker.observe_normal_input_in(checked, 0, budget, &bindings) {
                    Err(Failure::LocalType(local, source)) => Ok(Err((local, source))),
                    other => observation::optional(other).map(Ok),
                }
            });
            self.input_facts.observing_annotation_head = false;
            let (local, source) = match result? {
                Ok(identity) => return Ok(identity),
                Err(read) => read,
            };
            let Some(identity) = self.named_type_identity(local, source, span)? else {
                return Ok(None);
            };
            if bindings
                .insert(local, std::sync::Arc::new(identity))
                .is_some()
            {
                return Ok(None);
            }
        }
    }

    pub(in crate::program) fn local_binding_annotation(
        &mut self,
        site: hir::HirId,
        ty: &Expr<'_>,
    ) -> Result<Term, Error> {
        let collecting = self
            .input_facts
            .annotation
            .as_ref()
            .is_some_and(|a| a.site == site);
        if !collecting {
            return self.type_expr(ty);
        }
        self.input_facts.annotation.as_mut().unwrap().forming = true;
        let result = self.type_expr(ty);
        self.input_facts.annotation.as_mut().unwrap().forming = false;
        result
    }

    pub(in crate::program) fn collecting_named_types(&self) -> bool {
        self.input_facts
            .annotation
            .as_ref()
            .is_some_and(|a| a.forming)
    }

    /// Keep a computed annotation in the invocation that owns its lexical reads
    /// and control transfers. Structural evidence can narrow the initializer's
    /// expected domain, but the original producer must still execute.
    pub(in crate::program) fn computed_annotation_type(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<CompletedType, Error> {
        let annotation = self.input_facts.annotation.take().unwrap();
        let checked = self.expr(expr, Some(&p(P::Type)));
        self.input_facts.annotation = Some(annotation);
        let checked = checked?;
        match checked.kind {
            K::TypeValue(_) | K::Literal(n::Literal::Description(_)) => {
                return self.complete_checked_type(expr, checked);
            }
            _ => {}
        }
        if matches!(expr.kind, H::NFApplication(..)) {
            return self.complete_annotation_head(expr, checked);
        }
        let known = match self.preserved_input_identity(&checked, |_, _| Ok(None), expr.span)? {
            Some(known) => Some(known),
            None => self.projected_type_return(&checked, expr.span)?,
        };
        self.retain_annotation_type(checked, known, expr.span)
    }

    /// A self-contained head can still form a concrete domain for checking its
    /// initializer. Open plans and caller control transfers need this frame.
    pub(in crate::program) fn complete_annotation_head(
        &mut self,
        expr: &Expr<'_>,
        checked: n::Expr,
    ) -> Result<CompletedType, Error> {
        let span = expr.span;
        let needs_frame = self.with_observation_budget(span, |_, budget| {
            if !nir::free_bindings::collect(&checked, &mut budget.steps)?.is_empty() {
                return Ok(true);
            }
            let mut transfers = false;
            nir::visit::expression_in_closure_environment(
                &checked,
                &mut budget.steps,
                |node, _| {
                    transfers |= matches!(node.kind, K::Return(_) | K::Break(_) | K::Continue(_));
                    Ok(())
                },
            )
            .map_err(Failure::Invalid)?;
            Ok(transfers)
        })?;
        if needs_frame || !self.checked_declaration_inputs(&checked, span)?.is_empty() {
            self.retain_formed_head(checked, span)
        } else {
            self.complete_checked_type(expr, checked)
        }
    }

    /// A successful application of a known head exposes its actual arguments.
    /// Match the completed value once, after every operand and formation check,
    /// and use those field identities to check the initializer.
    fn retain_formed_head(&mut self, checked: n::Expr, span: Span) -> Result<CompletedType, Error> {
        let (head, arguments) = match &checked.kind {
            K::NormalFormType {
                head, arguments, ..
            } => (*head, arguments),
            K::ApplyNormalForm {
                callee, arguments, ..
            } if let K::Function(head) = callee.kind => (head, arguments),
            _ => return self.retain_annotation_type(checked, None, span),
        };
        let count = arguments.len();
        dependent_inputs::tasks::spend(&mut self.required_steps, count, span)?;
        let Some(parameters) = self
            .source_declarations
            .owner(head)
            .and_then(|owner| self.normal_parameters.get(&owner))
            .filter(|parameters| count != 0 && parameters.len() == count)
        else {
            return self.retain_annotation_type(checked, None, span);
        };
        // A lifted input carries runtime environment evidence. Keep the
        // formed identity intact at this purely comptime boundary.
        if parameters
            .iter()
            .any(|(_, _, domain)| Sort::of_comptime_domain(domain) == Sort::Runtime)
        {
            return self.retain_annotation_type(checked, None, span);
        }
        let signature = parameters
            .iter()
            .map(|(id, _, domain)| {
                declaration_inputs::copy_input_domain(domain, &mut self.required_steps, span)
                    .map(|domain| (*id, domain))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let owner = self.input_facts.annotation.as_ref().unwrap().site.owner;
        let mut fields = Vec::with_capacity(count);
        for (_, domain) in &signature {
            let binder = if *domain == p(P::Type) {
                self.fresh_type_binder(owner, span)?
            } else {
                self.fresh_value_binder(owner, span)?
            };
            fields.push((
                binder,
                Term::Rigid {
                    declaration: binder.declaration,
                    sort: binder.sort,
                },
            ));
        }
        // Rebind the declaration's entire telescope at this application. Keep
        // its domain dependencies even when some result arguments are known.
        let replacements: BTreeMap<_, _> = signature
            .iter()
            .zip(&fields)
            .map(|((id, domain), (_, rigid))| (*id, (domain, rigid)))
            .collect();
        let constants = arguments
            .iter()
            .map(|arg| self.annotation_constant(arg.expression(), span))
            .collect::<Result<Vec<_>, _>>()?;
        // The lexical telescope retains every dependency. For a constructed
        // value we can additionally use closed evidence for the Type formals,
        // even when a dependent collector was written first.
        let type_arguments: BTreeMap<_, _> = signature
            .iter()
            .zip(&fields)
            .zip(&constants)
            .filter_map(|(((_, ty), (binder, _)), known)| {
                (*ty == p(P::Type)).then_some((binder.declaration, (ty, known.as_ref()?)))
            })
            .collect();
        let mut terms = Vec::with_capacity(count);
        let mut patterns = Vec::with_capacity(count);
        let mut projections = Vec::new();
        let mut outputs = Vec::with_capacity(count);
        for (slot, ((argument, (_, domain)), (binder, rigid))) in
            arguments.iter().zip(&signature).zip(&fields).enumerate()
        {
            let domain = self.with_observation_budget(span, |_, budget| {
                Ok(nir::instantiate_observed_domain(
                    domain,
                    |id| replacements.get(&id).copied(),
                    &[],
                    &mut budget.steps,
                )?)
            })?;
            let local = self.spread_binding(span)?;
            let known = if let Some(value) = &constants[slot] {
                Some(declaration_inputs::copy_input_domain(
                    value,
                    &mut self.required_steps,
                    span,
                )?)
            } else {
                let exposed = self.with_observation_budget(span, |_, budget| {
                    Ok(nir::instantiate_observed_domain(
                        &domain,
                        |id| type_arguments.get(&id).copied(),
                        &[],
                        &mut budget.steps,
                    )?)
                })?;
                self.constructed_annotation(argument, &exposed, local, &mut projections, span)?
            };
            terms.push(known.unwrap_or_else(|| rigid.clone()));
            patterns.push(n::Pattern::Binding {
                binding: local,
                inner: Box::new(n::Pattern::Wild),
            });
            outputs.push(Output::new(
                local,
                Input {
                    binder: *binder,
                    producer: checked.source,
                    stable_local: None,
                },
                domain,
            ));
        }
        let preparation = n::Expr {
            source: checked.source,
            ty: p(P::Void),
            kind: K::LetPattern {
                pattern: n::Pattern::NormalForm {
                    head,
                    arguments: patterns,
                },
                value: Box::new(checked),
                guard: None,
                otherwise: None,
            },
        };
        self.input_facts
            .annotation
            .as_mut()
            .unwrap()
            .inputs
            .push(AnnotationStep {
                preparation: Some(Arc::new(preparation)),
                outputs: outputs.into(),
            });
        self.input_facts
            .annotation
            .as_mut()
            .unwrap()
            .inputs
            .extend(projections);
        Ok(CompletedType::Identity(Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: terms,
        }))
    }

    fn retain_annotation_type(
        &mut self,
        checked: n::Expr,
        known: Option<Term>,
        span: Span,
    ) -> Result<CompletedType, Error> {
        let owner = self.input_facts.annotation.as_ref().unwrap().site.owner;
        let binder = self.fresh_type_binder(owner, span)?;
        let input = Input {
            binder,
            producer: checked.source,
            stable_local: None,
        };
        let (local, preparation) = match checked.kind {
            K::Local(local) => (local, None),
            _ => {
                let local = self.spread_binding(span)?;
                (
                    local,
                    Some(Arc::new(n::Expr {
                        source: checked.source,
                        ty: p(P::Void),
                        kind: K::Let {
                            binding: local,
                            value: Box::new(checked),
                        },
                    })),
                )
            }
        };
        self.input_facts
            .annotation
            .as_mut()
            .unwrap()
            .inputs
            .push(AnnotationStep {
                preparation,
                outputs: Arc::from([Output::new(local, input, p(P::Type))]),
            });
        Ok(CompletedType::Identity(known.unwrap_or(Term::Rigid {
            declaration: binder.declaration,
            sort: binder.sort,
        })))
    }

    pub(in crate::program) fn named_type_identity(
        &mut self,
        binding: n::LocalId,
        producer: n::SourceRange,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if !self.uses_lexical_operands()
            || !self.immutable.contains(&binding)
            || self.locals.get(&binding) != Some(&p(P::Type))
        {
            return Ok(None);
        }
        let binder = if let Some(binder) = self.input_facts.named_types.get(binding) {
            binder
        } else if self
            .input_facts
            .annotation
            .as_ref()
            .is_some_and(|a| a.forming && a.site.owner.def_id.raw() == binding.owner)
            && !self.input_facts.has_identity(&binding)
        {
            let owner = self.input_facts.annotation.as_ref().unwrap().site.owner;
            let binder = self.fresh_type_binder(owner, span)?;
            self.input_facts.named_types.bind(binding, binder);
            self.input_facts
                .annotation
                .as_mut()
                .unwrap()
                .inputs
                .push(AnnotationStep {
                    preparation: None,
                    outputs: Arc::from([Output::new(
                        binding,
                        Input {
                            binder,
                            producer,
                            stable_local: None,
                        },
                        p(P::Type),
                    )]),
                });
            binder
        } else {
            return Ok(None);
        };
        // The lexical reference remains a capture even if an earlier statement
        // already established the same identity for its own annotation.
        if !self.captures.is_empty() {
            self.record_runtime_expr_captures(
                &n::Expr {
                    source: producer,
                    ty: p(P::Type),
                    kind: K::Local(binding),
                },
                span,
            )?;
        }
        Ok(Some(Term::Rigid {
            declaration: binder.declaration,
            sort: binder.sort,
        }))
    }
}
