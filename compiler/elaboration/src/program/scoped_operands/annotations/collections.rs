//! Retain the structure guaranteed by checked list constructors. Index the
//! completed collection so no element or spread producer runs a second time.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn annotation_constant(
        &mut self,
        expr: Option<&n::Expr>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Some(expr) = expr else { return Ok(None) };
        if !matches!(
            expr.kind,
            K::TypeValue(_) | K::ComptimeValue(_) | K::Literal(_)
        ) || !self.checked_declaration_inputs(expr, span)?.is_empty()
        {
            return Ok(None);
        }
        Ok(self
            .with_observation_budget(span, |checker, budget| {
                checker.normal_input_hint(expr, budget)
            })?
            .filter(|ty| ty.check_scope(&[]).is_ok()))
    }

    pub(super) fn constructed_annotation(
        &mut self,
        argument: &n::NormalFormArgument,
        domain: &Term,
        root: n::LocalId,
        projections: &mut Vec<AnnotationStep>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let (path, leaf) = self.constructor_path(argument.expression(), domain, span)?;
        if let Some(term) = self.annotation_constant(leaf, span)?
            && term.sort() == Sort::of_comptime_domain(domain)
        {
            return Ok(Some(wrap_components(&path, term)));
        }
        if !path.is_empty() {
            let (term, output) =
                self.projected_output(domain, root, argument.source(), path, span)?;
            projections.push(AnnotationStep {
                preparation: None,
                outputs: Arc::from([output]),
            });
            return Ok(Some(term));
        }
        let Some(element) = self.list_element_type(domain) else {
            return Ok(None);
        };
        // A free element domain can still become Type when an outer scope
        // specializes. Preserve the collected value across that sort change.
        if matches!(
            element,
            Term::Rigid { .. } | Term::Bound { .. } | Term::Lift(_)
        ) {
            return Ok(None);
        }
        let Some(fields) = construction_fields(argument, &mut self.required_steps, span)? else {
            return Ok(None);
        };
        let mut terms = Vec::with_capacity(fields.len());
        for (index, field) in fields.into_iter().enumerate() {
            let (path, leaf) = self.constructor_path(field, element, span)?;
            if let Some(term) = self.annotation_constant(leaf, span)?
                && term.sort() == Sort::of_comptime_domain(element)
            {
                terms.push(wrap_components(&path, term));
                continue;
            }
            let source = field.map_or(argument.source(), |f| f.source);
            let local = self.spread_binding(span)?;
            let (term, output) = self.projected_output(element, local, source, path, span)?;
            let read = n::Expr {
                source,
                ty: declaration_inputs::copy_input_domain(element, &mut self.required_steps, span)?,
                kind: K::TypeListIndex(
                    Box::new(n::Expr {
                        source,
                        ty: declaration_inputs::copy_input_domain(
                            domain,
                            &mut self.required_steps,
                            span,
                        )?,
                        kind: K::Local(root),
                    }),
                    Box::new(n::Expr {
                        source,
                        ty: p(P::Integer),
                        kind: K::Literal(n::Literal::Integer(index.to_string())),
                    }),
                ),
            };
            projections.push(AnnotationStep {
                preparation: Some(Arc::new(n::Expr {
                    source,
                    ty: p(P::Void),
                    kind: K::Let {
                        binding: local,
                        value: Box::new(read),
                    },
                })),
                outputs: Arc::from([output]),
            });
            terms.push(term);
        }
        self.freeze_normal_list(domain, terms, span).map(Some)
    }

    fn constructor_path<'e>(
        &mut self,
        mut leaf: Option<&'e n::Expr>,
        domain: &Term,
        span: Span,
    ) -> Result<(Vec<n::TypeComponent>, Option<&'e n::Expr>), Error> {
        let mut path = Vec::new();
        while let Some(value) = leaf {
            dependent_inputs::tasks::spend(&mut self.required_steps, 1, span)?;
            if let Some(tail) = completion_value(value) {
                leaf = Some(tail);
                continue;
            }
            if *domain != p(P::Type) {
                break;
            }
            let (step, inner) = match &value.kind {
                K::OptionalType(inner) => (n::TypeComponent::OptionalPayload, inner),
                K::PointerType(inner) => (n::TypeComponent::Pointee, inner),
                _ => break,
            };
            path.push(step);
            leaf = Some(inner);
        }
        Ok((path, leaf))
    }

    fn projected_output(
        &mut self,
        domain: &Term,
        local: n::LocalId,
        source: n::SourceRange,
        components: Vec<n::TypeComponent>,
        span: Span,
    ) -> Result<(Term, Output), Error> {
        let owner = self.input_facts.annotation.as_ref().unwrap().site.owner;
        let binder = if *domain == p(P::Type) {
            self.fresh_type_binder(owner, span)?
        } else {
            self.fresh_value_binder(owner, span)?
        };
        let term = wrap_components(
            &components,
            Term::Rigid {
                declaration: binder.declaration,
                sort: binder.sort,
            },
        );
        let output = Output {
            local,
            input: Input {
                binder,
                producer: source,
                stable_local: None,
            },
            domain: declaration_inputs::copy_input_domain(domain, &mut self.required_steps, span)?,
            components,
        };
        Ok((term, output))
    }
}

// A successful block supplies its tail. The original plan still owns all
// preceding statements and every control transfer.
fn completion_value(expr: &n::Expr) -> Option<&n::Expr> {
    match &expr.kind {
        K::Block {
            result: Some(tail), ..
        }
        | K::CallPreparation {
            result: Some(tail), ..
        } => Some(tail),
        _ => None,
    }
}

fn wrap_components(path: &[n::TypeComponent], term: Term) -> Term {
    path.iter()
        .rev()
        .fold(term, |inner, component| component.wrap(inner))
}

/// Concatenation and literal spreads preserve their constituent elements.
/// A spread of unknown width prevents projecting any fixed layout here.
fn construction_fields<'a>(
    argument: &'a n::NormalFormArgument,
    steps: &mut u64,
    span: Span,
) -> Result<Option<Vec<Option<&'a n::Expr>>>, Error> {
    use n::NormalFormArgument as A;
    enum Part<'a> {
        List(&'a n::Expr),
        Item(&'a A),
    }
    let mut work = Vec::new();
    match argument {
        A::Value(expr) => work.push(Part::List(expr)),
        A::List { items, .. } => {
            dependent_inputs::tasks::spend(steps, items.len(), span)?;
            work.extend(items.iter().rev().map(Part::Item));
        }
        _ => return Ok(None),
    }
    let mut fields = Vec::new();
    while let Some(part) = work.pop() {
        dependent_inputs::tasks::spend(steps, 1, span)?;
        match part {
            Part::List(expr) => match &expr.kind {
                K::TypeList(items) => {
                    dependent_inputs::tasks::spend(steps, items.len(), span)?;
                    fields.extend(items.iter().map(Some));
                }
                K::ListConcat(left, right) => {
                    work.push(Part::List(right));
                    work.push(Part::List(left));
                }
                _ => {
                    let Some(tail) = completion_value(expr) else {
                        return Ok(None);
                    };
                    work.push(Part::List(tail));
                }
            },
            Part::Item(A::ListSpread(list)) => work.push(Part::List(list)),
            Part::Item(item) => fields.push(item.expression()),
        }
    }
    Ok(Some(fields))
}
