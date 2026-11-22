//! Close each computed initializer type over the portion of its lexical block
//! that can read the initialized locals.
use super::*;

#[derive(Default)]
pub(in crate::program) struct BindingScopes(BTreeMap<n::LocalId, Arc<Inputs>>);

impl BindingScopes {
    pub(in crate::program) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(in crate::program) fn retain(
        &mut self,
        checker: &mut Checker<'_, '_>,
        boundary: Boundary,
        statement: &mut n::Expr,
        prefix: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<(), Error> {
        let Boundary::Consumer { inputs } = boundary else {
            return Ok(());
        };
        let (local, value) = match &mut statement.kind {
            K::Let { binding, value } => (Some(*binding), value),
            K::LetPattern { value, .. } => (None, value),
            _ => return Err(needs_frame(span, "expected a checked local binding")),
        };
        let used = checker.term_declaration_inputs(&value.ty, span)?;
        if !inputs
            .values()
            .any(|input| used.contains(&input.binder.declaration))
        {
            let source = value.source;
            let call = std::mem::replace(
                &mut **value,
                n::Expr {
                    source,
                    ty: p(P::Void),
                    kind: K::Unit,
                },
            );
            **value = checker.close_operand_consumer(call, &inputs, Consumer::Call, span)?;
            return Ok(());
        }
        let binding = match local {
            Some(binding) => binding,
            None => {
                // Pattern stabilization can split a declaration into several
                // statements. Keep its original subject at a unique local.
                let binding = checker.spread_binding(span)?;
                let read = n::Expr {
                    source: value.source,
                    ty: declaration_inputs::copy_input_domain(
                        &value.ty,
                        &mut checker.required_steps,
                        span,
                    )?,
                    kind: K::Local(binding),
                };
                prefix.push(n::Expr {
                    source: value.source,
                    ty: p(P::Void),
                    kind: K::Let {
                        binding,
                        value: std::mem::replace(value, Box::new(read)),
                    },
                });
                binding
            }
        };
        self.0.insert(binding, inputs);
        Ok(())
    }

    pub(in crate::program) fn finish(
        mut self,
        checker: &mut Checker<'_, '_>,
        mut block: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let K::Block { statements, result } = &mut block.kind else {
            unreachable!()
        };
        let source = block.source;
        let mut tail = result.take();
        let mut suffix = Vec::new();
        let mut diverges = false;
        let mut depth = 0;
        for statement in std::mem::take(statements).into_iter().rev() {
            dependent_inputs::tasks::spend(&mut checker.required_steps, 1, span)?;
            let inputs = match statement.kind {
                K::Let { binding, .. } => self.0.remove(&binding),
                _ => None,
            };
            diverges |= is_never(&statement.ty);
            suffix.push(statement);
            if let Some(inputs) = inputs {
                suffix.reverse();
                let ty = match &tail {
                    _ if diverges => p(P::Never),
                    Some(value) => declaration_inputs::copy_input_domain(
                        &value.ty,
                        &mut checker.required_steps,
                        span,
                    )?,
                    None => p(P::Void),
                };
                let body = n::Expr {
                    source,
                    ty,
                    kind: K::Block {
                        statements: std::mem::take(&mut suffix),
                        result: tail,
                    },
                };
                let body =
                    checker.close_operand_consumer(body, &inputs, Consumer::Binding, span)?;
                depth += 1; // The block containing this initializer.
                let mut current = &body;
                while let K::WithTypeInputs { body, .. } = &current.kind {
                    depth += 1;
                    current = body;
                }
                if depth >= nir::copy_budget::MAX_COPY_DEPTH {
                    return Err(error(span, "lexical operand nesting budget exhausted"));
                }
                tail = Some(Box::new(body));
                diverges = false;
            }
        }
        if !self.0.is_empty() {
            return Err(needs_frame(
                span,
                "local type initializer was lost during block completion",
            ));
        }
        suffix.reverse();
        *statements = suffix;
        *result = tail;
        Ok(block)
    }
}

impl std::fmt::Display for BindingScopes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(local-operand-scopes")?;
        for (binding, inputs) in &self.0 {
            write!(f, " (local {binding}")?;
            for input in inputs.values() {
                write!(f, " {}", input.binder)?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}
