//! Retain the structure guaranteed by checked list constructors. Index the
//! completed collection so no element or spread producer runs a second time.
use super::construction::Construction;
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
        let element = self.list_element_type(domain).filter(|element| {
            !matches!(
                element,
                Term::Rigid { .. } | Term::Bound { .. } | Term::Lift(_)
            )
        });
        let fields = match self.observe_construction(argument, domain, element, span)? {
            Construction::NoReturn => return Ok(None),
            Construction::Field(field) => {
                let (path, constant) = field.into_parts();
                if let Some(term) = constant {
                    return Ok(Some(wrap_components(&path, term)));
                }
                if path.is_empty() {
                    return Ok(None);
                }
                let (term, output) =
                    self.projected_output(domain, root, argument.source(), path, span)?;
                projections.push(AnnotationStep {
                    preparation: None,
                    outputs: Arc::from([output]),
                });
                return Ok(Some(term));
            }
            Construction::List(fields) => fields,
        };
        let element = element.expect("list observation requires a registered element domain");
        let mut terms = Vec::with_capacity(fields.len());
        for (index, field) in fields.into_iter().enumerate() {
            let source = field.source.unwrap_or(argument.source());
            let (path, constant) = field.into_parts();
            if let Some(term) = constant {
                terms.push(wrap_components(&path, term));
                continue;
            }
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

fn wrap_components(path: &[n::TypeComponent], term: Term) -> Term {
    path.iter()
        .rev()
        .fold(term, |inner, component| component.wrap(inner))
}
