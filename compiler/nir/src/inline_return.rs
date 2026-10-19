//! Lower a logical inlined-function return boundary into existing owned-value
//! and loop control nodes. The caller supplies fresh identities and registers
//! the returned private enum before running ordinary verification.
use crate::program::{self as n, Expr, ExprKind as K, LocalId, SourceRange};
use crate::{DeclarationId, Primitive as P, Term};

/// Remove returns in result position only. No continuation is copied or moved
/// across a local scope; earlier statements and nested callables are untouched.
/// The caller owns rollback if the bounded traversal fails.
pub fn simplify_tail_returns(
    body: &mut Expr,
    result: &Term,
    fuel: &mut usize,
) -> Result<(), String> {
    fn simplify(
        expr: &mut Expr,
        result: &Term,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth >= 128 {
            return Err("inline tail return nesting budget exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("inline tail return budget exhausted")?;
        crate::type_computations::charge_term(result, fuel).map_err(|error| error.to_string())?;
        match &mut expr.kind {
            K::Return(value) => {
                let value = value.take().map(|value| *value).unwrap_or(Expr {
                    ty: Term::Primitive(P::Void),
                    source: expr.source,
                    kind: K::Unit,
                });
                *expr = value;
                simplify(expr, result, fuel, depth + 1)?;
            }
            K::Block {
                statements,
                result: tail,
            } => {
                if let Some(tail) = tail {
                    simplify(tail, result, fuel, depth + 1)?;
                    expr.ty = tail.ty.clone();
                } else if expr.ty == Term::Primitive(P::Never) {
                    if let Some(last) = statements.last_mut() {
                        // A discarded diverging last statement cannot normally
                        // complete. Its return result becomes this block's tail.
                        let candidate = match &mut last.kind {
                            K::Discard(value) => value.as_mut(),
                            _ => last,
                        };
                        simplify(candidate, result, fuel, depth + 1)?;
                        if candidate.ty != Term::Primitive(P::Never) {
                            let last = statements.pop().unwrap();
                            let value = match last.kind {
                                K::Discard(value) => *value,
                                _ => last,
                            };
                            expr.ty = value.ty.clone();
                            *tail = Some(Box::new(value));
                        }
                    }
                }
            }
            K::If {
                then_branch,
                else_branch,
                ..
            } => {
                simplify(then_branch, result, fuel, depth + 1)?;
                if let Some(branch) = else_branch.as_mut() {
                    simplify(branch, result, fuel, depth + 1)?;
                }
                if then_branch.ty != Term::Primitive(P::Never)
                    || else_branch
                        .as_ref()
                        .is_some_and(|branch| branch.ty != Term::Primitive(P::Never))
                {
                    expr.ty = result.clone();
                }
            }
            K::Match { arms, .. } => {
                for arm in arms.iter_mut() {
                    simplify(&mut arm.body, result, fuel, depth + 1)?;
                }
                if arms
                    .iter()
                    .any(|arm| arm.body.ty != Term::Primitive(P::Never))
                {
                    expr.ty = result.clone();
                }
            }
            _ => {}
        }
        Ok(())
    }
    simplify(body, result, fuel, 0)
}

/// Whether this expression exits its lexical function boundary. Closure bodies
/// have their own boundary, but capture initializers execute in this one.
pub fn has_lexical_return(body: &Expr, fuel: &mut usize) -> Result<bool, String> {
    fn scan(body: &Expr, fuel: &mut usize, depth: usize) -> Result<bool, String> {
        if depth >= 64 {
            return Err("inline return capture nesting budget exhausted".into());
        }
        let mut found = false;
        crate::visit::expression_pruned(body, fuel, |expr, fuel| {
            if found {
                return Ok(false);
            }
            match &expr.kind {
                K::Return(_) => {
                    found = true;
                    Ok(false)
                }
                K::Closure { captures, .. } => {
                    for capture in captures {
                        if let Some(value) = &capture.value {
                            if scan(value, fuel, depth + 1)? {
                                found = true;
                                break;
                            }
                        }
                    }
                    Ok(false)
                }
                _ => Ok(true),
            }
        })?;
        Ok(found)
    }
    scan(body, fuel, 0)
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    fn expr(kind: K) -> Expr {
        Expr {
            ty: Term::Primitive(P::Void),
            kind,
            source: SourceRange { start: 0, end: 0 },
        }
    }
    #[test]
    fn tail_simplification_preserves_non_tail_and_closure_boundaries() {
        let result = Term::Primitive(P::Void);
        let early = expr(K::Return(None));
        let closure = expr(K::Closure {
            parameters: vec![],
            captures: vec![],
            body: std::sync::Arc::new(early.clone()),
        });
        let mut body = expr(K::Block {
            statements: vec![early.clone(), closure.clone()],
            result: Some(Box::new(early.clone())),
        });
        simplify_tail_returns(&mut body, &result, &mut 100).unwrap();
        let K::Block {
            statements,
            result: tail,
        } = &body.kind
        else {
            panic!()
        };
        assert_eq!(statements, &vec![early, closure]);
        assert!(matches!(tail.as_ref().unwrap().kind, K::Unit));
        assert!(has_lexical_return(&body, &mut 100).unwrap());
        assert!(simplify_tail_returns(&mut body, &result, &mut 0).is_err());
    }
    #[test]
    fn closure_returns_are_separate_but_capture_returns_are_lexical() {
        let mut closure = expr(K::Closure {
            parameters: vec![],
            captures: vec![],
            body: std::sync::Arc::new(expr(K::Return(None))),
        });
        assert!(!has_lexical_return(&closure, &mut 100).unwrap());
        let K::Closure { captures, .. } = &mut closure.kind else {
            unreachable!()
        };
        captures.push(n::Capture {
            binding: LocalId { owner: 0, index: 0 },
            ty: Term::Primitive(P::Void),
            value: Some(Box::new(expr(K::Return(None)))),
        });
        assert!(has_lexical_return(&closure, &mut 100).unwrap());
        assert!(has_lexical_return(&closure, &mut 0).is_err());
        assert!(has_lexical_return(&closure, &mut 100).unwrap());
    }
}

pub struct Boundary {
    pub packet: DeclarationId,
    pub slot: LocalId,
    pub exit: LocalId,
    pub result: LocalId,
    pub unreachable: LocalId,
}
impl std::fmt::Display for Boundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(inline-return-boundary (packet {}) (slot {}) (exit {}) (result {}) (unreachable {}))",
            self.packet, self.slot, self.exit, self.result, self.unreachable
        )
    }
}

struct Lower<'a> {
    resume: bool,
    ids: Boundary,
    packet: Term,
    fuel: &'a mut usize,
    sources: &'a mut Vec<n::ExpansionSource>,
    site: SourceRange,
    empty_replacements: Vec<SourceRange>,
}
impl Lower<'_> {
    fn node(&mut self, ty: Term, kind: K, original: SourceRange) -> Result<Expr, String> {
        self.tick()?;
        let index = u32::try_from(self.sources.len())
            .map_err(|_| "inline return source space exhausted")?;
        self.sources.push(n::ExpansionSource {
            original,
            site: self.site,
        });
        Ok(Expr {
            ty,
            kind,
            source: SourceRange::generated(index),
        })
    }
    fn tick(&mut self) -> Result<(), String> {
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or("inline return budget exhausted")?;
        Ok(())
    }
    fn deliver(&mut self, value: Expr) -> Result<Expr, String> {
        let origin = value.source;
        let ready = self.node(
            self.packet.clone(),
            K::Variant {
                declaration: self.ids.packet,
                index: 1,
                fields: vec![value],
            },
            origin,
        )?;
        let assign = self.node(
            Term::Primitive(P::Void),
            K::Assign {
                place: n::Place {
                    root: self.ids.slot,
                    fields: vec![],
                },
                value: Box::new(ready),
            },
            origin,
        )?;
        self.empty_replacements.push(assign.source);
        let exit = self.node(Term::Primitive(P::Never), K::Break(self.ids.exit), origin)?;
        self.node(
            Term::Primitive(P::Never),
            K::Block {
                statements: vec![assign],
                result: Some(Box::new(exit)),
            },
            origin,
        )
    }
}
impl Lower<'_> {
    fn rewrite_returns(&mut self, body: &mut Expr, depth: usize) -> Result<(), String> {
        if depth >= 64 {
            return Err("inline return capture nesting budget exhausted".into());
        }
        let mut fuel = *self.fuel;
        let result = crate::visit::expression_mut_pruned(body, &mut fuel, |expr, remaining| {
            *self.fuel = *remaining;
            let result = (|| {
                match &mut expr.kind {
                    K::Closure { captures, .. } => {
                        // Captures execute in this boundary; the closure body
                        // owns a separate return target.
                        for capture in captures {
                            if let Some(value) = &mut capture.value {
                                self.rewrite_returns(value, depth + 1)?;
                            }
                        }
                        Ok(false)
                    }
                    K::EffectHandle { computation, .. } if self.resume => {
                        // A nested handler installs its own resume target.
                        // Its computation still runs in the current scope.
                        self.rewrite_returns(computation, depth + 1)?;
                        Ok(false)
                    }
                    K::EffectResume(value) if self.resume => {
                        let value = std::mem::replace(
                            value,
                            Box::new(Expr {
                                ty: Term::Primitive(P::Void),
                                source: expr.source,
                                kind: K::Unit,
                            }),
                        );
                        *expr = self.deliver(*value)?;
                        Ok(true)
                    }
                    K::Return(value) if !self.resume => {
                        let value = match value.take() {
                            Some(value) => *value,
                            None => self.node(Term::Primitive(P::Void), K::Unit, expr.source)?,
                        };
                        *expr = self.deliver(value)?;
                        // Visit the moved operand too: nested returns still
                        // target this boundary. No Return remains at this node.
                        Ok(true)
                    }
                    _ => Ok(true),
                }
            })();
            *remaining = *self.fuel;
            result
        });
        *self.fuel = fuel;
        result
    }
}

/// `body` is already type checked and hygienically renamed. No dummy value of
/// its result type is manufactured: the private slot starts in an empty case.
/// Failure discards the private input and rolls back added source provenance.
pub fn lower(
    body: Expr,
    result: Term,
    ids: Boundary,
    sources: &mut Vec<n::ExpansionSource>,
    site: SourceRange,
    fuel: &mut usize,
) -> Result<(Expr, n::Enum, Vec<SourceRange>), String> {
    let checkpoint = sources.len();
    let lowered = lower_inner(body, result, ids, sources, site, fuel, false);
    if lowered.is_err() {
        sources.truncate(checkpoint);
    }
    lowered
}

/// Deliver only this handler's resumes. Lexical function returns and loop
/// transfers pass through the region unchanged and retain ordinary cleanup.
pub fn lower_resume(
    body: Expr,
    result: Term,
    ids: Boundary,
    sources: &mut Vec<n::ExpansionSource>,
    site: SourceRange,
    fuel: &mut usize,
) -> Result<(Expr, n::Enum, Vec<SourceRange>), String> {
    let checkpoint = sources.len();
    let lowered = lower_inner(body, result, ids, sources, site, fuel, true);
    if lowered.is_err() {
        sources.truncate(checkpoint);
    }
    lowered
}

fn lower_inner(
    mut body: Expr,
    result: Term,
    ids: Boundary,
    sources: &mut Vec<n::ExpansionSource>,
    site: SourceRange,
    fuel: &mut usize,
    resume: bool,
) -> Result<(Expr, n::Enum, Vec<SourceRange>), String> {
    if std::collections::BTreeSet::from([ids.slot, ids.exit, ids.result, ids.unreachable]).len()
        != 4
    {
        return Err("inline return boundary requires distinct local identities".into());
    }
    crate::type_computations::charge_term(&result, fuel).map_err(|e| e.to_string())?;
    if !result.runtime_dependencies().is_empty()
        || !result
            .substitution_parameters()
            .map_err(|failure| failure.to_string())?
            .is_empty()
    {
        return Err("inline return result requires a closed declaration-scoped type".into());
    }
    let parameters = result
        .rigid_parameters()
        .map_err(|failure| failure.to_string())?;
    if parameters.values().any(|sort| *sort != crate::Sort::Type) {
        return Err("inline return runtime-dependent result storage is not yet implemented".into());
    }
    let type_parameters = parameters.keys().copied().collect::<Vec<_>>();
    let arguments = type_parameters
        .iter()
        .map(|declaration| Term::Rigid {
            declaration: *declaration,
            sort: crate::Sort::Type,
        })
        .collect();
    let packet = Term::Nominal {
        declaration: ids.packet,
        arguments,
    };
    let descriptor = n::Enum {
        parameter_types: vec![],
        declaration: ids.packet,
        name: "<inline-return-packet>".into(),
        type_parameters,
        capabilities: vec![],
        requirements: vec![],
        variants: vec![
            n::Variant {
                name: "pending".into(),
                shape: n::VariantShape::Tuple,
                fields: vec![],
            },
            n::Variant {
                name: "ready".into(),
                shape: n::VariantShape::Tuple,
                fields: vec![result.clone()],
            },
        ],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
    };
    let mut lower = Lower {
        resume,
        ids,
        packet,
        fuel,
        sources,
        site,
        empty_replacements: vec![],
    };
    lower.rewrite_returns(&mut body, 0)?;
    let pending = lower.node(
        lower.packet.clone(),
        K::Variant {
            declaration: lower.ids.packet,
            index: 0,
            fields: vec![],
        },
        site,
    )?;
    let init = lower.node(
        Term::Primitive(P::Void),
        K::Let {
            binding: lower.ids.slot,
            value: Box::new(pending),
        },
        site,
    )?;
    let delivered = lower.deliver(body)?;
    let region = lower.node(
        Term::Primitive(P::Void),
        K::Loop {
            identity: lower.ids.exit,
            body: Box::new(delivered),
        },
        site,
    )?;
    let slot = lower.node(lower.packet.clone(), K::Local(lower.ids.slot), site)?;
    let value = lower.node(result.clone(), K::Local(lower.ids.result), site)?;
    // Every finite region exit delivers ready. Keep the impossible empty case
    // divergent without inventing a value or adding an unchecked extraction.
    let unit = lower.node(Term::Primitive(P::Void), K::Unit, site)?;
    let unreachable = lower.node(
        Term::Primitive(P::Never),
        K::Loop {
            identity: lower.ids.unreachable,
            body: Box::new(unit),
        },
        site,
    )?;
    let selected = lower.node(
        result.clone(),
        K::Match {
            value: Box::new(slot),
            arms: vec![
                n::MatchArm {
                    pattern: n::Pattern::Variant {
                        declaration: lower.ids.packet,
                        index: 1,
                        fields: vec![n::Pattern::Binding {
                            binding: lower.ids.result,
                            inner: Box::new(n::Pattern::Wild),
                        }],
                    },
                    guard: None,
                    body: value,
                },
                n::MatchArm {
                    pattern: n::Pattern::Wild,
                    guard: None,
                    body: unreachable,
                },
            ],
        },
        site,
    )?;
    let root = lower.node(
        result,
        K::Block {
            statements: vec![init, region],
            result: Some(Box::new(selected)),
        },
        site,
    )?;
    Ok((root, descriptor, lower.empty_replacements))
}
