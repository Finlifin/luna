//! Rebuild result expressions against the settled domain. Ordinary expression
//! checking owns field contexts, constructor routing and result conversions.
use super::*;

// A bare Symbol is a contextual constructor head here. A projection on a
// Symbol may also call an ordinary method; it cannot be delayed on syntax alone.
pub(super) fn fields<'h>(site: &Expr<'h>) -> Option<impl Iterator<Item = &'h Expr<'h>>> {
    let (items, arguments): (&[Expr<'h>], &[Arg<'h>]) = match site.kind {
        H::Tuple(items) | H::List(items) => (items, &[]),
        H::Application(_, args) | H::ExtendedApplication(_, args)
            if enum_groups::has_bare_symbol_head(site) =>
        {
            (&[], args)
        }
        _ => return None,
    };
    Some(items.iter().chain(arguments.iter().map(|arg| match arg {
        Arg::Positional(e)
        | Arg::Named(_, e)
        | Arg::Expand(e)
        | Arg::Implicit(e)
        | Arg::PostLambda(e) => *e,
    })))
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    All,
    Selected,
}

impl Position<'_> {
    fn has_choice(&self) -> bool {
        self.sites.len() != 1 || self.sites.iter().any(|site| {
            matches!(site, Alternative::Aggregate { fields, .. } if fields.iter().any(Position::has_choice))
        })
    }
}

pub(super) fn check(
    checker: &mut Checker<'_, '_>,
    site: &Expr<'_>,
    fields: &[Position<'_>],
    domain: &Term,
    active: bool,
) -> Result<n::Expr, Error> {
    let arena = hir::HirArena::new();
    let mut lets = Vec::new();
    if !active || fields.iter().any(Position::has_choice) {
        let witness = aggregate(checker, &arena, site, fields, View::All, &mut lets)?;
        let checked = checker.expr(&witness, Some(domain))?;
        if !active {
            return Ok(checked);
        }
    }
    let selected = aggregate(checker, &arena, site, fields, View::Selected, &mut lets)?;
    let value = checker.expr(&selected, Some(domain))?;
    if lets.is_empty() {
        return Ok(value);
    }
    Ok(n::Expr {
        source: source(site.span),
        ty: declaration_inputs::copy_input_domain(
            &value.ty,
            &mut checker.required_steps,
            site.span,
        )?,
        kind: K::Block {
            statements: lets,
            result: Some(Box::new(value)),
        },
    })
}

fn aggregate<'a, 'h: 'a>(
    checker: &mut Checker<'_, '_>,
    arena: &'a hir::HirArena,
    site: &Expr<'h>,
    fields: &[Position<'h>],
    view: View,
    lets: &mut Vec<n::Expr>,
) -> Result<Expr<'a>, Error> {
    let mut items = Vec::with_capacity(fields.len());
    for field in fields {
        let item = if view == View::All && field.sites.len() > 1 {
            // Preserve all return sites for contextual checking in one tree.
            // This witness is discarded before invocation; no guard is replayed.
            let mut arms = Vec::with_capacity(field.sites.len());
            for (index, alt) in field.sites.iter().enumerate() {
                let body = arena.alloc_expr(render(checker, arena, alt, view, lets)?);
                let cond = (index + 1 < field.sites.len()).then(|| {
                    arena.alloc_expr(Expr {
                        hir_id: body.hir_id,
                        span: body.span,
                        kind: H::Lit(hir::Lit {
                            span: body.span,
                            kind: hir::LitKind::Bool(false),
                        }),
                    })
                });
                arms.push(hir::CondictionArm {
                    hir_id: body.hir_id,
                    span: body.span,
                    cond,
                    body,
                });
            }
            Expr {
                hir_id: site.hir_id,
                span: site.span,
                kind: H::When(arena.alloc_cond_arm_slice(arms)),
            }
        } else {
            let index = match view {
                View::All => 0,
                View::Selected => field
                    .selected
                    .ok_or_else(|| error(site.span, "missing selected aggregate field"))?,
            };
            let alt = field
                .sites
                .get(index)
                .ok_or_else(|| error(site.span, "missing aggregate result site"))?;
            render(checker, arena, alt, view, lets)?
        };
        items.push(item);
    }
    let kind = match site.kind {
        H::Tuple(_) => H::Tuple(arena.alloc_expr_slice(items)),
        H::List(_) => H::List(arena.alloc_expr_slice(items)),
        H::Application(head, args) | H::ExtendedApplication(head, args) => {
            if args.len() != items.len() {
                return Err(error(site.span, "staged constructor field count mismatch"));
            }
            let args = arena.alloc_arg_slice(
                args.iter()
                    .zip(items)
                    .map(|(arg, item)| {
                        let item = arena.alloc_expr(item);
                        match arg {
                            Arg::Positional(_) => Arg::Positional(item),
                            Arg::Named(name, _) => Arg::Named(*name, item),
                            Arg::Expand(_) => Arg::Expand(item),
                            Arg::Implicit(_) => Arg::Implicit(item),
                            Arg::PostLambda(_) => Arg::PostLambda(item),
                        }
                    })
                    .collect::<Vec<_>>(),
            );
            if matches!(site.kind, H::ExtendedApplication(..)) {
                H::ExtendedApplication(head, args)
            } else {
                H::Application(head, args)
            }
        }
        _ => return Err(error(site.span, "invalid aggregate clause result")),
    };
    Ok(Expr {
        span: site.span,
        hir_id: site.hir_id,
        kind,
    })
}

fn render<'a, 'h: 'a>(
    checker: &mut Checker<'_, '_>,
    arena: &'a hir::HirArena,
    alt: &Alternative<'h>,
    view: View,
    lets: &mut Vec<n::Expr>,
) -> Result<Expr<'a>, Error> {
    let (site, ty, value) = match alt {
        Alternative::Literal(expr) => return Ok((**expr).clone()),
        Alternative::Aggregate { site, fields } => {
            return aggregate(checker, arena, site, fields, view, lets);
        }
        Alternative::Producer { site, domain } if view == View::All => (
            *site,
            domain
                .as_ref()
                .ok_or_else(|| error(site.span, "missing aggregate producer domain"))?,
            None,
        ),
        Alternative::Producer { site, .. } => {
            return Err(error(site.span, "missing selected aggregate producer"));
        }
        Alternative::Frozen { site, value } => (*site, &value.ty, Some(value)),
    };
    // The enclosing required root scopes these fresh locals. Only the selected
    // tree enters the VM, with a real binding for every producer it reads.
    let binding = checker.spread_binding(site.span)?;
    let ty = declaration_inputs::copy_input_domain(ty, &mut checker.required_steps, site.span)?;
    checker.locals.insert(binding, ty);
    checker.immutable.insert(binding);
    if let (View::Selected, Some(value)) = (view, value) {
        constants::charge_copy(value, &mut checker.required_steps, site.span)?;
        lets.push(n::Expr {
            source: source(site.span),
            ty: p(P::Void),
            kind: K::Let {
                binding,
                value: value.clone(),
            },
        });
    }
    Ok(Expr {
        hir_id: site.hir_id,
        span: site.span,
        kind: H::ResolvedLocal(
            hir::common::Ident::new("$clause-field", site.span).name,
            hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index)),
        ),
    })
}
