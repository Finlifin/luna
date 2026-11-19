//! Inspect the normal returns of checked Type producers without invoking them.
use super::*;

enum Work<'a> {
    Enter(DeclarationId),
    Result(&'a n::Expr),
    Finish(DeclarationId),
}

pub(super) fn guarantees(
    world: &RequiredWorld,
    entry: DeclarationId,
    head: DeclarationId,
    fuel: &mut usize,
) -> Result<bool, String> {
    let package = world.package();
    let callables = match world
        .phase
        .as_ref()
        .and_then(|phase| phase.inspection_for(package))
        .and_then(|source| source.callables_for(package))
    {
        Some(callables) => callables,
        None => {
            nir::callable_source::Callables::new(package, fuel).map_err(|reason| reason.message)?
        }
    };
    let mut work = vec![Work::Enter(entry)];
    let mut active = BTreeSet::new();
    let mut checked = BTreeSet::new();
    let mut saw_head = false;
    while let Some(task) = work.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("result head inspection budget exhausted")?;
        match task {
            Work::Enter(id) => {
                if checked.contains(&id) {
                    continue;
                }
                if !active.insert(id) {
                    return Ok(false);
                }
                let Some(producer) = callables
                    .definition(&id)
                    .filter(|f| f.phase == n::FunctionPhase::Comptime)
                else {
                    return Ok(false);
                };
                let body = &producer.body;
                work.push(Work::Finish(id));
                work.push(Work::Result(body));
                // Capture initializers can return from this function. The
                // body of a newly constructed closure has a separate exit.
                nir::visit::expression_in_closure_environment(body, fuel, |expr, _| {
                    if let K::Return(value) = &expr.kind {
                        work.push(Work::Result(value.as_deref().unwrap_or(expr)));
                    }
                    Ok(())
                })?;
            }
            Work::Finish(id) => {
                active.remove(&id);
                checked.insert(id);
            }
            Work::Result(value) => match &value.kind {
                K::Return(None) => return Ok(false),
                // Its lexical return operand was scheduled by Enter. A
                // diverging path has no additional fallthrough value.
                K::Return(Some(_)) => (),
                _ if is_never(&value.ty) => (),
                K::Block {
                    result: Some(value),
                    ..
                }
                | K::CallPreparation {
                    result: Some(value),
                    ..
                }
                | K::UnsafeScope(value) => work.push(Work::Result(value)),
                K::If {
                    then_branch,
                    else_branch: Some(no),
                    ..
                } => {
                    work.push(Work::Result(no));
                    work.push(Work::Result(then_branch));
                }
                K::Match { arms, .. } => {
                    work.extend(arms.iter().rev().map(|arm| Work::Result(&arm.body)));
                }
                K::NormalFormType { head: found, .. }
                | K::TypeValue(Term::NormalForm { head: found, .. })
                    if *found == head =>
                {
                    saw_head = true
                }
                K::Literal(n::Literal::Description(description))
                    if package
                        .refinements
                        .structure(description)?
                        .is_some_and(|app| app.head() == head) =>
                {
                    saw_head = true
                }
                K::Call { callee, .. } => match &callee.kind {
                    K::Function(id)
                    | K::FunctionInstance {
                        declaration: id, ..
                    } => work.push(Work::Enter(*id)),
                    _ => return Ok(false),
                },
                _ => return Ok(false),
            },
        }
    }
    Ok(saw_head)
}
