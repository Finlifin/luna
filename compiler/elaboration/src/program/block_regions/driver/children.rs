//! Route a source child to its retained frame or its atomic checking rule.
//! This dispatch is shared by all result consumers.
use super::*;
use region::Child;

pub(super) fn enter<'h, 'a: 'h, 'arena: 'h>(
    frames: &mut Vec<RegionFrame<'h>>,
    checker: &mut Checker<'a, 'arena>,
    request: Request<'h>,
) -> Result<Option<Child<'h>>, Error> {
    let (block, destination, boundary) = match request {
        Request::Collection(collection, expected) => {
            let expected =
                if checker.is_default_collection_shape(&expected, false, collection.span)? {
                    Some(expected)
                } else {
                    None
                };
            return Ok(Some(Child {
                expr: arguments::RoutedSource::Collection(collection),
                destination: Destination::from_expected(expected),
            }));
        }
        Request::Block(block, destination) => (block, destination, None),
        Request::Expression(input, destination) => {
            let expr = input.as_ref();
            if matches!(expr.kind, H::Tuple(_) | H::List(_))
                && let Some(depth) = destination.contextual_depth()
            {
                frames.push(RegionFrame::Aggregate(aggregates::Frame::contextual(
                    expr, depth,
                )?));
                return Ok(None);
            }
            if let H::Tuple(items) = expr.kind
                && matches!(
                    destination.structural_input(),
                    Some(
                        crate::calls::InputContext::Tuple(_)
                            | crate::calls::InputContext::NormalTuple { .. }
                    )
                )
            {
                frames.push(RegionFrame::Aggregate(aggregates::Frame::partial_tuple(
                    expr,
                    items,
                    destination,
                )?));
                return Ok(None);
            }
            if let H::NFApplication(head, inputs) = expr.kind
                && let H::ResolvedDef { def_id, .. } = head.kind
                && checker.data_consumer_call(expr)?.is_none()
                && let Some(view) = destination.ordinary_view()
                && view.input.is_none()
                && view
                    .expected
                    .is_none_or(|ty| matches!(ty, Term::Primitive(_)) || ty.is_comptime_callable())
            {
                // Captured declaration inputs still go through their
                // existing lexical/default supply before NF preparation.
                let source_owner = checker.package.definition_owner(def_id);
                let captured = if let Some(owner) = source_owner {
                    checker
                        .nominal_captures(owner, expr.span)?
                        .iter()
                        .any(|id| checker.alias_value_bindings.contains_key(id))
                } else {
                    false
                };
                if !captured
                    && let Some(owner) = source_owner
                    && let Some(application) =
                        checker.static_normal_application(owner, head.span)?
                    && checker.prepare_region_callable_delivery(
                        expr,
                        view.expected,
                        view.partial,
                    )?
                {
                    frames.push(RegionFrame::NormalApplication(
                        normal_applications::Frame::begin_normal_application(
                            checker,
                            application,
                            inputs,
                            expr.span,
                            destination,
                            true,
                        )?,
                    ));
                    return Ok(None);
                }
                if let Some(owner) = source_owner
                    && !captured
                    && !checker.runtime_value_context()
                    && !checker.in_code_body
                    && matches!(checker.package.item(owner).map(|item| &item.kind),
                                    Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig))
                                    if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
                    && checker.prepare_region_callable_delivery(
                        expr,
                        view.expected,
                        view.partial,
                    )?
                {
                    checker.ensure_signature(owner, expr.span)?;
                    let prefix = checker.lexical_function_arguments(owner, expr.span)?;
                    let preserve_effect = checker.preserve_effect_value;
                    if let Some(preparation) = checker.begin_comptime_function_instance(
                        expr,
                        owner,
                        inputs,
                        &prefix,
                        view.expected,
                        false,
                        Some(preserve_effect),
                    )? {
                        checker.preserve_effect_value = false;
                        frames.push(RegionFrame::CallableApplication(preparation));
                        return Ok(None);
                    }
                }
            }
            // Nonprimitive targets first pass through ordinary
            // delivery adaptation, which enters the same frame.
            if (comparisons::is_comparison(expr)
                || logical::is_logical(expr)
                || arithmetic::operator(expr).is_some()
                || unary::operator(expr).is_some())
                && let Some(view) = destination.ordinary_view()
                && view.input.is_none()
                && view
                    .expected
                    .is_none_or(|ty| matches!(ty, Term::Primitive(_)))
            {
                let frame = if unary::operator(expr).is_some() {
                    RegionFrame::Unary(unary::Frame::new(checker, expr, destination, true)?)
                } else if logical::is_logical(expr) {
                    RegionFrame::Logical(logical::Frame::new(checker, expr, destination, true)?)
                } else if arithmetic::operator(expr).is_some() {
                    RegionFrame::Arithmetic(arithmetic::Frame::new(
                        checker,
                        expr,
                        destination,
                        true,
                    )?)
                } else {
                    RegionFrame::Comparison(comparisons::Frame::new(
                        checker,
                        expr,
                        destination,
                        true,
                    )?)
                };
                frames.push(frame);
                return Ok(None);
            }
            // Retain the input rule and its delivery boundary;
            // probes use the same child driver as final checks.
            if destination.structural_input().is_some() && matches!(expr.kind, H::If(_, _, Some(_)))
            {
                frames.push(RegionFrame::Conditional(conditionals::Frame::partial(
                    checker,
                    expr,
                    destination,
                    true,
                )?));
                return Ok(None);
            }
            if destination.structural_input().is_some() {
                let frame = match expr.kind {
                    H::When(_) => Some(RegionFrame::When(when::Frame::ordinary(
                        checker,
                        expr,
                        destination.clone(),
                        true,
                    )?)),
                    H::Match(..) => Some(RegionFrame::Match(matches::Frame::ordinary(
                        checker,
                        expr,
                        destination.clone(),
                        true,
                    )?)),
                    _ => None,
                };
                if let Some(frame) = frame {
                    frames.push(frame);
                    return Ok(None);
                }
            }
            if let Some(depth) = destination.contextual_depth()
                && matches!(expr.kind, H::Match(..))
            {
                frames.push(RegionFrame::Match(matches::Frame::new(
                    checker, expr, depth,
                )?));
                return Ok(None);
            }
            if let Some(depth) = destination.contextual_depth()
                && let H::When(arms) = expr.kind
                && arms.iter().any(|arm| arm.cond.is_none())
            {
                frames.push(RegionFrame::When(when::Frame::new(checker, expr, depth)?));
                return Ok(None);
            }
            // Ordinary branches enter after delivery prepares the
            // complete target, as in expr_context.
            if let Some(depth) = destination.contextual_depth()
                && matches!(expr.kind, H::If(_, _, Some(_)))
            {
                frames.push(RegionFrame::Conditional(conditionals::Frame::new(
                    checker, expr, depth,
                )?));
                return Ok(None);
            }
            if matches!(expr.kind, H::Match(..))
                && let Some(view) = destination.ordinary_view()
                && (view.input.is_none() || view.expected.is_some())
                && checker.prepare_region_branch_delivery(expr, view.expected, view.partial)?
            {
                frames.push(RegionFrame::Match(matches::Frame::ordinary(
                    checker,
                    expr,
                    destination,
                    true,
                )?));
                return Ok(None);
            }
            if matches!(expr.kind, H::When(_))
                && let Some(view) = destination.ordinary_view()
                && (view.input.is_none() || view.expected.is_some())
                && checker.prepare_region_branch_delivery(expr, view.expected, view.partial)?
            {
                frames.push(RegionFrame::When(when::Frame::ordinary(
                    checker,
                    expr,
                    destination,
                    true,
                )?));
                return Ok(None);
            }
            if matches!(expr.kind, H::If(..))
                && let Some(view) = destination.ordinary_view()
                && (view.input.is_none() || view.expected.is_some())
                && checker.prepare_region_branch_delivery(expr, view.expected, view.partial)?
            {
                frames.push(RegionFrame::Conditional(conditionals::Frame::ordinary(
                    checker,
                    expr,
                    destination,
                    true,
                )?));
                return Ok(None);
            }
            if let Some(nested) = destination.nested(expr)? {
                let H::Block(block) = expr.kind else {
                    unreachable!("nested block destination")
                };
                let boundary = matches!(nested, Destination::Ordinary(_)).then(|| {
                    ExpressionBoundary::enter(
                        checker,
                        expr,
                        block,
                        nested.structural_input().is_some(),
                    )
                });
                (block, nested, boundary)
            } else {
                return Ok(Some(Child {
                    expr: arguments::RoutedSource::Expression(input),
                    destination,
                }));
            }
        }
    };
    let scope = match checker
        .charge_block_frame(block.span)
        .and_then(|()| LexicalScope::enter(checker, block.span))
    {
        Ok(scope) => scope,
        Err(error) => {
            if let Some(boundary) = &boundary {
                boundary.leave(checker);
            }
            return Err(error);
        }
    };
    frames.push(RegionFrame::Block(BlockFrame {
        progress: Progress::new(block, destination),
        scope,
        boundary,
    }));
    Ok(None)
}
