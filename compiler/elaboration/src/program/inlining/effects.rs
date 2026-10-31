//! Eliminate a closed direct handler after its callees have been expanded.
//! Unsupported routing/control transfers keep the complete installation intact.
use super::*;

pub(super) fn expand(package: &mut n::Package, fuel: &mut usize) -> Result<(usize, usize), String> {
    let inline: BTreeSet<_> = package
        .effects
        .iter()
        .filter(|effect| effect.inline && effect.abi == nir::effects::Abi::Direct)
        .map(|effect| effect.declaration)
        .collect();
    if inline.is_empty() {
        return Ok((0, 0));
    }
    let mut pass = Installations {
        inline: &inline,
        next: &mut package.quote_identity_cursor,
        sources: &mut package.expansion_sources,
        enums: &mut package.enums,
        packets: &mut package.internal_return_packets,
        empty: &mut package.internal_return_empty_replacements,
        fuel,
        installations: 0,
        requests: 0,
    };
    for function in &mut package.functions {
        if function.phase != n::FunctionPhase::Runtime {
            continue;
        }
        // Rewriter clones types even when no installation can be rewritten.
        // First scan only for a matching installation; keep this scan bounded.
        let mut eligible = false;
        if nir::visit::expression(&function.body, pass.fuel, |node, _| {
            if let K::EffectHandle { handlers, .. } = &node.kind {
                eligible |= !handlers.is_empty()
                    && handlers.iter().all(|handler| {
                        handler.abi == nir::effects::Abi::Direct
                            && inline.contains(&handler.operation.declaration)
                    });
            }
            Ok(())
        })
        .is_err()
        {
            break;
        }
        if !eligible {
            continue;
        }
        if charge(&function.body, pass.fuel).is_err() {
            break;
        }
        function.body.rewrite_with(&mut pass)?;
    }
    Ok((pass.installations, pass.requests))
}

/// Retry closed inline installations after staged quote inputs and Type
/// substitutions have completed. No ordinary callee expansion is attempted;
/// unsupported installations retain the same transactional fallback as NIR.
pub(crate) fn expand_materialized(
    package: &n::Package,
    inline: &BTreeSet<DeclarationId>,
    expression: &mut n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    enums: &mut Vec<n::Enum>,
    fuel: &mut usize,
) -> Result<(usize, usize), String> {
    if inline.is_empty() {
        return Ok((0, 0));
    }
    // Avoid copying package-wide rollback metadata for unrelated functions.
    // This traversal also pays for the type clones performed by Rewriter.
    let mut eligible = false;
    let checked = nir::visit::expression(expression, fuel, |node, fuel| {
        if let K::EffectHandle { handlers, .. } = &node.kind {
            eligible |= !handlers.is_empty()
                && handlers.iter().all(|handler| {
                    handler.abi == nir::effects::Abi::Direct
                        && inline.contains(&handler.operation.declaration)
                });
        }
        nir::type_computations::charge_term(&node.ty, fuel).map_err(|e| e.to_string())
    });
    if checked.is_err() || !eligible {
        return Ok((0, 0));
    }
    let Some(remaining) = fuel.checked_sub(
        package.internal_return_packets.len() + package.internal_return_empty_replacements.len(),
    ) else {
        return Ok((0, 0));
    };
    *fuel = remaining;
    let mut packets = package.internal_return_packets.clone();
    let mut empty = package.internal_return_empty_replacements.clone();
    let mut pass = Installations {
        inline,
        next,
        sources,
        enums,
        packets: &mut packets,
        empty: &mut empty,
        fuel,
        installations: 0,
        requests: 0,
    };
    expression.rewrite_with(&mut pass)?;
    Ok((pass.installations, pass.requests))
}

fn charge(expression: &n::Expr, fuel: &mut usize) -> Result<(), String> {
    nir::visit::expression(expression, fuel, |expr, fuel| {
        nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
    })
}

struct Installations<'a> {
    inline: &'a BTreeSet<DeclarationId>,
    next: &'a mut u32,
    sources: &'a mut Vec<n::ExpansionSource>,
    enums: &'a mut Vec<n::Enum>,
    packets: &'a mut BTreeSet<DeclarationId>,
    empty: &'a mut BTreeMap<(u32, u32), DeclarationId>,
    fuel: &'a mut usize,
    installations: usize,
    requests: usize,
}
impl n::Rewriter for Installations<'_> {
    type Error = String;
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        Ok(ty.clone())
    }
    fn expression(&mut self, expression: &mut n::Expr) -> Result<bool, String> {
        if *self.fuel == 0 {
            return Ok(false);
        }
        *self.fuel -= 1;
        let K::EffectHandle { handlers, .. } = &expression.kind else {
            return Ok(true);
        };
        if handlers.is_empty()
            || handlers.iter().any(|handler| {
                handler.abi != nir::effects::Abi::Direct
                    || !self.inline.contains(&handler.operation.declaration)
            })
        {
            return Ok(true);
        }
        if charge(expression, self.fuel).is_err() {
            return Ok(false);
        }
        // Closed nested installations have their own routing environment.
        // Commit those independent rewrites before attempting the enclosing
        // installation; its rollback must not undo already valid inner work.
        if let K::EffectHandle {
            computation,
            handlers,
            ..
        } = &mut expression.kind
        {
            computation.rewrite_with(self)?;
            for handler in handlers {
                for arm in &mut handler.arms {
                    arm.pattern.rewrite_with(self)?;
                    if let Some(guard) = &mut arm.guard {
                        guard.rewrite_with(self)?;
                    }
                    arm.body.rewrite_with(self)?;
                }
            }
        }
        if charge(expression, self.fuel).is_err() {
            return Ok(false);
        }
        let mut candidate = expression.clone();
        let end = self.sources.len();
        let next = *self.next;
        let enum_end = self.enums.len();
        let mut packets = BTreeSet::new();
        let mut empty = BTreeMap::new();
        let attempt = lower(
            &mut candidate,
            self.next,
            self.sources,
            self.enums,
            &mut packets,
            &mut empty,
            self.empty,
            self.fuel,
        );
        match attempt {
            Ok(requests) => {
                self.packets.extend(packets);
                self.empty.extend(empty);
                *expression = candidate;
                self.installations += 1;
                self.requests += requests;
            }
            Err(_) => {
                self.sources.truncate(end);
                self.enums.truncate(enum_end);
                *self.next = next;
            }
        }
        Ok(false)
    }
}

fn lower(
    expression: &mut n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    enums: &mut Vec<n::Enum>,
    packets: &mut BTreeSet<DeclarationId>,
    empty: &mut BTreeMap<(u32, u32), DeclarationId>,
    known_empty: &BTreeMap<(u32, u32), DeclarationId>,
    fuel: &mut usize,
) -> Result<usize, String> {
    let K::EffectHandle {
        computation,
        handlers,
        outer,
    } = &mut expression.kind
    else {
        unreachable!()
    };
    // Handler bodies execute in the ambient row, with this installation
    // masked. Normalize their runs separately before copying them at requests.
    let mut templates: Vec<_> = handlers
        .iter_mut()
        .map(|handler| n::Expr {
            ty: expression.ty.clone(),
            source: expression.source,
            kind: K::Match {
                value: Box::new(n::Expr {
                    ty: p(P::Void),
                    source: expression.source,
                    kind: K::Unit,
                }),
                arms: std::mem::take(&mut handler.arms),
            },
        })
        .collect();
    {
        let mut ambient = Requests {
            handlers: &[],
            outer,
            row: (0..outer.len()).collect(),
            templates: &[],
            next,
            sources,
            enums,
            packets,
            empty,
            known_empty,
            fuel,
            requests: 0,
            allow_resume: true,
        };
        for template in &mut templates {
            template.rewrite_with(&mut ambient)?;
        }
    }
    *fuel = fuel
        .checked_sub(handlers.len().saturating_add(outer.len()))
        .ok_or("inline effect row budget exhausted")?;
    let row = (0..handlers.len() + outer.len()).collect();
    let mut pass = Requests {
        handlers,
        outer,
        row,
        templates: &templates,
        next,
        sources,
        enums,
        packets,
        empty,
        known_empty,
        fuel,
        requests: 0,
        allow_resume: false,
    };
    computation.rewrite_with(&mut pass)?;
    let requests = pass.requests;
    let body = std::mem::replace(
        computation,
        Box::new(n::Expr {
            ty: p(P::Void),
            source: expression.source,
            kind: K::Unit,
        }),
    );
    *expression = *body;
    Ok(requests)
}

struct Requests<'a> {
    allow_resume: bool,
    handlers: &'a [n::EffectHandler],
    outer: &'a [nir::effects::Operation],
    /// Local row slots mapped to the installation's handlers then ambient row.
    row: Vec<usize>,
    templates: &'a [n::Expr],
    known_empty: &'a BTreeMap<(u32, u32), DeclarationId>,
    next: &'a mut u32,
    sources: &'a mut Vec<n::ExpansionSource>,
    enums: &'a mut Vec<n::Enum>,
    packets: &'a mut BTreeSet<DeclarationId>,
    empty: &'a mut BTreeMap<(u32, u32), DeclarationId>,
    fuel: &'a mut usize,
    requests: usize,
}
impl n::Rewriter for Requests<'_> {
    type Error = String;
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        match ty {
            Term::Computation { effects, result } if self.matches_row(effects) => {
                Ok(*result.clone())
            }
            Term::Computation { .. } => Err("residual computation row".into()),
            _ => Ok(ty.clone()),
        }
    }
    fn expression(&mut self, expression: &mut n::Expr) -> Result<bool, String> {
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or("inline effect budget exhausted")?;
        match &mut expression.kind {
            K::EffectComputation { effects, body } => {
                if !self.matches_row(effects) {
                    return Err("different effect environment".into());
                }
                body.rewrite_with(self)?;
                *expression = *std::mem::replace(
                    body,
                    Box::new(n::Expr {
                        ty: p(P::Void),
                        source: expression.source,
                        kind: K::Unit,
                    }),
                );
                Ok(false)
            }
            K::EffectRun { computation, slots } => {
                let Term::Computation { effects, .. } = &computation.ty else {
                    return Err("inline run requires computation type".into());
                };
                if slots.len() != effects.len() {
                    return Err("inline run row arity mismatch".into());
                }
                *self.fuel = self
                    .fuel
                    .checked_sub(slots.len().saturating_add(1))
                    .ok_or("inline run mapping budget exhausted")?;
                let mut mapped = Vec::with_capacity(slots.len());
                for (slot, operation) in slots.iter().zip(effects) {
                    let installed = *self
                        .row
                        .get(*slot)
                        .ok_or("inline run slot outside current row")?;
                    if self.operation(installed) != Some(operation) {
                        return Err("inline run operation identity mismatch".into());
                    }
                    mapped.push(installed);
                }
                let previous = std::mem::replace(&mut self.row, mapped);
                let rewritten = computation.rewrite_with(self);
                self.row = previous;
                rewritten?;
                *expression = *std::mem::replace(
                    computation,
                    Box::new(n::Expr {
                        ty: p(P::Void),
                        source: expression.source,
                        kind: K::Unit,
                    }),
                );
                Ok(false)
            }
            K::EffectRequest {
                operation,
                abi,
                allowed_index,
                arguments,
                ..
            } => {
                let installed = *self
                    .row
                    .get(*allowed_index)
                    .ok_or("invalid inline effect row slot")?;
                if installed >= self.handlers.len() {
                    if self.operation(installed) != Some(operation)
                        || *abi != nir::effects::Abi::Direct
                    {
                        return Err("invalid ambient inline effect slot".into());
                    }
                    for argument in arguments.iter_mut() {
                        argument.rewrite_with(self)?;
                    }
                    *allowed_index = installed - self.handlers.len();
                    return Ok(false);
                }
                let handler = &self.handlers[installed];
                let fallback = handler.fallback;
                let fallback_signature = if fallback.is_some() {
                    nir::type_computations::charge_term(&handler.signature, self.fuel)
                        .map_err(|error| error.to_string())?;
                    Some(handler.signature.clone())
                } else {
                    None
                };
                if handler.operation != *operation || *abi != nir::effects::Abi::Direct {
                    return Err("different operation identity".into());
                }
                for argument in arguments.iter_mut() {
                    argument.rewrite_with(self)?;
                }
                let value = n::Expr {
                    ty: Term::Tuple(arguments.iter().map(|a| a.ty.clone()).collect()),
                    source: expression.source,
                    kind: K::Tuple(std::mem::take(arguments)),
                };
                let template = &self.templates[installed];
                charge(template, self.fuel)?;
                let mut replacement = template.clone();
                replacement.ty = expression.ty.clone();
                if let Some(slot) = fallback {
                    if self.outer.get(slot) != Some(operation) {
                        return Err("inline fallback operation identity mismatch".into());
                    }
                    // Ordinary failed candidates roll back before this final
                    // arm takes the original payload. Never evaluate request
                    // arguments again or recursively select this installation.
                    let Term::Tuple(types) = &value.ty else {
                        unreachable!()
                    };
                    let mut fields = Vec::with_capacity(types.len());
                    let mut arguments = Vec::with_capacity(types.len());
                    for ty in types {
                        let binding = self.fresh()?;
                        fields.push(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        });
                        arguments.push(self.node(
                            ty.clone(),
                            K::Local(binding),
                            expression.source,
                        )?);
                    }
                    let body = self.node(
                        expression.ty.clone(),
                        K::EffectRequest {
                            operation: operation.clone(),
                            abi: *abi,
                            allowed_index: slot,
                            signature: Box::new(fallback_signature.unwrap()),
                            arguments,
                        },
                        expression.source,
                    )?;
                    let K::Match { arms, .. } = &mut replacement.kind else {
                        unreachable!()
                    };
                    arms.push(n::MatchArm {
                        pattern: n::Pattern::Tuple(fields),
                        guard: None,
                        body,
                    });
                }
                let mut connections: BTreeMap<_, _> =
                    nir::free_bindings::collect(&replacement, self.fuel)?
                        .into_iter()
                        .map(|id| (id, id))
                        .collect();
                // Locally declared loops receive fresh identities with their
                // body. Transfers to the installing caller retain its identity.
                let mut loops = BTreeSet::new();
                nir::visit::expression(&replacement, self.fuel, |expr, _| {
                    if let K::Loop { identity, .. } = expr.kind {
                        loops.insert(identity);
                    }
                    Ok(())
                })?;
                nir::visit::expression(&replacement, self.fuel, |expr, _| {
                    if let K::Break(id) | K::Continue(id) = expr.kind {
                        if !loops.contains(&id) {
                            connections.insert(id, id);
                        }
                    }
                    Ok(())
                })?;
                let source_begin = self.sources.len();
                cmtt::freshen(
                    &mut replacement,
                    connections,
                    self.next,
                    self.sources,
                    Span::new(
                        rustc_span::BytePos(expression.source.start),
                        rustc_span::BytePos(expression.source.end),
                    ),
                )
                .map_err(|e| e.to_string())?;
                // Copied inline calls carry empty-result-slot write evidence
                // keyed by source identity. Publish it for the fresh copy too.
                for index in source_begin..self.sources.len() {
                    *self.fuel = self
                        .fuel
                        .checked_sub(1)
                        .ok_or("inline cleanup metadata budget exhausted")?;
                    let origin = self.sources[index].original;
                    let key = (origin.start, origin.end);
                    if let Some(packet) = self
                        .known_empty
                        .get(&key)
                        .or_else(|| self.empty.get(&key))
                        .copied()
                    {
                        self.empty.insert((u32::MAX, index as u32), packet);
                    }
                }
                let K::Match { value: input, .. } = &mut replacement.kind else {
                    unreachable!()
                };
                // Evaluate the original request inputs outside the resume
                // region. Their own returns must keep their original target.
                let binding = self.fresh()?;
                *input =
                    Box::new(self.node(value.ty.clone(), K::Local(binding), expression.source)?);
                let mut resumes = false;
                nir::visit::expression(&replacement, self.fuel, |expr, _| {
                    resumes |= matches!(expr.kind, K::EffectResume(_));
                    Ok(())
                })?;
                if resumes {
                    let slot = self.fresh()?;
                    let exit = self.fresh()?;
                    let result = self.fresh()?;
                    let unreachable = self.fresh()?;
                    let packet = DeclarationId {
                        package: u64::MAX - 4,
                        local: slot.index,
                    };
                    if self.enums.iter().any(|item| item.declaration == packet) {
                        return Err("inline resume packet identity collision".into());
                    }
                    let (body, descriptor, empties) = nir::inline_return::lower_resume(
                        replacement,
                        expression.ty.clone(),
                        nir::inline_return::Boundary {
                            packet,
                            slot,
                            exit,
                            result,
                            unreachable,
                        },
                        self.sources,
                        expression.source,
                        self.fuel,
                    )?;
                    self.enums.push(descriptor);
                    self.packets.insert(packet);
                    for source in empties {
                        self.empty.insert((source.start, source.end), packet);
                    }
                    replacement = body;
                }
                let statement = self.node(
                    p(P::Void),
                    K::Let {
                        binding,
                        value: Box::new(value),
                    },
                    expression.source,
                )?;
                *expression = self.node(
                    expression.ty.clone(),
                    K::Block {
                        statements: vec![statement],
                        result: Some(Box::new(replacement)),
                    },
                    expression.source,
                )?;
                self.requests += 1;
                Ok(false)
            }
            K::Closure { captures, body, .. } => {
                // The body runs on invocation, not at this installation. Only
                // effect-free bodies can be retained without a routing plan.
                nir::visit::expression(body, self.fuel, |expr, _| {
                    if matches!(
                        expr.kind,
                        K::EffectHandle { .. }
                            | K::EffectRun { .. }
                            | K::EffectComputation { .. }
                            | K::EffectRequest { .. }
                            | K::EffectResume(_)
                    ) || matches!(expr.ty, Term::Computation { .. })
                    {
                        return Err("closure body requires its own effect environment".into());
                    }
                    Ok(())
                })?;
                for capture in captures {
                    if let Some(value) = &mut capture.value {
                        value.rewrite_with(self)?;
                    }
                }
                Ok(false)
            }
            K::EffectResume(value) if self.allow_resume => {
                value.rewrite_with(self)?;
                Ok(false)
            }
            K::EffectHandle { .. } | K::EffectResume(_) => Err("nested effect boundary".into()),
            K::Call { .. } if matches!(expression.ty, Term::Computation { .. }) => {
                Err("callee was not expanded".into())
            }
            _ => Ok(true),
        }
    }
}

impl Requests<'_> {
    fn operation(&self, index: usize) -> Option<&nir::effects::Operation> {
        if index < self.handlers.len() {
            Some(&self.handlers[index].operation)
        } else {
            self.outer.get(index - self.handlers.len())
        }
    }
    fn matches_row(&self, effects: &[nir::effects::Operation]) -> bool {
        effects.len() == self.row.len()
            && effects
                .iter()
                .zip(&self.row)
                .all(|(operation, index)| self.operation(*index) == Some(operation))
    }
    fn node(&mut self, ty: Term, kind: K, site: n::SourceRange) -> Result<n::Expr, String> {
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or("inline resume node budget exhausted")?;
        let index = u32::try_from(self.sources.len())
            .map_err(|_| "inline resume source space exhausted")?;
        self.sources.push(n::ExpansionSource {
            original: site,
            site,
        });
        Ok(n::Expr {
            ty,
            kind,
            source: n::SourceRange::generated(index),
        })
    }
    fn fresh(&mut self) -> Result<n::LocalId, String> {
        let index = *self.next;
        *self.next = self
            .next
            .checked_add(1)
            .ok_or("inline resume identity space exhausted")?;
        Ok(n::LocalId {
            owner: u32::MAX,
            index,
        })
    }
}

#[cfg(test)]
mod materialized_tests {
    use super::*;

    #[test]
    fn unrelated_body_does_not_copy_package_return_metadata() {
        let mut package = n::Package::default();
        for local in 0..1000 {
            package
                .internal_return_packets
                .insert(DeclarationId { package: 7, local });
        }
        let mut body = n::Expr {
            ty: Term::Primitive(nir::Primitive::Void),
            source: n::SourceRange { start: 0, end: 1 },
            kind: K::Unit,
        };
        let original = body.clone();
        let inline = BTreeSet::from([DeclarationId {
            package: 7,
            local: 1001,
        }]);
        let mut next = 17;
        let mut sources = Vec::new();
        let mut enums = Vec::new();
        let mut fuel = 2000;
        assert_eq!(
            expand_materialized(
                &package,
                &inline,
                &mut body,
                &mut next,
                &mut sources,
                &mut enums,
                &mut fuel
            )
            .unwrap(),
            (0, 0)
        );
        assert!(
            fuel > 1900,
            "unrelated metadata consumed the body budget: {fuel}"
        );
        assert_eq!(body, original);
        assert_eq!(next, 17);
        assert!(sources.is_empty() && enums.is_empty());
    }

    #[test]
    fn empty_inline_index_skips_the_optional_pass_without_work() {
        let mut body = n::Expr {
            ty: Term::Primitive(nir::Primitive::Void),
            source: n::SourceRange { start: 0, end: 1 },
            kind: K::Unit,
        };
        let mut fuel = 0;
        assert_eq!(
            expand_materialized(
                &n::Package::default(),
                &BTreeSet::new(),
                &mut body,
                &mut 0,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut fuel
            )
            .unwrap(),
            (0, 0)
        );
        assert_eq!(fuel, 0);
    }
}
